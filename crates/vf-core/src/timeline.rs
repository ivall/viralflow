//! Where clips may sit on a track: clips never overlap, placement looks for the
//! nearest free spot, trims stop at neighbours.

use std::collections::{HashMap, HashSet};

pub const MIN_CLIP_DURATION: f64 = 0.08;

/// Upper bound used by placement; the timeline itself is open-ended.
pub const HORIZON: f64 = 24.0 * 3600.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub id: String,
    pub start: f64,
    pub duration: f64,
}

impl Span {
    pub fn new(id: impl Into<String>, start: f64, duration: f64) -> Self {
        Span { id: id.into(), start, duration }
    }

    pub fn end(&self) -> f64 {
        self.start + self.duration
    }
}

pub fn spans_of(items: &[crate::model::TimelineItem]) -> Vec<Span> {
    items.iter().map(|item| Span::new(item.id.clone(), item.start, item.duration)).collect()
}

fn sorted_obstacles(items: &[Span], ignored: &HashSet<&str>) -> Vec<Span> {
    let mut list: Vec<Span> = items
        .iter()
        .filter(|item| !ignored.contains(item.id.as_str()))
        .map(|item| Span { id: item.id.clone(), start: item.start.max(0.0), duration: item.duration.max(MIN_CLIP_DURATION) })
        .collect();
    list.sort_by(|a, b| a.start.total_cmp(&b.start).then_with(|| a.id.cmp(&b.id)));
    list
}

fn open_gaps(items: &[Span], duration: f64, ignored: &HashSet<&str>) -> Vec<(f64, f64)> {
    let mut gaps = Vec::new();
    let mut cursor: f64 = 0.0;
    for item in sorted_obstacles(items, ignored) {
        if item.start > cursor {
            gaps.push((cursor, duration.min(item.start)));
        }
        cursor = cursor.max(item.end());
        if cursor >= duration {
            break;
        }
    }
    if cursor < duration {
        gaps.push((cursor, duration));
    }
    gaps
}

fn nearest_start(gaps: &[(f64, f64)], desired: f64, width: f64) -> Option<f64> {
    let mut best: Option<f64> = None;
    let mut best_distance = f64::INFINITY;
    for &(gap_start, gap_end) in gaps {
        if gap_end - gap_start + 1e-6 < width {
            continue;
        }
        let start = (gap_end - width).min(gap_start.max(desired));
        let distance = (start - desired).abs();
        if distance < best_distance - 1e-6 || ((distance - best_distance).abs() <= 1e-6 && best.is_none_or(|b| start < b)) {
            best = Some(start);
            best_distance = distance;
        }
    }
    best
}

