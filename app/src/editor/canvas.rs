//! Direct manipulation on the preview: picking layers, moving with snapping,
//! scaling from the corners and rotating with the knob.

use vf_core::edit::TransformPatch;
use vf_core::engine::scene::{layer_quad, Layer};

pub type Point = (f64, f64);

/// Whether `point` lies inside the convex quad (any winding).
pub fn inside(point: Point, quad: &[Point; 4]) -> bool {
    let mut sign = 0.0f64;
    for index in 0..4 {
        let a = quad[index];
        let b = quad[(index + 1) % 4];
        let cross = (b.0 - a.0) * (point.1 - a.1) - (b.1 - a.1) * (point.0 - a.0);
        if cross.abs() < 1e-9 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

/// Top-most layer under `point`, preferring the primary selection.
pub fn hit_test<'a>(layers: &'a [(String, Layer)], point: Point, primary: Option<&str>) -> Option<&'a str> {
    let hits = hits(layers, point);
    hits.iter().copied().find(|id| Some(*id) == primary).or(hits.first().copied())
}

/// Every layer under `point`, top-most first.
pub fn hits(layers: &[(String, Layer)], point: Point) -> Vec<&str> {
    layers.iter().rev().filter(|(_, layer)| inside(point, &layer_quad(layer))).map(|(id, _)| id.as_str()).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Move,
    Scale,
    Rotate,
}

#[derive(Debug, Clone)]
struct Start {
    id: String,
    x: f64,
    y: f64,
    scale: f64,
    rotation: f64,
    half_w: f64,
    half_h: f64,
}

/// A drag in progress, in project pixels.
#[derive(Debug, Clone)]
pub struct Gesture {
    pub mode: Mode,
    start: Point,
    center: Point,
    starts: Vec<Start>,
    /// The first real movement starts an undo step.
    pub changed: bool,
}

/// Snapping guides to draw (project pixels).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Guides {
    pub x: Option<f64>,
    pub y: Option<f64>,
}

impl Gesture {
    /// `targets`: the unlocked selected layers (id, layer) at the playhead.
    pub fn begin(mode: Mode, start: Point, targets: &[(String, Layer)]) -> Option<Self> {
        if targets.is_empty() {
            return None;
        }
        let count = targets.len() as f64;
        let center = (targets.iter().map(|(_, l)| l.cx).sum::<f64>() / count, targets.iter().map(|(_, l)| l.cy).sum::<f64>() / count);
        let starts = targets
            .iter()
            .map(|(id, l)| Start {
                id: id.clone(),
                x: l.cx,
                y: l.cy,
                scale: l.scale,
                rotation: l.rotation,
                half_w: l.box_w * l.scale / 2.0,
                half_h: l.box_h * l.scale / 2.0,
            })
            .collect();
        Some(Gesture { mode, start, center, starts, changed: false })
    }

    /// Transform changes for the pointer at `point`. `pixel` = project pixels
    /// per screen pixel; `snap` enables snapping, `fine` (Shift) rotates in 15° steps.
    pub fn update(&mut self, point: Point, canvas: (f64, f64), pixel: f64, snap: bool, fine: bool) -> Option<(Vec<(String, TransformPatch)>, Guides)> {
        let distance = ((point.0 - self.start.0).powi(2) + (point.1 - self.start.1).powi(2)).sqrt() / pixel.max(1e-9);
        if !self.changed && distance < 2.0 {
            return None;
        }
        self.changed = true;
        let mut guides = Guides::default();
        let patches = match self.mode {
            Mode::Move => {
                let mut dx = point.0 - self.start.0;
                let mut dy = point.1 - self.start.1;
                if self.starts.len() == 1 && snap {
                    let s = &self.starts[0];
                    let range = 6.0 * pixel;
                    let cx = s.x + dx;
                    let cy = s.y + dy;
                    for (value, target) in [(cx, canvas.0 / 2.0), (cx - s.half_w, 0.0), (cx + s.half_w, canvas.0)] {
                        if (value - target).abs() < range {
                            dx += target - value;
                            guides.x = Some(target);
                            break;
                        }
                    }
                    for (value, target) in [(cy, canvas.1 / 2.0), (cy - s.half_h, 0.0), (cy + s.half_h, canvas.1)] {
                        if (value - target).abs() < range {
                            dy += target - value;
                            guides.y = Some(target);
                            break;
                        }
                    }
                }
                self.starts
                    .iter()
                    .map(|s| {
                        (s.id.clone(), TransformPatch { x: Some((s.x + dx) / canvas.0 * 100.0), y: Some((s.y + dy) / canvas.1 * 100.0), ..Default::default() })
                    })
                    .collect()
            }
            Mode::Scale => {
                let start_distance = ((self.start.0 - self.center.0).powi(2) + (self.start.1 - self.center.1).powi(2)).sqrt().max(1.0);
                let factor = ((point.0 - self.center.0).powi(2) + (point.1 - self.center.1).powi(2)).sqrt() / start_distance;
                self.starts
                    .iter()
                    .map(|s| {
                        (s.id.clone(), TransformPatch { scale: Some(((s.scale * factor * 1000.0).round() / 1000.0).clamp(0.05, 10.0)), ..Default::default() })
                    })
                    .collect()
            }
            Mode::Rotate => {
                let start_angle = (self.start.1 - self.center.1).atan2(self.start.0 - self.center.0);
                let angle = (point.1 - self.center.1).atan2(point.0 - self.center.0);
                self.starts
                    .iter()
                    .map(|s| {
                        let mut rotation = s.rotation + (angle - start_angle).to_degrees();
                        if fine {
                            rotation = (rotation / 15.0).round() * 15.0;
                        } else if rotation.rem_euclid(90.0) < 2.0 || rotation.rem_euclid(90.0) > 88.0 {
                            rotation = (rotation / 90.0).round() * 90.0;
                        }
                        (s.id.clone(), TransformPatch { rotation: Some((rotation * 10.0).round() / 10.0), ..Default::default() })
                    })
                    .collect()
            }
        };
        Some((patches, guides))
    }
}

