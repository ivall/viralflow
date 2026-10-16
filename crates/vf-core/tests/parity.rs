//! The scene engine must keep producing the reference numbers recorded by the
//! original TypeScript preview (tests/fixtures/parity-expected.json), so the
//! port renders exactly what the previous app rendered.

use vf_core::engine::scene::{resolve_scene, scene_connections, TextMeasure};
use vf_core::model::{Project, TimelineItem};

struct FixedText;

impl TextMeasure for FixedText {
    fn text_box(&self, _item: &TimelineItem, _canvas_w: f64) -> (f64, f64) {
        (100.0, 50.0)
    }
}

#[test]
fn scene_matches_reference() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let project: Project = serde_json::from_slice(&std::fs::read(root.join("parity-project.json")).unwrap()).unwrap();
    let expected: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("parity-expected.json")).unwrap()).unwrap();
    let connections = scene_connections(&project);
    let close = |a: f64, b: &serde_json::Value, what: &str, time: f64| {
        let b = b.as_f64().unwrap_or(f64::NAN);
        assert!((a - b).abs() < 1e-6, "{what} at {time}: {a} vs reference {b}");
    };
    for sample in expected.as_array().unwrap() {
        let time = sample["time"].as_f64().unwrap();
        let layers = resolve_scene(&project, &connections, time, &FixedText);
        let reference = sample["layers"].as_array().unwrap();
        let ids: Vec<&str> = layers.iter().map(|layer| project.tracks[layer.track].items[layer.index].id.as_str()).collect();
        let reference_ids: Vec<&str> = reference.iter().map(|layer| layer["id"].as_str().unwrap()).collect();
        assert_eq!(ids, reference_ids, "layer order at {time}");
        for (layer, want) in layers.iter().zip(reference) {
            close(layer.box_w, &want["boxW"], "boxW", time);
            close(layer.box_h, &want["boxH"], "boxH", time);
            close(layer.cx, &want["cx"], "cx", time);
            close(layer.cy, &want["cy"], "cy", time);
            close(layer.scale, &want["scale"], "scale", time);
            close(layer.rotation, &want["rotation"], "rotation", time);
            close(layer.opacity, &want["opacity"], "opacity", time);
            close(layer.blur, &want["blur"], "blur", time);
            close(layer.clip_left, &want["clipLeft"], "clipLeft", time);
            close(layer.clip_right, &want["clipRight"], "clipRight", time);
            close(layer.vignette, &want["vignette"], "vignette", time);
            close(layer.grain, &want["grain"], "grain", time);
            match layer.media_time {
                Some(value) => close(value, &want["mediaTime"], "mediaTime", time),
                None => assert!(want["mediaTime"].is_null(), "mediaTime at {time}"),
            }
            let glitch: Vec<f64> = want["glitch"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
            assert_eq!(layer.glitch.len(), glitch.len(), "glitch bands at {time}");
            for (a, b) in layer.glitch.iter().zip(&glitch) {
                assert!((a - b).abs() < 1e-9, "glitch at {time}");
            }
            match (&layer.matrix, want["matrix"].as_array()) {
                (None, None) => {}
                (Some(matrix), Some(values)) => {
                    for (index, value) in values.iter().enumerate() {
                        close(matrix[index], value, &format!("matrix[{index}]"), time);
                    }
                }
                (ours, reference) => panic!("matrix presence differs at {time}: {:?} vs {:?}", ours.is_some(), reference.is_some()),
            }
        }
    }
}
