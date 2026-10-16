//! The timeline's sound. [`Mix`] renders any stretch of the timeline from the
//! clips' [`VoiceStream`]s; the preview's mixer thread plays it into the ring
//! the audio callback reads, and the export renders it from start to end, so
//! both hear exactly the same samples.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

use vf_core::engine::scene::{asset_has_audio, content_end, fade_gain};
use vf_core::model::{ItemKind, Project, TrackKind};
use vf_media::audio::{CHANNELS, SAMPLE_RATE};

use crate::loudness::{self, Master, Meter};
use crate::voice::{voice_chain, VoiceStream};
use crate::{Command, Shared};

/// Frames mixed per step of the preview (about 10 ms).
const CHUNK: usize = 480;
/// How far ahead clips get their stream opened.
const PREOPEN: f64 = 1.5;
/// Quiet time after a change before the preview measures the mix's loudness.
const MEASURE_DELAY: Duration = Duration::from_millis(800);

/// A clip that makes sound, flattened from the project.
#[derive(Clone)]
pub(crate) struct Clip {
    pub(crate) id: String,
    pub(crate) path: PathBuf,
    /// First and one-past-last timeline frame.
    pub(crate) first: i64,
    pub(crate) end: i64,
    pub(crate) source_in: f64,
    pub(crate) speed: f64,
    pub(crate) chain: String,
    pub(crate) item: vf_core::model::TimelineItem,
}

pub(crate) fn audible_clips(project: &Project) -> Vec<Clip> {
    let frame = |seconds: f64| (seconds * SAMPLE_RATE as f64).round() as i64;
    let mut clips = Vec::new();
    for track in &project.tracks {
        if track.muted || (track.hidden && track.kind != TrackKind::Audio) {
            continue;
        }
        for item in &track.items {
            if item.hidden || item.muted || item.volume() <= 0.0 || !item.kind.has_sound() {
                continue;
            }
            let asset = project.asset(item.asset_id.as_deref());
            if item.kind == ItemKind::Video && !asset_has_audio(asset) {
                continue;
            }
            let Some(asset) = asset else { continue };
            let first = frame(item.start.max(0.0));
            let end = frame(item.start + item.duration);
            if end <= first {
                continue;
            }
            clips.push(Clip {
                id: item.id.clone(),
                path: PathBuf::from(&asset.path),
                first,
                end,
                source_in: item.source_in(),
                speed: item.speed(),
                chain: voice_chain(item.speed(), item.voice_enhance.unwrap_or(0.0)),
                item: item.clone(),
            });
        }
    }
    clips
}

/// Everything about the clips that changes how the mix sounds.
fn fingerprint(clips: &[Clip], total: f64, target: Option<f64>) -> String {
    let mut text = format!("{total:.4}|{target:?}");
    for clip in clips {
        let item = &clip.item;
        text += &format!(
            "|{}:{}:{}:{}:{:.6}:{}:{:.4}:{:.4}:{:.4}",
            clip.id,
            clip.path.display(),
            clip.first,
            clip.end,
            clip.source_in,
            clip.chain,
            item.volume(),
            item.fade_in(),
            item.fade_out()
        );
    }
    text
}

struct Voice {
    stream: VoiceStream,
    path: PathBuf,
    source_in: f64,
    first: i64,
    /// Clip frame the stream reads next (negative: needs a restart).
    position: i64,
}

#[derive(Default)]
pub(crate) struct Mix {
    clips: Vec<Clip>,
    voices: HashMap<String, Voice>,
    failed: HashMap<String, PathBuf>,
    scratch: Vec<f32>,
}

