pub mod audio;
pub mod composite;
pub mod decode;
pub mod pixels;

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use tiny_skia::Pixmap;

use crate::engine::{media_layout, resolve_scene, track_connections, BoxProvider, Connection, Layer};
use crate::error::{AppError, AppResult};
use crate::ffmpeg::{command, hardware_encoders, stderr_tail, Tools};
use crate::model::{Project, TimelineItem};
use composite::{draw_layer, fill_background, Content};
use decode::{load_image, Geometry, VideoDecoder};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRaster {
    pub item_id: String,
    /// PNG encoded as base64.
    pub png: String,
    /// Box size in project pixels.
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub project: Project,
    pub output_path: String,
    #[serde(default)]
    pub rasters: Vec<TextRaster>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgress {
    pub job_id: String,
    pub progress: f64,
    pub stage: String,
    pub frame: u64,
    pub total_frames: u64,
    pub speed: f64,
    pub eta_seconds: Option<f64>,
}

#[derive(Default)]
pub struct Jobs {
    cancel: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl Jobs {
    pub fn register(&self, id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.cancel.lock().expect("jobs").insert(id.to_string(), flag.clone());
        flag
    }

    pub fn cancel(&self, id: &str) {
        if let Some(flag) = self.cancel.lock().expect("jobs").get(id) {
            flag.store(true, Ordering::SeqCst);
        }
    }

    pub fn finish(&self, id: &str) {
        self.cancel.lock().expect("jobs").remove(id);
    }
}

struct Rasters {
    boxes: HashMap<String, (f64, f64)>,
    pixmaps: HashMap<String, Pixmap>,
}

impl BoxProvider for Rasters {
    fn text_box(&self, item: &TimelineItem) -> Option<(f64, f64)> {
        self.boxes.get(&item.id).copied()
    }
}

fn even(value: f64) -> u32 {
    ((value.round() as u32).max(2) + 1) & !1
}

/// Highest scale a media item reaches, so it can be decoded sharp enough.
fn decode_factor(item: &TimelineItem) -> f64 {
    let mut max = item.transform.map(|t| t.scale).unwrap_or(1.0);
    for keyframe in &item.keyframes {
        max = max.max(keyframe.transform.scale);
    }
    let effect = matches!(item.effect.as_deref(), Some("zoom-in" | "zoom-out" | "punch"));
    if max <= 1.0001 && !effect {
        1.0
    } else {
        (max * 1.15).clamp(1.0, 2.5)
    }
}

fn video_args(settings: &crate::model::ExportSettings, hardware: Option<&str>) -> Vec<String> {
    let index = match settings.quality.as_str() {
        "draft" => 0,
        "standard" => 1,
        "ultra" => 3,
        _ => 2,
    };
    let s = |values: &[&str]| values.iter().map(|v| v.to_string()).collect::<Vec<_>>();
    match (settings.codec.as_str(), hardware) {
        ("vp9", _) => {
            let crf = [38, 33, 30, 24][index];
            let speed = ["4", "3", "2", "1"][index];
            let mut args = s(&["-c:v", "libvpx-vp9", "-b:v", "0", "-row-mt", "1", "-deadline", "good", "-cpu-used", speed]);
            args.extend(["-crf".into(), crf.to_string()]);
            args
        }
        ("h265", Some(encoder)) | ("h264", Some(encoder)) => {
            let q = [30, 26, 22, 18][index].to_string();
            let name = if settings.codec == "h265" { encoder.replace("h264", "hevc") } else { encoder.to_string() };
            let mut args = vec!["-c:v".to_string(), name.clone()];
            if name.contains("nvenc") {
                args.extend(s(&["-preset", "p5", "-rc", "vbr", "-b:v", "0", "-cq"]).into_iter().chain([q]));
            } else if name.contains("qsv") {
                args.extend(s(&["-global_quality"]).into_iter().chain([q]));
            } else if name.contains("amf") {
                args.extend(s(&["-rc", "cqp", "-qp_i"]).into_iter().chain([q.clone(), "-qp_p".into(), q]));
            } else {
                args.extend(s(&["-q:v", "60"]));
            }
            if name.starts_with("hevc") {
                args.extend(s(&["-tag:v", "hvc1"]));
            }
            args
        }
        ("h265", None) => {
            let crf = [30, 26, 22, 19][index];
            let preset = ["veryfast", "fast", "medium", "slow"][index];
            let mut args = s(&["-c:v", "libx265", "-preset", preset, "-tag:v", "hvc1", "-x265-params", "log-level=error"]);
            args.extend(["-crf".into(), crf.to_string()]);
            args
        }
        _ => {
            let crf = [28, 23, 19, 16][index];
            let preset = ["veryfast", "fast", "medium", "slow"][index];
            let mut args = s(&["-c:v", "libx264", "-preset", preset, "-profile:v", "high"]);
            args.extend(["-crf".into(), crf.to_string()]);
            args
        }
    }
}

pub struct Context<'a> {
    pub tools: &'a Tools,
    pub job_id: &'a str,
    pub jobs_dir: &'a Path,
    pub cancel: Arc<AtomicBool>,
    pub progress: &'a (dyn Fn(ExportProgress) + Sync),
}

struct DecoderSlot {
    decoder: VideoDecoder,
    end: f64,
}

pub fn export(context: Context, request: ExportRequest) -> AppResult<PathBuf> {
    let project = request.project;
    let output_path = PathBuf::from(&request.output_path);
    let total = project.content_end();
    if total < 0.05 {
        return Err(AppError::msg("The timeline is empty. Add a clip before exporting."));
    }
    for asset in &project.assets {
        let used = project.tracks.iter().flat_map(|track| &track.items).any(|item| item.asset_id.as_deref() == Some(&asset.id) && !item.hidden);
        if used && !Path::new(&asset.path).is_file() {
            return Err(AppError::msg(format!("Media file is missing: {}. Relink it in the Media panel.", asset.path)));
        }
    }
    let settings = project.export.clone();
    let fps = if [24, 25, 30, 50, 60].contains(&settings.fps) { settings.fps } else { 30 };
    let (pw, ph) = (project.width.max(16) as f64, project.height.max(16) as f64);
    let k = settings.resolution.clamp(240, 4320) as f64 / pw.min(ph);
    let (out_w, out_h) = (even(pw * k), even(ph * k));
    let total_frames = (total * fps as f64).ceil().max(1.0) as u64;
    let cancelled = || context.cancel.load(Ordering::SeqCst);
    let job_dir = context.jobs_dir.join(context.job_id);
    std::fs::create_dir_all(&job_dir)?;
    let report = |progress: f64, stage: &str, frame: u64, speed: f64, eta: Option<f64>| {
        (context.progress)(ExportProgress {
            job_id: context.job_id.to_string(),
            progress,
            stage: stage.to_string(),
            frame,
            total_frames,
            speed,
            eta_seconds: eta,
        })
    };

    let result = (|| -> AppResult<PathBuf> {
        report(0.0, "Mixing audio", 0, 0.0, None);
        let audio = audio::render_mix(context.tools, &project, total, &job_dir, |p| report(p * 0.05, "Mixing audio", 0, 0.0, None), &cancelled)?;

        report(0.05, "Preparing layers", 0, 0.0, None);
        let mut rasters = Rasters { boxes: HashMap::new(), pixmaps: HashMap::new() };
        for raster in request.rasters {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(raster.png.trim_start_matches("data:image/png;base64,"))
                .map_err(|_| AppError::msg("Invalid text image"))?;
            let pixmap = Pixmap::decode_png(&bytes).map_err(|error| AppError::msg(format!("Invalid text image: {error}")))?;
            rasters.boxes.insert(raster.item_id.clone(), (raster.width, raster.height));
            rasters.pixmaps.insert(raster.item_id, pixmap);
        }
        let mut images: HashMap<String, Pixmap> = HashMap::new();
        for track in &project.tracks {
            for item in track.items.iter().filter(|item| item.kind == "image" && !item.hidden) {
                let Some(asset) = project.asset(item.asset_id.as_deref()) else { continue };
                let (Some(sw), Some(sh)) = (asset.width, asset.height) else { continue };
                let layout = media_layout(sw, sh, item.crop.as_ref(), item.fit.as_deref().unwrap_or("contain"), item.focus.as_ref(), pw, ph);
                let factor = decode_factor(item);
                let geometry = Geometry::new(&layout, layout.box_w * k * factor, layout.box_h * k * factor);
                images.insert(item.id.clone(), load_image(context.tools, Path::new(&asset.path), geometry)?);
            }
        }
        let connections: Vec<Vec<Connection>> = project.tracks.iter().map(|track| track_connections(track, fps)).collect();

        let hardware = if settings.hardware && settings.codec != "vp9" { hardware_encoders(context.tools).into_iter().next() } else { None };
        let (format, partial) = {
            let format = match settings.format.as_str() {
                "mov" => "mov",
                "webm" => "webm",
                _ => "mp4",
            };
            (
                format,
                output_path.with_file_name(format!(
                    ".{}.partial",
                    output_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "export".into())
                )),
            )
        };
        let mut args: Vec<String> = vec![
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-y".into(),
            "-f".into(),
            "rawvideo".into(),
            "-pix_fmt".into(),
            "rgba".into(),
            "-s".into(),
            format!("{out_w}x{out_h}"),
            "-r".into(),
            fps.to_string(),
            "-i".into(),
            "-".into(),
        ];
        if let Some(audio) = &audio {
            args.extend(["-i".into(), audio.display().to_string()]);
        }
        args.extend(["-map".into(), "0:v".into()]);
        if audio.is_some() {
            args.extend(["-map".into(), "1:a".into()]);
        }
        args.extend(["-vf".into(), "scale=out_color_matrix=bt709:out_range=tv,format=yuv420p".into()]);
        args.extend(video_args(&settings, hardware.as_deref()));
        args.extend([
            "-colorspace".into(),
            "bt709".into(),
            "-color_primaries".into(),
            "bt709".into(),
            "-color_trc".into(),
            "bt709".into(),
            "-r".into(),
            fps.to_string(),
        ]);
        if audio.is_some() {
            if format == "webm" {
                args.extend(["-c:a".into(), "libopus".into(), "-b:a".into(), "160k".into()]);
            } else {
                args.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "192k".into()]);
            }
        }
        if format != "webm" {
            args.extend(["-movflags".into(), "+faststart".into()]);
        }
        args.extend(["-f".into(), format.into(), partial.display().to_string()]);

        let mut encoder = command(&context.tools.ffmpeg).args(&args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn()?;
        let mut stdin = encoder.stdin.take().expect("stdin");
        let mut stderr = encoder.stderr.take().expect("stderr");
        let stderr_thread = std::thread::spawn(move || {
            let mut buffer = Vec::new();
            let _ = std::io::Read::read_to_end(&mut stderr, &mut buffer);
            buffer
        });
        let (sender, receiver) = sync_channel::<Vec<u8>>(3);
        let writer = std::thread::spawn(move || -> std::io::Result<()> {
            for frame in receiver {
                stdin.write_all(&frame)?;
            }
            stdin.flush()
        });

        let mut decoders: HashMap<(usize, usize), DecoderSlot> = HashMap::new();
        let started = Instant::now();
        let mut last_report = Instant::now();
        let mut canvas = Pixmap::new(out_w, out_h).ok_or_else(|| AppError::msg("Invalid output size"))?;
        let render_result = (|| -> AppResult<()> {
            for frame in 0..total_frames {
                if cancelled() {
                    return Err(AppError::Cancelled);
                }
                let time = frame as f64 / fps as f64;
                let layers: Vec<Layer> = resolve_scene(&project, &connections, time, &rasters);
                fill_background(&mut canvas, &project.background);
                for layer in &layers {
                    let item = &project.tracks[layer.item_index.0].items[layer.item_index.1];
                    match item.kind.as_str() {
                        "video" => {
                            let Some(media_time) = layer.media_time else { continue };
                            let slot = match decoders.entry(layer.item_index) {
                                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                                std::collections::hash_map::Entry::Vacant(entry) => entry.insert(spawn_decoder(context.tools, &project, item, pw, ph, k, fps)?),
                            };
                            if let Some(pixmap) = slot.decoder.frame_at(media_time)? {
                                draw_layer(&mut canvas, layer, Content::Pixmap(&pixmap), k, frame as i64);
                            }
                        }
                        "image" => {
                            if let Some(pixmap) = images.get(&item.id) {
                                draw_layer(&mut canvas, layer, Content::Pixmap(pixmap), k, frame as i64);
                            }
                        }
                        "text" => {
                            if let Some(pixmap) = rasters.pixmaps.get(&item.id) {
                                draw_layer(&mut canvas, layer, Content::Pixmap(pixmap), k, frame as i64);
                            }
                        }
                        "shape" => {
                            let shape = item.shape.clone().unwrap_or_default();
                            if shape.kind == "blur" {
                                draw_layer(&mut canvas, layer, Content::BlurRegion { strength: shape.blur.max(1.0) }, k, frame as i64);
                            } else {
                                draw_layer(&mut canvas, layer, Content::Shape { kind: &shape.kind, color: &shape.color, radius: item.radius }, k, frame as i64);
                            }
                        }
                        _ => {}
                    }
                }
                decoders.retain(|_, slot| slot.end > time);
                prefetch_decoders(context.tools, &project, &mut decoders, time, pw, ph, k, fps);
                if sender.send(canvas.data().to_vec()).is_err() {
                    return Err(AppError::msg("The encoder stopped unexpectedly"));
                }
                if last_report.elapsed().as_millis() > 200 || frame + 1 == total_frames {
                    last_report = Instant::now();
                    let elapsed = started.elapsed().as_secs_f64().max(0.001);
                    let rate = (frame + 1) as f64 / elapsed;
                    let remaining = (total_frames - frame - 1) as f64 / rate.max(0.001);
                    report(0.06 + 0.92 * (frame + 1) as f64 / total_frames as f64, "Rendering video", frame + 1, rate / fps as f64, Some(remaining));
                }
            }
            Ok(())
        })();
        drop(sender);
        drop(decoders);
        let write_result = writer.join().unwrap_or_else(|_| Err(std::io::Error::other("writer panicked")));
        if render_result.is_err() || write_result.is_err() {
            let _ = encoder.kill();
        }
        let status = encoder.wait()?;
        let stderr = stderr_thread.join().unwrap_or_default();
        render_result?;
        if !status.success() || write_result.is_err() {
            let _ = std::fs::remove_file(&partial);
            return Err(AppError::msg(format!("Encoding failed: {}", stderr_tail(&stderr))));
        }
        report(0.99, "Finishing file", total_frames, 0.0, None);
        if output_path.exists() {
            std::fs::remove_file(&output_path)?;
        }
        std::fs::rename(&partial, &output_path)?;
        Ok(output_path.clone())
    })();

    let _ = std::fs::remove_dir_all(&job_dir);
    if result.is_err() {
        let partial = output_path.with_file_name(format!(".{}.partial", output_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
        let _ = std::fs::remove_file(partial);
    }
    result
}

fn spawn_decoder(tools: &Tools, project: &Project, item: &TimelineItem, pw: f64, ph: f64, k: f64, fps: u32) -> AppResult<DecoderSlot> {
    let asset = project.asset(item.asset_id.as_deref()).ok_or_else(|| AppError::msg(format!("Clip {} has no media", item.name)))?;
    let (Some(sw), Some(sh)) = (asset.width, asset.height) else {
        return Err(AppError::msg(format!("Unknown size of {}", asset.name)));
    };
    let layout = media_layout(sw, sh, item.crop.as_ref(), item.fit.as_deref().unwrap_or("contain"), item.focus.as_ref(), pw, ph);
    let factor = decode_factor(item);
    let geometry = Geometry::new(&layout, layout.box_w * k * factor, layout.box_h * k * factor);
    let decoder = VideoDecoder::spawn(tools, Path::new(&asset.path), geometry, item.source_in, item.duration * item.speed(), item.speed(), fps)?;
    Ok(DecoderSlot { decoder, end: item.end() + 1.6 })
}

/// Starts decoders ~1 s before their clip so FFmpeg start-up never stalls the render.
#[allow(clippy::too_many_arguments)]
fn prefetch_decoders(tools: &Tools, project: &Project, decoders: &mut HashMap<(usize, usize), DecoderSlot>, time: f64, pw: f64, ph: f64, k: f64, fps: u32) {
    for (track_index, track) in project.tracks.iter().enumerate() {
        if track.hidden || track.kind != "video" {
            continue;
        }
        for (item_index, item) in track.items.iter().enumerate() {
            if item.kind != "video" || item.hidden || decoders.contains_key(&(track_index, item_index)) {
                continue;
            }
            let lead = item.start - 1.6 - time;
            if lead > 0.0 && lead < 1.0 {
                if let Ok(slot) = spawn_decoder(tools, project, item, pw, ph, k, fps) {
                    decoders.insert((track_index, item_index), slot);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tools() -> Option<Tools> {
        crate::ffmpeg::locate("")
    }

    fn generate(tools: &Tools, args: &[&str], output: &Path) {
        let status = command(&tools.ffmpeg).args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).arg(output).status().unwrap();
        assert!(status.success());
    }

    #[test]
    fn exports_a_layered_project() {
        let Some(tools) = tools() else { return };
        let dir = std::env::temp_dir().join(format!("viralflow-export-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let video_a = dir.join("a.mp4");
        let video_b = dir.join("b.mp4");
        let image = dir.join("c.png");
        let tone = dir.join("tone.wav");
        generate(
            &tools,
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=s=1280x720:r=30:d=4",
                "-f",
                "lavfi",
                "-i",
                "sine=f=440:d=4",
                "-shortest",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
            ],
            &video_a,
        );
        generate(&tools, &["-f", "lavfi", "-i", "mandelbrot=s=720x1280:r=30", "-t", "3", "-c:v", "libx264", "-pix_fmt", "yuv420p"], &video_b);
        generate(&tools, &["-f", "lavfi", "-i", "smptebars=s=640x480", "-frames:v", "1"], &image);
        generate(&tools, &["-f", "lavfi", "-i", "sine=f=660:d=2"], &tone);
        let p = |path: &Path| path.display().to_string();
        let project: Project = serde_json::from_value(json!({
            "id": "test", "name": "Test", "width": 1080, "height": 1920, "fps": 30, "background": "#101010",
            "assets": [
                { "id": "a", "kind": "video", "name": "a", "path": p(&video_a), "duration": 4.0, "width": 1280.0, "height": 720.0, "hasAudio": true },
                { "id": "b", "kind": "video", "name": "b", "path": p(&video_b), "duration": 3.0, "width": 720.0, "height": 1280.0, "hasAudio": false },
                { "id": "c", "kind": "image", "name": "c", "path": p(&image), "width": 640.0, "height": 480.0 },
                { "id": "t", "kind": "audio", "name": "t", "path": p(&tone), "duration": 2.0 }
            ],
            "tracks": [
                { "id": "fx", "kind": "video", "items": [
                    { "id": "shape", "kind": "shape", "start": 0.5, "duration": 2.0, "radius": 30, "shape": { "kind": "rect", "width": 50, "height": 10, "color": "#ff3366" }, "animIn": { "kind": "pop", "duration": 0.3 } },
                    { "id": "blur", "kind": "shape", "start": 3.0, "duration": 2.0, "radius": 20, "shape": { "kind": "blur", "width": 60, "height": 20, "blur": 20 } }
                ]},
                { "id": "over", "kind": "video", "items": [
                    { "id": "img", "kind": "image", "assetId": "c", "start": 1.0, "duration": 3.0, "fit": "contain", "radius": 40,
                      "transform": { "x": 50, "y": 30, "scale": 0.6, "rotation": 8, "opacity": 1 },
                      "keyframes": [ { "id": "k1", "time": 0, "transform": { "x": 30, "y": 30, "scale": 0.5, "rotation": 0, "opacity": 1 }, "easing": "linear" },
                                     { "id": "k2", "time": 3, "transform": { "x": 70, "y": 40, "scale": 0.8, "rotation": 15, "opacity": 0.8 }, "easing": "ease-in-out" } ],
                      "adjust": { "saturation": 0.5, "vignette": 0.6 }, "effect": "vhs" }
                ]},
                { "id": "main", "kind": "video", "items": [
                    { "id": "va", "kind": "video", "assetId": "a", "start": 0, "duration": 3.0, "sourceIn": 0.5, "fit": "cover", "volume": 0.8, "adjust": { "temperature": 0.45, "saturation": 0.1 } },
                    { "id": "vb", "kind": "video", "assetId": "b", "start": 3.0, "duration": 2.5, "speed": 1.5, "fit": "cover", "transition": { "kind": "slide-left", "duration": 0.6 }, "effect": "glitch" }
                ]},
                { "id": "music", "kind": "audio", "items": [
                    { "id": "tone", "kind": "audio", "assetId": "t", "start": 1.0, "duration": 2.0, "volume": 0.5, "fadeIn": 0.5, "fadeOut": 0.5 }
                ]}
            ],
            "export": { "format": "mp4", "codec": "h264", "quality": "draft", "resolution": 720, "fps": 30 }
        }))
        .unwrap();
        let output = dir.join("out.mp4");
        let request = ExportRequest { project, output_path: p(&output), rasters: vec![] };
        let started = Instant::now();
        let progress = |_event: ExportProgress| {};
        let result =
            export(Context { tools: &tools, job_id: "test-job", jobs_dir: &dir, cancel: Arc::new(AtomicBool::new(false)), progress: &progress }, request)
                .expect("export");
        let elapsed = started.elapsed();
        assert_eq!(result, output);
        let probe = crate::media::probe(&tools, &output).unwrap();
        println!("rendered 5.5 s in {elapsed:?}: {probe:?}");
        assert_eq!((probe.width, probe.height), (Some(720), Some(1280)));
        assert!(probe.has_audio);
        assert!((probe.duration.unwrap() - 5.5).abs() < 0.15, "duration {:?}", probe.duration);
        // Keep a still for manual inspection.
        let still = dir.join("still.png");
        generate(&tools, &["-ss", "3.0", "-i", &p(&output), "-frames:v", "1"], &still);
        println!("inspect: {}", still.display());
    }
}
