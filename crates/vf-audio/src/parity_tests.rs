//! The preview and the export must sound the same: both go through
//! [`crate::mixer::Mix`] and the same master stage.

use std::path::{Path, PathBuf};

use vf_core::model::{ItemKind, MediaAsset, MediaKind, Project, TimelineItem, Track, TrackKind};
use vf_media::audio::{CHANNELS, SAMPLE_RATE};

use crate::mixdown::mixdown;
use crate::mixer::{measured_gain, preview};
use crate::voice::VoiceStream;

const RATE: f64 = SAMPLE_RATE as f64;

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vf-audio-parity-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 16-bit stereo 48 kHz WAV of `frames` frames from `sample(frame) -> (l, r)`.
fn write_wav(path: &Path, frames: usize, sample: impl Fn(usize) -> (f64, f64)) {
    let mut data = Vec::with_capacity(frames * 4);
    for i in 0..frames {
        let (l, r) = sample(i);
        for v in [l, r] {
            data.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
    }
    let mut file = Vec::new();
    file.extend_from_slice(b"RIFF");
    file.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    file.extend_from_slice(b"WAVEfmt ");
    file.extend_from_slice(&16u32.to_le_bytes());
    file.extend_from_slice(&1u16.to_le_bytes());
    file.extend_from_slice(&2u16.to_le_bytes());
    file.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    file.extend_from_slice(&(SAMPLE_RATE * 4).to_le_bytes());
    file.extend_from_slice(&4u16.to_le_bytes());
    file.extend_from_slice(&16u16.to_le_bytes());
    file.extend_from_slice(b"data");
    file.extend_from_slice(&(data.len() as u32).to_le_bytes());
    file.extend_from_slice(&data);
    std::fs::write(path, file).unwrap();
}

/// A voice-like test signal: a tone that wobbles in pitch and level, plus hiss.
fn speechy(i: usize) -> (f64, f64) {
    let t = i as f64 / RATE;
    let envelope = 0.35 + 0.25 * (t * 3.1).sin();
    let tone = (std::f64::consts::TAU * (220.0 * t + 12.0 * (t * 2.0).sin())).sin();
    let hiss = (((i * 1_103_515_245 + 12_345) % 65_536) as f64 / 65_536.0 - 0.5) * 0.04;
    (envelope * tone + hiss, envelope * tone * 0.8 - hiss)
}

fn sine_wav(dir: &Path, name: &str, seconds: f64, frequency: f64) -> PathBuf {
    let path = dir.join(name);
    write_wav(&path, (seconds * RATE) as usize, |i| {
        let v = (std::f64::consts::TAU * frequency * i as f64 / RATE).sin() * 0.5;
        (v, v)
    });
    path
}

fn asset(id: &str, path: &Path, seconds: f64) -> MediaAsset {
    MediaAsset {
        id: id.into(),
        kind: MediaKind::Audio,
        name: id.into(),
        path: path.display().to_string(),
        duration: Some(seconds),
        has_audio: Some(true),
        size: 1,
        ..Default::default()
    }
}

fn clip(id: &str, asset: &str, start: f64, duration: f64) -> TimelineItem {
    TimelineItem { id: id.into(), kind: ItemKind::Audio, start, duration, asset_id: Some(asset.into()), ..Default::default() }
}

fn project(items: Vec<TimelineItem>, assets: Vec<MediaAsset>, loudness: Option<f64>) -> Project {
    let mut project = Project { assets, tracks: vec![Track { id: "audio".into(), kind: TrackKind::Audio, items, ..Default::default() }], ..Default::default() };
    project.export.loudness = loudness;
    project
}

/// Frequency from rising zero crossings of the left channel.
fn frequency(samples: &[f32]) -> f64 {
    let left: Vec<f32> = samples.chunks_exact(CHANNELS).map(|f| f[0]).collect();
    let crossings = left.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
    crossings as f64 / (left.len() as f64 / RATE)
}

fn rms(samples: &[f32]) -> f64 {
    (samples.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / samples.len().max(1) as f64).sqrt()
}

#[test]
fn speed_changes_keep_the_pitch() {
    vf_media::init();
    let dir = temp_dir();
    let path = sine_wav(&dir, "tone.wav", 4.0, 440.0);
    for speed in [0.5, 2.0, 3.0] {
        let mut stream = VoiceStream::open(&path, 0.0, speed, 0.0).unwrap();
        let mut out = vec![0.0f32; (RATE * 1.0) as usize * CHANNELS];
        stream.read(&mut out).unwrap();
        // Skip the first 0.1 s where the tempo filter settles.
        let heard = frequency(&out[(RATE * 0.1) as usize * CHANNELS..]);
        assert!((heard - 440.0).abs() < 8.0, "speed {speed}: {heard} Hz");
    }
}

#[test]
fn filters_do_not_shift_the_sound_against_the_picture() {
    vf_media::init();
    let dir = temp_dir();
    let path = dir.join("click.wav");
    // Quiet hiss with a loud click at 1.5 s.
    write_wav(&path, (RATE * 3.0) as usize, |i| {
        let click = if (i as f64 - 1.5 * RATE).abs() < 24.0 { 0.9 } else { 0.0 };
        let hiss = (((i * 1_103_515_245 + 12_345) % 65_536) as f64 / 65_536.0 - 0.5) * 0.01;
        (click + hiss, click + hiss)
    });
    for (speed, enhance) in [(1.0, 1.0), (2.0, 0.0), (1.5, 0.7)] {
        let mut stream = VoiceStream::open(&path, 0.0, speed, enhance).unwrap();
        let mut out = vec![0.0f32; (RATE * 2.0) as usize * CHANNELS];
        stream.read(&mut out).unwrap();
        let loudest = out.chunks_exact(CHANNELS).enumerate().max_by(|a, b| a.1[0].abs().total_cmp(&b.1[0].abs())).unwrap().0;
        let at = loudest as f64 / RATE;
        let expected = 1.5 / speed;
        // Time stretching may move a lone click by up to half its window (about 15 ms).
        let allowed = if speed == 1.0 { 0.001 } else { 0.02 };
        assert!((at - expected).abs() < allowed, "speed {speed}, enhance {enhance}: click at {at:.4} s, expected {expected:.4} s");
    }
}

#[test]
fn preview_plays_exactly_what_the_export_renders() {
    vf_media::init();
    let dir = temp_dir();
    let voice = dir.join("voice.wav");
    write_wav(&voice, (RATE * 8.0) as usize, speechy);
    let music = sine_wav(&dir, "music.wav", 8.0, 330.0);
    let mut talk = clip("talk", "voice", 0.25, 3.0);
    talk.speed = Some(1.5);
    talk.source_in = Some(0.5);
    talk.voice_enhance = Some(0.6);
    talk.fade_in = Some(0.2);
    talk.fade_out = Some(0.4);
    let mut bed = clip("bed", "music", 1.0, 3.5);
    bed.volume = Some(0.4);
    bed.speed = Some(0.8);
    let total = 4.5;
    let frames = (total * RATE) as usize;
    for loudness in [None, Some(-14.0)] {
        let project = project(vec![talk.clone(), bed.clone()], vec![asset("voice", &voice, 8.0), asset("music", &music, 8.0)], loudness);
        let exported = mixdown(&project, total, &|_| {}, &|| false).unwrap().unwrap();
        assert_eq!(&exported[..(0.25 * RATE) as usize * CHANNELS].iter().fold(0.0f32, |a, b| a.max(b.abs())), &0.0, "silence before the first clip");
        let played = preview(&project, 0.0, frames, measured_gain(&project));
        assert_eq!(exported.len(), played.len());
        let worst = exported.iter().zip(&played).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(worst < 1e-6, "loudness {loudness:?}: preview differs from export by {worst}");
        assert!(rms(&exported) > 0.05, "the mix is audible: {}", rms(&exported));
        if let Some(target) = loudness {
            let measured = crate::loudness::integrated(&exported).unwrap();
            assert!((measured - target).abs() < 1.0, "normalised to {measured:.2} LUFS");
        }
    }
}

#[test]
fn playing_from_the_middle_matches_the_export_after_a_moment() {
    vf_media::init();
    let dir = temp_dir();
    let voice = dir.join("voice2.wav");
    write_wav(&voice, (RATE * 8.0) as usize, speechy);
    let mut talk = clip("talk", "voice", 0.0, 4.0);
    talk.speed = Some(1.25);
    talk.voice_enhance = Some(0.5);
    let project = project(vec![talk], vec![asset("voice", &voice, 8.0)], None);
    let exported = mixdown(&project, 4.0, &|_| {}, &|| false).unwrap().unwrap();
    let start = 1.7;
    let played = preview(&project, start, (RATE * 2.0) as usize, 1.0);
    // Filters start afresh at the seek; after 0.3 s both run in step.
    let settle = (RATE * 0.3) as usize * CHANNELS;
    let offset = (start * RATE) as usize * CHANNELS;
    let reference = &exported[offset + settle..offset + played.len()];
    // Time stretching restarted elsewhere lines its windows up differently,
    // so compare the level over 20 ms windows rather than the waveform.
    let window = (RATE * 0.02) as usize * CHANNELS;
    let level = |samples: &[f32]| samples.chunks(window).map(rms).collect::<Vec<_>>();
    let (expected, heard) = (level(reference), level(&played[settle..]));
    let difference: Vec<f32> = expected.iter().zip(&heard).map(|(a, b)| (a - b) as f32).collect();
    let as_f32: Vec<f32> = expected.iter().map(|v| *v as f32).collect();
    let ratio = rms(&difference) / rms(&as_f32);
    println!("level difference after a seek: {:.2} %", ratio * 100.0);
    assert!(ratio < 0.05, "preview after a seek strays from the export by {:.1} %", ratio * 100.0);
}

#[test]
#[ignore]
fn print_filter_latencies() {
    vf_media::init();
    let rate = SAMPLE_RATE as usize;
    // Silence, then a 1 kHz burst from 0.5 s.
    let input: Vec<f32> = (0..rate * 2)
        .flat_map(|i| {
            let v = if i >= rate / 2 { (i as f32 * 1000.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5 } else { 0.0 };
            [v, v]
        })
        .collect();
    for chain in [
        "highpass=f=80",
        "afftdn=nf=-45:nr=24",
        "afftdn=nf=-25:nr=6",
        "deesser=i=0.5",
        "acompressor=threshold=0.2:ratio=4:attack=15:release=120",
        "alimiter=limit=0.95:level=false",
        "alimiter=limit=0.95:level=false:latency=1",
        "atempo=1.5",
        "atempo=0.8",
        "atempo=2.0",
        "atempo=2.0,atempo=1.5",
    ] {
        let out = vf_media::filter::apply(chain, &input).unwrap();
        let onset = out.chunks_exact(CHANNELS).position(|f| f[0].abs() > 0.2).unwrap();
        println!("{chain}: onset {onset}, len {}", out.len() / 2);
    }
}

/// Same reading as ffmpeg's EBU R128 meter on varied material (when the CLI is installed).
#[test]
fn loudness_matches_ffmpeg_ebur128() {
    vf_media::init();
    let dir = temp_dir();
    let path = dir.join("varied.wav");
    write_wav(&path, (RATE * 9.0) as usize, |i| {
        let t = i as f64 / RATE;
        let (l, r) = speechy(i);
        // Loud, quiet and silent stretches.
        let level = if t < 3.0 {
            1.0
        } else if t < 5.0 {
            0.15
        } else if t < 6.0 {
            0.0
        } else {
            0.6
        };
        (l * level, r * level * 0.5)
    });
    let Ok(output) =
        std::process::Command::new("ffmpeg").args(["-hide_banner", "-nostats", "-i"]).arg(&path).args(["-af", "ebur128", "-f", "null", "-"]).output()
    else {
        return;
    };
    let log = String::from_utf8_lossy(&output.stderr);
    let Some(summary) = log.rfind("Integrated loudness:") else { return };
    let reference: f64 =
        log[summary..].lines().find_map(|l| l.trim().strip_prefix("I:")).and_then(|v| v.trim().trim_end_matches("LUFS").trim().parse().ok()).unwrap();
    let mut decoder = vf_media::audio::AudioDecoder::open(&path).unwrap();
    let mut samples = vec![0.0f32; (RATE * 9.0) as usize * CHANNELS];
    decoder.read(&mut samples).unwrap();
    let ours = crate::loudness::integrated(&samples).unwrap();
    println!("loudness: ours {ours:.2} LUFS, ffmpeg {reference:.1} LUFS");
    assert!((ours - reference).abs() < 0.15, "ours {ours:.2} LUFS, ffmpeg {reference:.1} LUFS");
}

#[test]
#[ignore]
fn preview_runs_faster_than_real_time() {
    vf_media::init();
    let dir = temp_dir();
    let voice = dir.join("bench.wav");
    write_wav(&voice, (RATE * 30.0) as usize, speechy);
    let items: Vec<TimelineItem> = (0..3)
        .map(|n| {
            let mut item = clip(&format!("c{n}"), "voice", 0.0, 20.0);
            item.speed = Some(1.0 + n as f64 * 0.3);
            item.voice_enhance = Some(1.0);
            item
        })
        .collect();
    let project = project(items, vec![asset("voice", &voice, 30.0)], Some(-14.0));
    let started = std::time::Instant::now();
    let _ = preview(&project, 2.0, (RATE * 10.0) as usize, 1.0);
    let seek = started.elapsed();
    println!("10 s of preview with 3 cleaned, stretched clips: {:.0} ms ({:.1}% of real time)", seek.as_secs_f64() * 1000.0, seek.as_secs_f64() / 10.0 * 100.0);
    let started = std::time::Instant::now();
    let gain = measured_gain(&project);
    println!("loudness measurement of 20 s: {:.0} ms (gain {gain:.2})", started.elapsed().as_secs_f64() * 1000.0);
}
