//! Decoding against real files generated with the ffmpeg command line tool
//! (skipped when it is not installed).

use std::path::{Path, PathBuf};
use std::process::Command;

use vf_core::model::MediaKind;
use vf_media::audio::{AudioDecoder, SAMPLE_RATE};
use vf_media::probe::probe;
use vf_media::video::VideoDecoder;

fn fixtures() -> Option<PathBuf> {
    let dir = std::env::temp_dir().join("vf-media-fixtures");
    std::fs::create_dir_all(&dir).ok()?;
    let make = |name: &str, args: &[&str]| -> Option<()> {
        let path = dir.join(name);
        if path.is_file() {
            return Some(());
        }
        let status = Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).arg(&path).status().ok()?;
        status.success().then_some(())
    };
    // Red channel = 8 × frame number, so each frame identifies itself.
    make(
        "counter.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=64x48:r=30:d=1,format=rgb24,geq=r='8*N':g=0:b=0",
            "-c:v",
            "libx264",
            "-g",
            "10",
            "-crf",
            "0",
            "-pix_fmt",
            "yuv444p",
        ],
    )?;
    make("tone.wav", &["-f", "lavfi", "-i", "sine=f=440:d=1:sample_rate=44100"])?;
    make("rotated.mp4", &["-display_rotation", "90", "-i", dir.join("counter.mp4").to_str()?, "-c", "copy"])?;
    Some(dir)
}

fn red_at(decoder: &mut VideoDecoder, time: f64) -> u8 {
    let frame = decoder.frame_at(time).unwrap().expect("frame");
    let center = ((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize;
    frame.data[center]
}

fn frame_number(red: u8) -> i32 {
    (red as f64 / 8.0).round() as i32
}

#[test]
fn probes_video_and_audio() {
    let Some(dir) = fixtures() else { return };
    let video = probe(&dir.join("counter.mp4")).unwrap();
    assert_eq!(video.kind, Some(MediaKind::Video));
    assert_eq!((video.width, video.height), (Some(64), Some(48)));
    assert!((video.fps.unwrap() - 30.0).abs() < 0.01);
    assert!((video.duration.unwrap() - 1.0).abs() < 0.1);
    let audio = probe(&dir.join("tone.wav")).unwrap();
    assert_eq!(audio.kind, Some(MediaKind::Audio));
    assert!(audio.has_audio);
}

#[test]
fn frames_are_exact_after_jumps() {
    let Some(dir) = fixtures() else { return };
    let mut decoder = VideoDecoder::open(&dir.join("counter.mp4"), 64, 48).unwrap();
    // Forward, a jump back across a keyframe, a jump forward, then sequential.
    for (time, expected) in [(0.0, 0), (0.5, 15), (0.12, 4), (0.9, 27), (0.93, 28), (0.966, 29), (0.2, 6)] {
        assert_eq!(frame_number(red_at(&mut decoder, time)), expected, "frame at {time}");
    }
}

#[test]
fn rotation_is_applied() {
    let Some(dir) = fixtures() else { return };
    let path: &Path = &dir.join("rotated.mp4");
    let info = probe(path).unwrap();
    assert_eq!(info.rotation % 180, 90);
    assert_eq!((info.width, info.height), (Some(48), Some(64)));
    let mut decoder = VideoDecoder::open(path, 1000, 1000).unwrap();
    let frame = decoder.frame_at(0.0).unwrap().unwrap();
    assert_eq!((frame.width, frame.height), (48, 64));
}

#[test]
fn audio_reads_and_seeks() {
    let Some(dir) = fixtures() else { return };
    let mut decoder = AudioDecoder::open(&dir.join("tone.wav")).unwrap();
    let mut total = 0;
    let mut chunk = vec![0.0f32; 4096];
    loop {
        let read = decoder.read(&mut chunk).unwrap();
        total += read;
        if read < chunk.len() {
            break;
        }
    }
    let frames = total / 2;
    assert!((frames as i64 - SAMPLE_RATE as i64).abs() < 1200, "{frames} frames");
    decoder.seek(0.5).unwrap();
    let read = decoder.read(&mut chunk).unwrap();
    assert!(read > 0);
    assert!((decoder.position() - (0.5 + (read / 2) as f64 / SAMPLE_RATE as f64)).abs() < 0.01);
    assert!(chunk.iter().any(|s| s.abs() > 0.05), "tone is audible after seeking");
}

#[test]
fn keyframe_decoding_returns_the_keyframe_before() {
    let Some(dir) = fixtures() else { return };
    // Keyframes every 10 frames (0, 10, 20 at 30 fps).
    let mut decoder = VideoDecoder::open_keyframes(&dir.join("counter.mp4"), 64, 48).unwrap();
    let mut number = |time: f64| {
        let frame = decoder.keyframe_at(time).unwrap().expect("keyframe");
        frame_number(frame.data[((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize])
    };
    assert_eq!(number(0.0), 0);
    assert_eq!(number(0.2), 0);
    assert_eq!(number(0.5), 10);
    assert_eq!(number(0.7), 20);
    assert_eq!(number(0.34), 10);
}

/// HEVC (decoded on the GPU where there is one) gives the same frames as the CPU.
#[test]
fn hevc_frames_match_on_gpu_and_cpu() {
    let Some(dir) = fixtures() else { return };
    let path = dir.join("counter-hevc.mp4");
    if !path.is_file() {
        let made = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "color=c=black:s=128x96:r=30:d=1,format=rgb24,geq=r='8*N':g=0:b=0"])
            .args(["-c:v", "libx265", "-x265-params", "log-level=error:keyint=10:lossless=1", "-pix_fmt", "yuv420p"])
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success());
        if !made {
            return;
        }
    }
    let mut gpu = VideoDecoder::open(&path, 128, 96).unwrap();
    println!("HEVC on the GPU: {}", gpu.is_hardware());
    for time in [0.0, 0.5, 0.2, 0.9, 0.1] {
        let expected = (time * 30.0f64).round() as i32;
        assert_eq!(frame_number(red_at(&mut gpu, time)), expected, "frame at {time}");
    }
}