impl Mix {
    pub(crate) fn new(clips: Vec<Clip>) -> Self {
        Mix { clips, ..Default::default() }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    pub(crate) fn set_clips(&mut self, clips: Vec<Clip>) {
        self.clips = clips;
        let clips = &self.clips;
        self.voices.retain(|id, voice| clips.iter().any(|c| &c.id == id && c.path == voice.path && c.chain == voice.stream.chain()));
        self.failed.retain(|id, path| clips.iter().any(|c| &c.id == id && &c.path == path));
    }

    /// The next render starts every clip afresh (after a seek).
    pub(crate) fn invalidate(&mut self) {
        for voice in self.voices.values_mut() {
            voice.position = -1;
        }
    }

    fn open(&mut self, index: usize, local: i64) -> bool {
        let clip = &self.clips[index];
        if self.failed.contains_key(&clip.id) {
            return false;
        }
        let source = clip.source_in + local as f64 / SAMPLE_RATE as f64 * clip.speed;
        let ready = match self.voices.get_mut(&clip.id) {
            Some(voice) if voice.position == local && voice.source_in == clip.source_in && voice.first == clip.first => return true,
            Some(voice) => voice.stream.restart(source).map(|_| ()),
            None => VoiceStream::open(&clip.path, source, clip.speed, clip.item.voice_enhance.unwrap_or(0.0)).map(|stream| {
                self.voices.insert(clip.id.clone(), Voice { stream, path: clip.path.clone(), source_in: 0.0, first: 0, position: 0 });
            }),
        };
        match ready {
            Ok(()) => {
                let voice = self.voices.get_mut(&clip.id).expect("voice");
                voice.source_in = clip.source_in;
                voice.first = clip.first;
                voice.position = local;
                true
            }
            Err(error) => {
                log::warn!("no sound from {}: {error}", clip.path.display());
                self.voices.remove(&clip.id);
                self.failed.insert(clip.id.clone(), clip.path.clone());
                false
            }
        }
    }

    /// Opens the streams of clips that start within `ahead` frames, so
    /// starting them later costs nothing.
    pub(crate) fn preopen(&mut self, frame: i64, ahead: i64) {
        for index in 0..self.clips.len() {
            let clip = &self.clips[index];
            if clip.first > frame && clip.first - frame < ahead && !self.voices.contains_key(&clip.id) {
                self.open(index, 0);
            }
        }
    }

    /// Mixes the timeline from `frame` into `out` (interleaved stereo),
    /// before the master stage.
    pub(crate) fn render(&mut self, frame: i64, out: &mut [f32]) {
        out.fill(0.0);
        let end = frame + (out.len() / CHANNELS) as i64;
        for index in 0..self.clips.len() {
            let (first, last) = {
                let clip = &self.clips[index];
                (frame.max(clip.first), end.min(clip.end))
            };
            if last <= first {
                continue;
            }
            let local = first - self.clips[index].first;
            if !self.open(index, local) {
                continue;
            }
            let count = (last - first) as usize;
            let clip = &self.clips[index];
            let voice = self.voices.get_mut(&clip.id).expect("voice");
            self.scratch.resize(count * CHANNELS, 0.0);
            if let Err(error) = voice.stream.read(&mut self.scratch) {
                log::warn!("sound of {} stopped: {error}", clip.path.display());
                voice.position = -1;
                continue;
            }
            voice.position = local + count as i64;
            let volume = clip.item.volume();
            let offset = (first - frame) as usize * CHANNELS;
            for (i, pair) in self.scratch.chunks_exact(CHANNELS).enumerate() {
                let time = (first + i as i64) as f64 / SAMPLE_RATE as f64;
                let gain = (volume * fade_gain(&clip.item, time)) as f32;
                out[offset + i * CHANNELS] += pair[0] * gain;
                out[offset + i * CHANNELS + 1] += pair[1] * gain;
            }
        }
    }
}

/// Renders `frames` frames from the start in blocks, handing each to `sink`.
/// Stops early (returning false) when `cancelled` says so.
pub(crate) fn render_all(mix: &mut Mix, frames: usize, mut sink: impl FnMut(usize, &[f32]), cancelled: &dyn Fn() -> bool) -> bool {
    const BLOCK: usize = 4800;
    let mut block = vec![0.0f32; BLOCK * CHANNELS];
    let mut done = 0;
    while done < frames {
        if cancelled() {
            return false;
        }
        let count = BLOCK.min(frames - done);
        mix.render(done as i64, &mut block[..count * CHANNELS]);
        sink(done, &block[..count * CHANNELS]);
        done += count;
    }
    true
}

/// Loudness of the whole timeline as the export renders it (before the master stage).
fn measure(project: &Project, cancelled: &dyn Fn() -> bool) -> Option<Option<f64>> {
    let total = content_end(project);
    let mut mix = Mix::new(audible_clips(project));
    let mut meter = Meter::default();
    let frames = (total * SAMPLE_RATE as f64).round() as usize;
    render_all(&mut mix, frames, |_, block| meter.push(block), cancelled).then(|| meter.integrated())
}

pub(crate) fn run(commands: Receiver<Command>, sender: Sender<Command>, mut producer: rtrb::Producer<f32>, shared: Arc<Shared>) {
    let mut mix = Mix::default();
    let mut master = Master::new(false, 1.0);
    let mut playing = false;
    let mut frame = 0i64;
    let mut volume = 1.0f32;
    let mut out = vec![0.0f32; CHUNK * CHANNELS];
    // Loudness measurements: the latest request wins.
    let generation = Arc::new(AtomicU64::new(0));
    let mut measured = String::new();
    loop {
        let wait = if playing { Duration::from_millis(3) } else { Duration::from_millis(50) };
        match commands.recv_timeout(wait) {
            Ok(Command::Project(project)) => {
                let clips = audible_clips(&project);
                let target = loudness::target(project.export.loudness);
                let print = fingerprint(&clips, content_end(&project), target);
                mix.set_clips(clips);
                if target.is_none() {
                    generation.fetch_add(1, Ordering::AcqRel);
                    master = Master::new(false, 1.0);
                    measured.clear();
                } else if print != measured {
                    if !master.is_normalised() {
                        master = Master::new(true, 1.0);
                    }
                    measured = print;
                    let id = generation.fetch_add(1, Ordering::AcqRel) + 1;
                    let (generation, sender) = (generation.clone(), sender.clone());
                    std::thread::Builder::new()
                        .name("audio-loudness".into())
                        .spawn(move || {
                            std::thread::sleep(MEASURE_DELAY);
                            let stale = || generation.load(Ordering::Acquire) != id;
                            if let Some(loudness) = measure(&project, &stale) {
                                if !stale() {
                                    let _ = sender.send(Command::Gain(id, loudness::normalising_gain(target, loudness)));
                                }
                            }
                        })
                        .ok();
                }
            }
            Ok(Command::Gain(id, gain)) => {
                if id == generation.load(Ordering::Acquire) {
                    master = Master::new(true, gain);
                }
            }
            Ok(Command::Play(at)) => {
                shared.playing.store(false, Ordering::Release);
                shared.played.store(0, Ordering::Release);
                shared.flush.fetch_add(1, Ordering::AcqRel);
                frame = (at.max(0.0) * SAMPLE_RATE as f64).round() as i64;
                mix.invalidate();
                master.reset();
                // Prime about 60 ms so playback starts without a gap.
                for _ in 0..6 {
                    step(&mut mix, &mut master, &mut frame, volume, &mut out, &mut producer);
                }
                playing = true;
                shared.playing.store(true, Ordering::Release);
            }
            Ok(Command::Stop) => {
                playing = false;
                shared.playing.store(false, Ordering::Release);
                shared.flush.fetch_add(1, Ordering::AcqRel);
            }
            Ok(Command::Master(value)) => volume = value,
            Ok(Command::Output(ring)) => producer = ring,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                generation.fetch_add(1, Ordering::AcqRel);
                return;
            }
        }
        if !playing {
            continue;
        }
        // Keep about 100 ms queued.
        while producer.slots() >= CHUNK * CHANNELS && producer.buffer().capacity() - producer.slots() < SAMPLE_RATE as usize / 10 * CHANNELS {
            step(&mut mix, &mut master, &mut frame, volume, &mut out, &mut producer);
        }
        // Open streams of clips that start soon, outside the time-critical loop.
        mix.preopen(frame, (PREOPEN * SAMPLE_RATE as f64) as i64);
    }
}

