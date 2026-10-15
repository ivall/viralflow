use std::path::{Path, PathBuf};

use crate::engine::asset_has_audio;
use crate::error::AppResult;
use crate::ffmpeg::Tools;
use crate::media::run_with_progress;
use crate::model::Project;

struct Voice {
    path: String,
    source_in: f64,
    speed: f64,
    duration: f64,
    start: f64,
    volume: f64,
    fade_in: f64,
    fade_out: f64,
    enhance: f64,
}

/// Voice cleanup for `amount` in 0..1: rumble cut, noise reduction, de-essing and
/// gentle compression. Empty when off. The preview approximates the cut and the
/// compressor (media-pool.ts); noise reduction is only heard in the export.
pub fn enhance_chain(amount: f64) -> String {
    if amount <= 0.0 {
        return String::new();
    }
    let a = amount.min(1.0);
    format!(
        "highpass=f=80,afftdn=nf={:.1}:nr={:.1},deesser=i={:.2},acompressor=threshold={:.3}:ratio={:.2}:attack=15:release=120,alimiter=limit=0.95:level=false",
        -25.0 - a * 20.0,
        6.0 + a * 18.0,
        a * 0.5,
        enhance_threshold(a),
        enhance_ratio(a)
    )
}

/// Compressor threshold (linear) and ratio of the voice cleanup; the preview uses the same curve.
pub fn enhance_threshold(amount: f64) -> f64 {
    0.5 - amount * 0.3
}

pub fn enhance_ratio(amount: f64) -> f64 {
    2.0 + amount * 2.0
}

/// Single-pass loudness normalisation of the final mix to `target` LUFS.
pub fn loudness_chain(target: Option<f64>) -> String {
    match target {
        Some(value) if value.is_finite() && value < 0.0 => format!("loudnorm=I={:.1}:TP=-1.5:LRA=11,aresample=48000,", value.clamp(-30.0, -10.0)),
        _ => String::new(),
    }
}

/// `atempo` only accepts 0.5..=2 per instance, so larger changes are chained.
pub fn atempo_chain(speed: f64) -> String {
    let mut rate = speed.clamp(0.1, 100.0);
    let mut parts = Vec::new();
    while rate < 0.5 - 1e-9 {
        parts.push("atempo=0.5".to_string());
        rate /= 0.5;
    }
    while rate > 2.0 + 1e-9 {
        parts.push("atempo=2.0".to_string());
        rate /= 2.0;
    }
    if (rate - 1.0).abs() > 1e-6 {
        parts.push(format!("atempo={rate:.6}"));
    }
    parts.join(",")
}

fn voices(project: &Project, total: f64) -> Vec<Voice> {
    let mut result = Vec::new();
    for track in &project.tracks {
        if track.muted {
            continue;
        }
        for item in &track.items {
            if item.hidden || item.muted || item.volume() <= 0.0 || item.start >= total {
                continue;
            }
            let asset = project.asset(item.asset_id.as_deref());
            let audible = match item.kind.as_str() {
                "audio" => true,
                "video" => asset_has_audio(asset),
                _ => false,
            };
            let Some(asset) = asset.filter(|_| audible) else { continue };
            if !Path::new(&asset.path).is_file() {
                continue;
            }
            let duration = item.duration.min(total - item.start);
            if duration <= 0.01 {
                continue;
            }
            result.push(Voice {
                path: asset.path.clone(),
                source_in: item.source_in.max(0.0),
                speed: item.speed(),
                duration,
                start: item.start.max(0.0),
                volume: item.volume(),
                fade_in: item.fade_in.max(0.0).min(duration),
                fade_out: item.fade_out.max(0.0).min(duration),
                enhance: item.voice_enhance.clamp(0.0, 1.0),
            });
        }
    }
    result
}

