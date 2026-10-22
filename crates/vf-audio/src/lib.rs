//! Preview playback of the timeline's sound. A mixer thread decodes and mixes
//! ahead into a lock-free ring; the audio callback only hands samples to the
//! device and counts them, which gives the playback clock the video follows.
//! When the output device changes (headphones plugged in or pulled out, a new
//! default device, another sample rate), the stream is rebuilt for it.

pub mod beats;
pub mod loudness;
pub mod mixdown;
mod mixer;
pub mod voice;

#[cfg(test)]
mod parity_tests;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use vf_core::model::Project;
use vf_media::audio::{CHANNELS, SAMPLE_RATE};

pub(crate) enum Command {
    Project(Arc<Project>),
    Play(f64),
    Stop,
    Master(f32),
    /// A new ring for a rebuilt output stream.
    Output(rtrb::Producer<f32>),
    /// Loudness gain measured for request `.0`.
    Gain(u64, f32),
}

/// State shared with the audio callback.
pub(crate) struct Shared {
    /// 48 kHz frames handed to the device since the last flush.
    played: AtomicU64,
    /// Bumped to make the callback drop everything queued.
    flush: AtomicU64,
    playing: AtomicBool,
    /// The device went away or changed: the stream needs rebuilding.
    broken: AtomicBool,
}

/// How often to look for an output device while there is none.
const RETRY: Duration = Duration::from_secs(2);
/// Ring size: a quarter of a second.
const RING: usize = SAMPLE_RATE as usize / 4 * CHANNELS;

pub struct Player {
    commands: Sender<Command>,
    shared: Arc<Shared>,
    stream: Option<cpal::Stream>,
    /// Last attempt at opening a device.
    opened_at: Instant,
    /// Streams built so far (1 + rebuilds).
    builds: u32,
    started_at: f64,
    started: Option<Instant>,
}

impl Player {
    /// Opens the default output device. Without one, playback still runs on a
    /// wall clock, silently.
    pub fn new() -> Self {
        vf_media::init();
        let shared = Arc::new(Shared { played: AtomicU64::new(0), flush: AtomicU64::new(0), playing: AtomicBool::new(false), broken: AtomicBool::new(false) });
        let (producer, consumer) = rtrb::RingBuffer::<f32>::new(RING);
        let stream = open_stream(consumer, shared.clone());
        if stream.is_none() {
            log::warn!("no audio output device; playback is silent");
        }
        let (commands, receiver) = channel();
        let mixer_shared = shared.clone();
        let loopback = commands.clone();
        std::thread::Builder::new()
            .name("audio-mixer".into())
            .spawn(move || mixer::run(receiver, loopback, producer, mixer_shared))
            .expect("audio mixer thread");
        Player { commands, shared, stream, opened_at: Instant::now(), builds: 1, started_at: 0.0, started: None }
    }

    /// Call regularly (every UI frame is fine): rebuilds the output stream
    /// after the device changed or went away, and picks up a device that
    /// appeared while there was none.
    pub fn maintain(&mut self) {
        let broken = self.shared.broken.load(Ordering::Acquire);
        let missing = self.stream.is_none() && self.opened_at.elapsed() >= RETRY;
        if broken || missing {
            self.rebuild();
        }
    }

    fn rebuild(&mut self) {
        self.shared.broken.store(false, Ordering::Release);
        self.opened_at = Instant::now();
        // The old stream stops before the new one starts.
        self.stream = None;
        let (producer, consumer) = rtrb::RingBuffer::<f32>::new(RING);
        self.stream = open_stream(consumer, self.shared.clone());
        if self.stream.is_some() {
            self.builds += 1;
            log::info!("audio output rebuilt for the current device");
            let _ = self.commands.send(Command::Output(producer));
        }
    }

    /// The project to play (call again after every change).
    pub fn set_project(&self, project: Arc<Project>) {
        let _ = self.commands.send(Command::Project(project));
    }

    pub fn set_master(&self, volume: f32) {
        let _ = self.commands.send(Command::Master(volume.clamp(0.0, 2.0)));
    }

    /// Starts playing from `time` (timeline seconds).
    pub fn play(&mut self, time: f64) {
        self.started_at = time;
        self.started = Some(Instant::now());
        let _ = self.commands.send(Command::Play(time));
    }

    pub fn stop(&mut self) {
        self.started = None;
        let _ = self.commands.send(Command::Stop);
    }

