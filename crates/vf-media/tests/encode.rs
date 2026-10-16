//! Round trips through the encoder: what is written decodes back.

use vf_media::audio::{AudioDecoder, SAMPLE_RATE};
use vf_media::encode::{Container, EncodeSettings, Encoder, VideoCodec};
use vf_media::probe::probe;
use vf_media::video::VideoDecoder;

fn encode(name: &str, codec: VideoCodec, container: Container, hardware: bool) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join("vf-media-encode");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let (width, height, fps) = (320u32, 180u32, 30u32);
    let settings = EncodeSettings { path: path.clone(), width, height, fps, codec, container, quality: 2, hardware, audio: true };
    let mut encoder = Encoder::create(&settings).unwrap();
    let used = encoder.encoder_name.clone();
    let samples_per_frame = SAMPLE_RATE as usize / fps as usize;
    for frame in 0..30 {
        // Orange picture; audio a 440 Hz tone.
        let rgba: Vec<u8> = (0..width * height).flat_map(|_| [230u8, 120, 30, 255]).collect();
        encoder.push_video(&rgba).unwrap();
        let audio: Vec<f32> = (0..samples_per_frame)
            .flat_map(|i| {
                let t = (frame * samples_per_frame + i) as f32 / SAMPLE_RATE as f32;
                let v = (t * 440.0 * std::f32::consts::TAU).sin() * 0.5;
                [v, v]
            })
            .collect();
        encoder.push_audio(&audio).unwrap();
    }
    encoder.finish().unwrap();
    (path, used)
}

fn check(path: &std::path::Path) {
    let info = probe(path).unwrap();
    assert_eq!((info.width, info.height), (Some(320), Some(180)), "{info:?}");
    assert!(info.has_audio, "{info:?}");
    let duration = info.duration.unwrap();
    assert!((duration - 1.0).abs() < 0.15, "duration {duration}");
    let mut video = VideoDecoder::open(path, 320, 180).unwrap();
    let frame = video.frame_at(0.5).unwrap().expect("frame");
    let center = ((90 * 320 + 160) * 4) as usize;
    let rgb = &frame.data[center..center + 3];
    assert!((rgb[0] as i32 - 230).abs() < 12 && (rgb[1] as i32 - 120).abs() < 12 && (rgb[2] as i32 - 30).abs() < 12, "colour {rgb:?}");
    let mut audio = AudioDecoder::open(path).unwrap();
    let mut buffer = vec![0.0f32; SAMPLE_RATE as usize];
    audio.read(&mut buffer).unwrap();
    let peak = buffer.iter().fold(0.0f32, |a, b| a.max(b.abs()));
    assert!(peak > 0.3, "audio peak {peak}");
}

#[test]
fn h264_aac_mp4() {
    let (path, used) = encode("h264.mp4", VideoCodec::H264, Container::Mp4, false);
    assert_eq!(used, "libx264");
    check(&path);
}

#[test]
fn vp9_opus_webm() {
    let (path, _) = encode("vp9.webm", VideoCodec::Vp9, Container::Webm, false);
    check(&path);
}

#[test]
fn h265_mov() {
    let (path, _) = encode("h265.mov", VideoCodec::H265, Container::Mov, false);
    check(&path);
}

#[test]
fn hardware_or_fallback() {
    let (path, used) = encode("hw.mp4", VideoCodec::H264, Container::Mp4, true);
    eprintln!("hardware export used {used}");
    check(&path);
}