/// Renders the whole soundtrack to a WAV file. Returns None when the project is silent.
pub fn render_mix(
    tools: &Tools,
    project: &Project,
    total: f64,
    job_dir: &Path,
    on_progress: impl FnMut(f64),
    cancelled: &dyn Fn() -> bool,
) -> AppResult<Option<PathBuf>> {
    let voices = voices(project, total);
    if voices.is_empty() {
        return Ok(None);
    }
    let mut args: Vec<String> = Vec::new();
    let mut graph: Vec<String> = Vec::new();
    for (index, voice) in voices.iter().enumerate() {
        args.extend([
            "-ss".into(),
            format!("{:.4}", voice.source_in),
            "-t".into(),
            format!("{:.4}", voice.duration * voice.speed + 0.25),
            "-i".into(),
            voice.path.clone(),
        ]);
        let mut chain = vec!["aresample=48000".to_string(), "aformat=sample_fmts=fltp:channel_layouts=stereo".to_string()];
        let tempo = atempo_chain(voice.speed);
        if !tempo.is_empty() {
            chain.push(tempo);
        }
        chain.push(format!("atrim=duration={:.4}", voice.duration));
        chain.push("asetpts=PTS-STARTPTS".into());
        let enhance = enhance_chain(voice.enhance);
        if !enhance.is_empty() {
            chain.push(enhance);
        }
        chain.push(format!("volume={:.4}", voice.volume));
        if voice.fade_in > 0.0 {
            chain.push(format!("afade=t=in:st=0:d={:.4}", voice.fade_in));
        }
        if voice.fade_out > 0.0 {
            chain.push(format!("afade=t=out:st={:.4}:d={:.4}", (voice.duration - voice.fade_out).max(0.0), voice.fade_out));
        }
        let delay = (voice.start * 1000.0).round() as i64;
        if delay > 0 {
            chain.push(format!("adelay={delay}:all=1"));
        }
        graph.push(format!("[{index}:a]{}[a{index}]", chain.join(",")));
    }
    let labels: String = (0..voices.len()).map(|index| format!("[a{index}]")).collect();
    graph.push(format!(
        "{labels}amix=inputs={}:normalize=0:duration=longest:dropout_transition=0,{}alimiter=limit=0.98:level=false,apad=whole_dur={total:.4},atrim=duration={total:.4}[aout]",
        voices.len(),
        loudness_chain(project.export.loudness)
    ));
    let graph = graph.join(";");
    let script = job_dir.join("audio.graph");
    std::fs::write(&script, &graph)?;
    args.extend([
        "-filter_complex_script".into(),
        script.display().to_string(),
        "-map".into(),
        "[aout]".into(),
        "-c:a".into(),
        "pcm_s16le".into(),
        "-ar".into(),
        "48000".into(),
    ]);
    let output = job_dir.join("audio.wav");
    args.push(output.display().to_string());
    run_with_progress(tools, &args, total, on_progress, cancelled)?;
    Ok(Some(output))
}

#[cfg(test)]
mod tests {
    use super::{atempo_chain, enhance_chain, loudness_chain};

    #[test]
    fn tempo_chains_extreme_speeds() {
        assert_eq!(atempo_chain(1.0), "");
        assert_eq!(atempo_chain(4.0), "atempo=2.0,atempo=2.000000");
        assert_eq!(atempo_chain(0.25), "atempo=0.5,atempo=0.500000");
    }

    #[test]
    fn voice_cleanup_scales_with_amount() {
        assert_eq!(enhance_chain(0.0), "");
        let full = enhance_chain(1.0);
        assert!(full.starts_with("highpass=f=80,afftdn=nf=-45.0:nr=24.0,deesser=i=0.50,"), "{full}");
        assert!(full.contains("acompressor=threshold=0.200:ratio=4.00"), "{full}");
    }

    #[test]
    fn loudness_is_optional() {
        assert_eq!(loudness_chain(None), "");
        assert_eq!(loudness_chain(Some(0.0)), "");
        assert_eq!(loudness_chain(Some(-14.0)), "loudnorm=I=-14.0:TP=-1.5:LRA=11,aresample=48000,");
    }
}