/// Position of the rotation knob of a quad (22 px above the top edge, in screen pixels).
pub fn knob(quad: &[Point; 4], rotation: f64, pixel: f64) -> Point {
    let (sin, cos) = rotation.to_radians().sin_cos();
    ((quad[0].0 + quad[1].0) / 2.0 + sin * 22.0 * pixel, (quad[0].1 + quad[1].1) / 2.0 - cos * 22.0 * pixel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vf_core::model::{BlendMode, ItemKind};

    fn layer(cx: f64, cy: f64) -> Layer {
        Layer {
            track: 0,
            index: 0,
            kind: ItemKind::Shape,
            box_w: 100.0,
            box_h: 50.0,
            source: None,
            media_time: None,
            cx,
            cy,
            scale: 1.0,
            rotation: 0.0,
            flip_x: false,
            flip_y: false,
            opacity: 1.0,
            blur: 0.0,
            blend: BlendMode::Normal,
            radius: 0.0,
            matrix: None,
            vignette: 0.0,
            clip_left: 0.0,
            clip_right: 1.0,
            grain: 0.0,
            scanlines: false,
            glitch: vec![],
        }
    }

    #[test]
    fn picks_the_top_layer_but_prefers_the_selection() {
        let layers = vec![("bottom".to_string(), layer(100.0, 100.0)), ("top".to_string(), layer(110.0, 100.0))];
        assert_eq!(hit_test(&layers, (105.0, 100.0), None), Some("top"));
        assert_eq!(hit_test(&layers, (105.0, 100.0), Some("bottom")), Some("bottom"));
        assert_eq!(hit_test(&layers, (500.0, 500.0), None), None);
    }

    #[test]
    fn moving_snaps_to_the_centre() {
        let targets = vec![("a".to_string(), layer(100.0, 100.0))];
        let mut gesture = Gesture::begin(Mode::Move, (100.0, 100.0), &targets).unwrap();
        // Dragging the centre to 538 on a 1080 wide canvas snaps to 540.
        let (patches, guides) = gesture.update((538.0, 300.0), (1080.0, 1920.0), 1.0, true, false).unwrap();
        assert_eq!(guides.x, Some(540.0));
        assert!((patches[0].1.x.unwrap() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn small_jitter_does_not_start_a_drag() {
        let targets = vec![("a".to_string(), layer(100.0, 100.0))];
        let mut gesture = Gesture::begin(Mode::Move, (100.0, 100.0), &targets).unwrap();
        assert!(gesture.update((101.0, 100.0), (1080.0, 1920.0), 1.0, true, false).is_none());
    }

    #[test]
    fn scaling_and_rotation() {
        let targets = vec![("a".to_string(), layer(0.0, 0.0))];
        let mut gesture = Gesture::begin(Mode::Scale, (10.0, 0.0), &targets).unwrap();
        let (patches, _) = gesture.update((20.0, 0.0), (1080.0, 1920.0), 1.0, true, false).unwrap();
        assert_eq!(patches[0].1.scale, Some(2.0));
        let mut gesture = Gesture::begin(Mode::Rotate, (10.0, 0.0), &targets).unwrap();
        let (patches, _) = gesture.update((0.0, 10.0), (1080.0, 1920.0), 1.0, true, false).unwrap();
        assert_eq!(patches[0].1.rotation, Some(90.0));
    }
}
