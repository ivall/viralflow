//! How the preview keeps up with a phone-sized video: playback at real time,
//! scrubbing back and stepping frame by frame. Run with
//! `cargo test --release -p vf-render --test preview_bench -- --ignored --nocapture`.

use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use vf_core::model::*;
use vf_core::presets::create_project;
use vf_render::compositor::render_target;
use vf_render::{FrameRenderer, Gpu, Mode, TextEngine};

fn phone_video() -> Option<std::path::PathBuf> {
    let dir = std::env::temp_dir().join("vf-render-fixtures");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("phone-1080x1920.mp4");
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
                "testsrc2=s=1080x1920:r=30:d=8",
                "-c:v",
                "libx264",
                "-preset",
                "fast",
                "-crf",
                "23",
                "-g",
                "30",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&path)
            .status()
            .ok()?;
        status.success().then_some(())?;
    }
    Some(path)
}

fn project(path: &std::path::Path) -> Project {
    let mut project = create_project("Bench", 1080, 1920, 30);
    project.assets.push(MediaAsset {
        id: "a".into(),
        kind: MediaKind::Video,
        name: "phone".into(),
        path: path.display().to_string(),
        duration: Some(8.0),
        width: Some(1080.0),
        height: Some(1920.0),
        size: 1,
        ..Default::default()
    });
    let video = TimelineItem {
        id: "v".into(),
        kind: ItemKind::Video,
        start: 0.0,
        duration: 8.0,
        asset_id: Some("a".into()),
        transform: Some(Transform::default()),
        ..Default::default()
    };
    project.tracks = vec![Track { id: "tv".into(), kind: TrackKind::Video, items: vec![video], ..Default::default() }];
    project
}

/// Memory the process holds in RAM (what the task manager shows), in MB.
#[cfg(windows)]
fn resident_mb() -> f64 {
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        cb: u32,
        page_faults: u32,
        peak_working_set: usize,
        working_set: usize,
        quota_peak_paged: usize,
        quota_paged: usize,
        quota_peak_non_paged: usize,
        quota_non_paged: usize,
        pagefile: usize,
        peak_pagefile: usize,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut Counters, size: u32) -> i32;
    }
    let mut counters = Counters { cb: std::mem::size_of::<Counters>() as u32, ..Default::default() };
    // SAFETY: a counters struct of the stated size for the current process.
    unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    counters.working_set as f64 / 1048576.0
}

#[cfg(not(windows))]
fn resident_mb() -> f64 {
    0.0
}

/// Renders `time` until the picture is exact; returns how long that took.
fn settle(renderer: &mut FrameRenderer, project: &Project, time: f64, target: &wgpu::Texture) -> Duration {
    let started = Instant::now();
    while !renderer.render(project, time, target, 0.5).complete {
        std::thread::sleep(Duration::from_millis(1));
        if started.elapsed() > Duration::from_secs(5) {
            break;
        }
    }
    started.elapsed()
}

#[test]
#[ignore]
fn preview_keeps_up_with_a_phone_video() {
    let Some(path) = phone_video() else { return };
    let Ok(gpu) = Gpu::new() else { return };
    let project = project(&path);
    let mut renderer = FrameRenderer::new(&gpu, Arc::new(TextEngine::new()), Mode::Preview);
    let target = render_target(&gpu, 540, 960);
    let before = resident_mb();
    let first = settle(&mut renderer, &project, 0.0, &target);

    // Playback: 5 s at 30 fps on the wall clock.
    renderer.set_playing(true);
    let started = Instant::now();
    let (mut exact, mut frames) = (0, 0);
    while started.elapsed() < Duration::from_secs(5) {
        let time = started.elapsed().as_secs_f64();
        if renderer.render(&project, time, &target, 0.5).complete {
            exact += 1;
        }
        frames += 1;
        let next = Duration::from_secs_f64(frames as f64 / 30.0);
        if let Some(wait) = next.checked_sub(started.elapsed()) {
            std::thread::sleep(wait);
        }
    }
    renderer.set_playing(false);
    println!(
        "memory: {before:.0} MB before, {:.0} MB after 5 s of playback ({:.0} MB of cached frames)",
        resident_mb(),
        vf_render::preview_frame_bytes() as f64 / 1048576.0
    );

    // Scrubbing back over what was just played, then forward frame by frame.
    let back: Vec<Duration> = (0..20).map(|i| settle(&mut renderer, &project, 4.8 - i as f64 * 0.2, &target)).collect();
    let steps: Vec<Duration> = (0..30).map(|i| settle(&mut renderer, &project, 6.0 + i as f64 / 30.0, &target)).collect();
    let average = |d: &[Duration]| d.iter().sum::<Duration>().as_secs_f64() * 1000.0 / d.len() as f64;
    let worst = |d: &[Duration]| d.iter().max().unwrap().as_secs_f64() * 1000.0;
    println!("first frame: {:.0} ms", first.as_secs_f64() * 1000.0);
    println!("playback: {exact}/{frames} frames exact ({:.0} %)", exact as f64 / frames as f64 * 100.0);
    println!("scrub back over played part: {:.1} ms average, {:.0} ms worst", average(&back), worst(&back));
    println!("step forward frame by frame: {:.1} ms average, {:.0} ms worst", average(&steps), worst(&steps));
}