/// Mixes, masters and queues the next chunk of the preview.
fn step(mix: &mut Mix, master: &mut Master, frame: &mut i64, volume: f32, out: &mut [f32], producer: &mut rtrb::Producer<f32>) {
    mix.render(*frame, out);
    master.process(out);
    if volume != 1.0 {
        out.iter_mut().for_each(|s| *s *= volume);
    }
    push(producer, out);
    *frame += (out.len() / CHANNELS) as i64;
}

fn push(producer: &mut rtrb::Producer<f32>, samples: &[f32]) {
    if let Ok(mut chunk) = producer.write_chunk_uninit(samples.len().min(producer.slots())) {
        let (first, second) = chunk.as_mut_slices();
        let split = first.len();
        for (slot, value) in first.iter_mut().zip(samples) {
            slot.write(*value);
        }
        for (slot, value) in second.iter_mut().zip(&samples[split..]) {
            slot.write(*value);
        }
        let len = first.len() + second.len();
        unsafe { chunk.commit(len) };
    }
}

/// What the preview plays from `start` for `frames` frames, rendered offline
/// with the preview's chunking, master stage and `gain`.
#[cfg(test)]
pub(crate) fn preview(project: &Project, start: f64, frames: usize, gain: f32) -> Vec<f32> {
    let mut mix = Mix::new(audible_clips(project));
    let mut master = Master::new(loudness::target(project.export.loudness).is_some(), gain);
    let mut frame = (start * SAMPLE_RATE as f64).round() as i64;
    let mut chunk = vec![0.0f32; CHUNK * CHANNELS];
    let mut out = Vec::with_capacity(frames * CHANNELS + chunk.len());
    while out.len() < frames * CHANNELS {
        mix.preopen(frame, (PREOPEN * SAMPLE_RATE as f64) as i64);
        mix.render(frame, &mut chunk);
        master.process(&mut chunk);
        out.extend_from_slice(&chunk);
        frame += CHUNK as i64;
    }
    out.truncate(frames * CHANNELS);
    out
}

#[cfg(test)]
pub(crate) fn measured_gain(project: &Project) -> f32 {
    let target = loudness::target(project.export.loudness);
    loudness::normalising_gain(target, measure(project, &|| false).flatten())
}
