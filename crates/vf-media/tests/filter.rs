use vf_media::audio::SAMPLE_RATE;
use vf_media::filter::apply;

fn sine(seconds: f64, amplitude: f32) -> Vec<f32> {
    let frames = (seconds * SAMPLE_RATE as f64) as usize;
    (0..frames)
        .flat_map(|i| {
            let v = (i as f32 * 440.0 * std::f32::consts::TAU / SAMPLE_RATE as f32).sin() * amplitude;
            [v, v]
        })
        .collect()
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |a, b| a.max(b.abs()))
}

#[test]
fn volume_and_tempo_run_in_process() {
    let input = sine(1.0, 0.8);
    let quieter = apply("volume=0.5", &input).unwrap();
    assert_eq!(quieter.len(), input.len());
    assert!((peak(&quieter) - 0.4).abs() < 0.01, "{}", peak(&quieter));
    let faster = apply("atempo=2.0", &input).unwrap();
    let ratio = faster.len() as f64 / input.len() as f64;
    assert!((ratio - 0.5).abs() < 0.03, "{ratio}");
}

#[test]
fn loudness_normalisation_raises_quiet_audio() {
    let input = sine(3.0, 0.02);
    let out = apply("loudnorm=I=-14:TP=-1.5:LRA=11,aresample=48000", &input).unwrap();
    assert!(out.len() as f64 > input.len() as f64 * 0.95, "{} vs {}", out.len(), input.len());
    assert!(peak(&out) > 0.1, "{}", peak(&out));
}
