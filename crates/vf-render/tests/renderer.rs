//! A project with a real video, text and a shape, rendered end to end
//! (skipped without a GPU or the ffmpeg command line tool).

use std::process::Command;
use std::sync::Arc;

use vf_core::model::*;
use vf_core::presets::{create_project, text_item};
use vf_render::compositor::{read, render_target};
use vf_render::{FrameRenderer, Gpu, Mode, TextEngine};

fn counter_video() -> Option<std::path::PathBuf> {
    let dir = std::env::temp_dir().join("vf-render-fixtures");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("counter.mp4");
    if !path.is_file() {
        let status = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=64x64:r=30:d=2,format=rgb24,geq=r='4*N':g=0:b=0",
                "-c:v",
                "libx264",
                "-crf",
                "0",
                "-pix_fmt",
                "yuv444p",
            ])
            .arg(&path)
            .status()
            .ok()?;
        status.success().then_some(())?;
    }
    Some(path)
}

#[test]
fn renders_video_text_and_shape() {
    let Some(path) = counter_video() else { return };
    let Ok(gpu) = Gpu::new() else { return };
    let mut project = create_project("Test", 64, 64, 30);
    project.assets.push(MediaAsset {
        id: "a".into(),
        kind: MediaKind::Video,
        name: "counter".into(),
        path: path.display().to_string(),
        duration: Some(2.0),
        width: Some(64.0),
        height: Some(64.0),
        size: 1,
        ..Default::default()
    });
    let video = TimelineItem {
        id: "v".into(),
        kind: ItemKind::Video,
        start: 0.0,
        duration: 2.0,
        asset_id: Some("a".into()),
        fit: Some(Fit::Fill),
        transform: Some(Transform::default()),
        ..Default::default()
    };
    let mut text = text_item("I", None);
    text.id = "t".into();
    text.start = 0.0;
    text.duration = 2.0;
    text.text.as_mut().unwrap().size = 40.0;
    text.text.as_mut().unwrap().color = "#00ff00".into();
    project.tracks = vec![
        Track { id: "tt".into(), kind: TrackKind::Text, items: vec![text], ..Default::default() },
        Track { id: "tv".into(), kind: TrackKind::Video, items: vec![video], ..Default::default() },
    ];
    let mut renderer = FrameRenderer::new(&gpu, Arc::new(TextEngine::new()), Mode::Export);
    let target = render_target(&gpu, 64, 64);
    let stats = renderer.render(&project, 1.0, &target, 1.0);
    assert!(stats.complete);
    let pixels = read(&gpu, &target);
    // Frame 30 of the counter: red = 4 × 30 = 120 (corner, away from the text).
    let corner = &pixels[(2 * 64 + 2) * 4..(2 * 64 + 2) * 4 + 4];
    assert!((corner[0] as i32 - 120).abs() <= 4, "video frame at 1 s: {corner:?}");
    let green = pixels.chunks_exact(4).filter(|p| p[1] > 200 && p[0] < 100).count();
    assert!(green > 20, "text is drawn: {green} green pixels");
}
