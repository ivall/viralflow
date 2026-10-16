//! Decoding speed of heavy phone formats, on the GPU and on the CPU. Run with
//! `cargo test --release -p vf-media --test decode_bench -- --ignored --nocapture --test-threads 1`
//! (uses the videos the vf-render preview benchmark folder holds).

use std::time::Instant;

use vf_media::video::VideoDecoder;

fn fixture(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::temp_dir().join("vf-render-fixtures").join(name);
    path.is_file().then_some(path)
}

/// Seconds of CPU time this process has used (all threads).
#[cfg(windows)]
fn cpu_seconds() -> f64 {
    #[repr(C)]
    #[derive(Default)]
    struct FileTime(u32, u32);
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn GetProcessTimes(process: isize, created: *mut FileTime, exited: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
    }
    let (mut a, mut b, mut kernel, mut user) = (FileTime::default(), FileTime::default(), FileTime::default(), FileTime::default());
    // SAFETY: valid out pointers for the current process.
    unsafe { GetProcessTimes(GetCurrentProcess(), &mut a, &mut b, &mut kernel, &mut user) };
    let ticks = |t: &FileTime| ((t.1 as u64) << 32 | t.0 as u64) as f64 / 1e7;
    ticks(&kernel) + ticks(&user)
}

#[cfg(not(windows))]
fn cpu_seconds() -> f64 {
    0.0
}

fn run(name: &str, hardware: bool) -> Option<Vec<u8>> {
    let path = fixture(name)?;
    std::env::set_var("VF_HWDECODE", if hardware { "" } else { "off" });
    let mut decoder = VideoDecoder::open(&path, 1920, 1920).unwrap();
    let on_gpu = decoder.is_hardware();
    let (started, cpu) = (Instant::now(), cpu_seconds());
    let mut last = None;
    for i in 0..90 {
        last = decoder.frame_at(i as f64 / 30.0).unwrap();
    }
    let took = started.elapsed().as_secs_f64();
    let used = cpu_seconds() - cpu;
    println!("{name} on the {}: {:.0} fps, CPU {:.0} % of one core", if on_gpu { "GPU" } else { "CPU" }, 90.0 / took, used / took * 100.0);
    last.map(|image| image.data)
}

#[test]
#[ignore]
fn heavy_formats_decode_on_the_gpu() {
    for name in ["hevc-4k.mp4", "h264-4k.mp4", "phone-1080x1920.mp4"] {
        let (Some(gpu), Some(cpu)) = (run(name, true), run(name, false)) else { continue };
        let difference = gpu.iter().zip(&cpu).map(|(a, b)| (*a as i32 - *b as i32).unsigned_abs() as u64).sum::<u64>() as f64 / gpu.len() as f64;
        println!("  same picture: average difference {difference:.3} levels");
        assert!(difference < 1.0, "{name}: GPU and CPU frames differ by {difference}");
    }
    std::env::remove_var("VF_HWDECODE");
}

#[test]
#[ignore]
fn filmstrips_decode_one_frame_each() {
    let long_gop = std::env::temp_dir().join("vf-render-fixtures").join("long-gop-1080.mp4");
    if !long_gop.is_file() {
        let _ = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=s=1080x1920:r=30:d=16",
                "-c:v",
                "libx264",
                "-preset",
                "fast",
                "-g",
                "240",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&long_gop)
            .status();
    }
    for name in ["hevc-4k.mp4", "phone-1080x1920.mp4", "long-gop-1080.mp4"] {
        let Some(path) = fixture(name) else { continue };
        let duration = vf_media::probe::probe(&path).unwrap().duration.unwrap();
        let count = duration.ceil() as usize;
        let started = Instant::now();
        let mut every = VideoDecoder::open(&path, 256, 64).unwrap();
        for index in 0..count {
            every.frame_at(duration * (index as f64 + 0.5) / count as f64).unwrap();
        }
        let before = started.elapsed().as_secs_f64() * 1000.0;
        let started = Instant::now();
        let frames = vf_media::overview::filmstrip(&path, duration, count, 64).unwrap();
        let after = started.elapsed().as_secs_f64() * 1000.0;
        println!("{name}: filmstrip of {} frames: {before:.0} ms decoding every frame, {after:.0} ms keyframes only", frames.len());
    }
}