/// The closest legal position for `candidate` without changing its length.
pub fn place_span(items: &[Span], candidate: &Span, horizon: f64, ignore: &[&str]) -> Option<Span> {
    let duration = candidate.duration.max(MIN_CLIP_DURATION).min(horizon.max(MIN_CLIP_DURATION));
    let mut ignored: HashSet<&str> = ignore.iter().copied().collect();
    ignored.insert(candidate.id.as_str());
    let start = nearest_start(&open_gaps(items, horizon, &ignored), candidate.start.max(0.0), duration)?;
    Some(Span { id: candidate.id.clone(), start, duration })
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectionSpan {
    pub id: String,
    pub track_id: String,
    pub start: f64,
    pub duration: f64,
}

/// Moves a multi-track selection by one shared delta to the nearest legal spot.
/// Returns the new starts, or None when there is no room.
pub fn place_selection(tracks: &[(String, Vec<Span>)], candidates: &[SelectionSpan], horizon: f64) -> Option<Vec<(String, f64)>> {
    if candidates.is_empty() {
        return Some(vec![]);
    }
    let selected: HashSet<&str> = candidates.iter().map(|c| c.id.as_str()).collect();
    let obstacles: HashMap<&str, Vec<Span>> = tracks.iter().map(|(id, items)| (id.as_str(), sorted_obstacles(items, &selected))).collect();
    let end = |c: &SelectionSpan| c.start + c.duration;
    let lower = -candidates.iter().map(|c| c.start).fold(f64::INFINITY, f64::min);
    let upper = horizon - candidates.iter().map(end).fold(f64::NEG_INFINITY, f64::max);
    if lower > upper + 1e-6 {
        return None;
    }
    let empty = Vec::new();
    let mut forbidden = Vec::new();
    for candidate in candidates {
        for obstacle in obstacles.get(candidate.track_id.as_str()).unwrap_or(&empty) {
            let start = obstacle.start - end(candidate);
            let stop = obstacle.end() - candidate.start;
            if stop > lower + 1e-6 && start < upper - 1e-6 {
                forbidden.push((start, stop));
            }
        }
    }
    let valid = |delta: f64| {
        if delta < lower - 1e-6 || delta > upper + 1e-6 {
            return false;
        }
        candidates.iter().all(|candidate| {
            let start = candidate.start + delta;
            let stop = end(candidate) + delta;
            obstacles.get(candidate.track_id.as_str()).unwrap_or(&empty).iter().all(|obstacle| start >= obstacle.end() - 1e-6 || stop <= obstacle.start + 1e-6)
        })
    };
    let fit = |value: f64| value.max(lower).min(upper);
    let mut points = vec![fit(0.0), lower, upper];
    for (start, stop) in forbidden {
        points.push(fit(start));
        points.push(fit(stop));
    }
    let mut rounded: Vec<f64> = points.into_iter().map(|v| (v * 1e6).round() / 1e6).collect();
    rounded.sort_by(|a, b| a.abs().total_cmp(&b.abs()).then_with(|| a.total_cmp(b)));
    rounded.dedup();
    let delta = rounded.into_iter().find(|delta| valid(*delta))?;
    Some(candidates.iter().map(|c| (c.id.clone(), c.start + delta)).collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Start,
    End,
}

/// Constrains one trim edge against its immediate neighbour.
pub fn trim_span(items: &[Span], current: &Span, candidate: &Span, edge: Edge, horizon: f64) -> Span {
    let ignored: HashSet<&str> = [current.id.as_str()].into_iter().collect();
    let obstacles = sorted_obstacles(items, &ignored);
    match edge {
        Edge::Start => {
            let fixed_end = horizon.min(candidate.end());
            let previous_end = obstacles.iter().filter(|item| item.start < current.start + 1e-6).fold(0.0f64, |value, item| value.max(item.end()));
            let start = (fixed_end - MIN_CLIP_DURATION).min(previous_end.max(candidate.start));
            Span { id: candidate.id.clone(), start, duration: (fixed_end - start).max(MIN_CLIP_DURATION) }
        }
        Edge::End => {
            let next_start = obstacles.iter().filter(|item| item.start >= current.end() - 1e-6).fold(horizon, |value, item| value.min(item.start));
            let duration = candidate.duration.min(next_start - current.start).max(MIN_CLIP_DURATION);
            Span { id: candidate.id.clone(), start: current.start, duration }
        }
    }
}

pub struct RippleResize {
    pub duration: f64,
    pub starts: HashMap<String, f64>,
    pub end: f64,
}

/// Changes a clip length and pushes only the neighbours that would overlap.
pub fn ripple_resize(items: &[Span], item_id: &str, next_duration: f64) -> RippleResize {
    let duration = next_duration.max(MIN_CLIP_DURATION);
    let Some(current) = items.iter().find(|item| item.id == item_id) else {
        return RippleResize { duration, starts: HashMap::new(), end: 0.0 };
    };
    let mut starts = HashMap::new();
    let mut cursor = current.start + duration;
    let mut following: Vec<&Span> = items.iter().filter(|item| item.id != item_id && item.start >= current.start - 1e-6).collect();
    following.sort_by(|a, b| a.start.total_cmp(&b.start).then_with(|| a.id.cmp(&b.id)));
    for item in following {
        let start = if item.start < cursor { cursor } else { item.start };
        if (start - item.start).abs() > 1e-6 {
            starts.insert(item.id.clone(), start);
        }
        cursor = start + item.duration.max(MIN_CLIP_DURATION);
    }
    RippleResize { duration, starts, end: cursor }
}

/// Repairs imported tracks: keeps the order and shifts collisions to the right.
pub fn repair_layout(items: &[Span]) -> HashMap<String, f64> {
    let mut sorted: Vec<&Span> = items.iter().collect();
    sorted.sort_by(|a, b| a.start.total_cmp(&b.start).then_with(|| a.id.cmp(&b.id)));
    let mut starts = HashMap::new();
    let mut cursor: f64 = 0.0;
    for item in sorted {
        let start = item.start.max(0.0).max(cursor);
        starts.insert(item.id.clone(), start);
        cursor = start + item.duration.max(MIN_CLIP_DURATION);
    }
    starts
}

pub fn has_overlap(items: &[Span]) -> bool {
    let mut sorted: Vec<&Span> = items.iter().collect();
    sorted.sort_by(|a, b| a.start.total_cmp(&b.start));
    sorted.windows(2).any(|pair| pair[1].start < pair[0].end() - 1e-6)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(list: &[(&str, f64, f64)]) -> Vec<Span> {
        list.iter().map(|(id, start, duration)| Span::new(*id, *start, *duration)).collect()
    }

    #[test]
    fn place_finds_nearest_gap() {
        let items = spans(&[("a", 0.0, 2.0), ("b", 3.0, 2.0)]);
        let placed = place_span(&items, &Span::new("x", 1.0, 1.0), HORIZON, &[]).unwrap();
        assert!((placed.start - 2.0).abs() < 1e-9);
        let placed = place_span(&items, &Span::new("x", 1.0, 3.0), HORIZON, &[]).unwrap();
        assert!((placed.start - 5.0).abs() < 1e-9);
    }

    #[test]
    fn selection_moves_together() {
        let tracks = vec![("t1".to_string(), spans(&[("a", 0.0, 1.0), ("c", 4.0, 1.0)])), ("t2".to_string(), spans(&[("b", 0.0, 1.0)]))];
        let moves = vec![
            SelectionSpan { id: "a".into(), track_id: "t1".into(), start: 3.5, duration: 1.0 },
            SelectionSpan { id: "b".into(), track_id: "t2".into(), start: 3.5, duration: 1.0 },
        ];
        let placed = place_selection(&tracks, &moves, HORIZON).unwrap();
        // 3.5 would overlap "c" (4..5): the nearest legal delta puts both at 3.0.
        assert!(placed.iter().all(|(_, start)| (start - 3.0).abs() < 1e-6), "{placed:?}");
    }

    #[test]
    fn trim_stops_at_neighbours() {
        let items = spans(&[("a", 0.0, 2.0), ("b", 2.0, 2.0), ("c", 5.0, 1.0)]);
        let b = Span::new("b", 2.0, 2.0);
        let trimmed = trim_span(&items, &b, &Span::new("b", 1.0, 3.0), Edge::Start, HORIZON);
        assert!((trimmed.start - 2.0).abs() < 1e-9);
        let trimmed = trim_span(&items, &b, &Span::new("b", 2.0, 10.0), Edge::End, HORIZON);
        assert!((trimmed.duration - 3.0).abs() < 1e-9);
    }

    #[test]
    fn ripple_pushes_only_overlapping() {
        let items = spans(&[("a", 0.0, 2.0), ("b", 2.0, 1.0), ("c", 10.0, 1.0)]);
        let result = ripple_resize(&items, "a", 3.0);
        assert_eq!(result.starts.get("b"), Some(&3.0));
        assert!(!result.starts.contains_key("c"));
    }

    #[test]
    fn repair_removes_overlaps() {
        let items = spans(&[("a", 0.0, 2.0), ("b", 1.0, 2.0)]);
        let starts = repair_layout(&items);
        assert_eq!(starts["b"], 2.0);
    }
}