    pub fn is_playing(&self) -> bool {
        self.started.is_some()
    }

    /// Current playback position: follows the samples the device has played,
    /// or the wall clock when there is no device.
    pub fn time(&self) -> f64 {
        let Some(started) = self.started else { return self.started_at };
        if self.stream.is_some() && self.shared.playing.load(Ordering::Acquire) {
            let played = self.shared.played.load(Ordering::Acquire) as f64 / SAMPLE_RATE as f64;
            // Until the first samples reach the device, hold the start position.
            return self.started_at + played;
        }
        self.started_at + started.elapsed().as_secs_f64()
    }
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

fn open_stream(mut consumer: rtrb::Consumer<f32>, shared: Arc<Shared>) -> Option<cpal::Stream> {
    let host = cpal::default_host();
    let device = host.default_output_device()?;
    let config = device.default_output_config().ok()?;
    let rate = config.sample_rate() as f64;
    let channels = config.channels() as usize;
    if config.sample_format() != cpal::SampleFormat::F32 {
        log::warn!("audio device format {:?} is not f32", config.sample_format());
    }
    let step = SAMPLE_RATE as f64 / rate;
    let shared_errors = shared.clone();
    // Linear resampling from 48 kHz to the device rate.
    let mut phase = 0.0f64;
    let mut previous = [0.0f32; 2];
    let mut next = [0.0f32; 2];
    let mut have_next = false;
    let mut generation = 0u64;
    let stream = device
        .build_output_stream::<f32, _, _>(
            config.config(),
            move |data: &mut [f32], _| {
                let flush = shared.flush.load(Ordering::Acquire);
                if flush != generation {
                    generation = flush;
                    while consumer.pop().is_ok() {}
                    phase = 0.0;
                    have_next = false;
                    previous = [0.0; 2];
                    shared.played.store(0, Ordering::Release);
                }
                let playing = shared.playing.load(Ordering::Acquire);
                let mut consumed = 0u64;
                for frame in data.chunks_mut(channels) {
                    if !playing {
                        frame.fill(0.0);
                        continue;
                    }
                    while phase >= 1.0 || !have_next {
                        if have_next {
                            previous = next;
                            phase -= 1.0;
                        }
                        match (consumer.pop(), consumer.pop()) {
                            (Ok(left), Ok(right)) => {
                                next = [left, right];
                                have_next = true;
                                consumed += 1;
                            }
                            // Underrun: repeat silence rather than stall the clock.
                            _ => {
                                next = [0.0, 0.0];
                                have_next = true;
                                break;
                            }
                        }
                    }
                    let t = phase as f32;
                    let left = previous[0] + (next[0] - previous[0]) * t;
                    let right = previous[1] + (next[1] - previous[1]) * t;
                    match frame.len() {
                        1 => frame[0] = (left + right) * 0.5,
                        _ => {
                            frame[0] = left;
                            frame[1] = right;
                            for extra in &mut frame[2..] {
                                *extra = 0.0;
                            }
                        }
                    }
                    phase += step;
                }
                if consumed > 0 {
                    shared.played.fetch_add(consumed, Ordering::AcqRel);
                }
            },
            {
                let shared = shared_errors;
                move |error: cpal::Error| {
                    use cpal::ErrorKind::*;
                    match error.kind() {
                        // Rerouted to another device (maybe another rate or
                        // channel count), or gone: build a stream for what is there now.
                        DeviceChanged | DeviceNotAvailable | StreamInvalidated => {
                            log::info!("audio output: {error}");
                            shared.broken.store(true, Ordering::Release);
                        }
                        _ => log::error!("audio output: {error}"),
                    }
                }
            },
            None,
        )
        .ok()?;
    stream.play().ok()?;
    Some(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A changed or lost device gets a fresh stream; the clock keeps running.
    #[test]
    fn a_broken_output_is_rebuilt() {
        let mut player = Player::new();
        // Silent: no project, so nothing plays.
        player.set_master(0.0);
        let had_device = player.stream.is_some();
        player.shared.broken.store(true, Ordering::Release);
        player.maintain();
        assert!(!player.shared.broken.load(Ordering::Acquire));
        if had_device {
            assert_eq!(player.builds, 2, "the stream was rebuilt");
            assert!(player.stream.is_some());
        }
        player.play(1.0);
        std::thread::sleep(Duration::from_millis(150));
        assert!(player.time() > 1.0, "playback time moves after a rebuild: {}", player.time());
        player.stop();
    }
}
