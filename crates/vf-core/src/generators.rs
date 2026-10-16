//! Auto-edit: a whole project built from a voiceover, a background video,
//! photos and captions. Every part stays an ordinary, editable clip.

use crate::engine::keyframes::normalize_transform;
use crate::engine::math::{clamp, uid};
use crate::model::*;
use crate::presets::{caption_style, create_project, track_label};

fn track(kind: TrackKind, name: &str, items: Vec<TimelineItem>) -> Track {
    let id = uid("track");
    Track {
        id: id.clone(),
        kind,
        name: name.into(),
        items: items.into_iter().map(|item| TimelineItem { track_id: id.clone(), ..item }).collect(),
        ..Default::default()
    }
}

fn caption_items(cues: &[CaptionCue], accent: &str) -> Vec<TimelineItem> {
    cues.iter()
        .enumerate()
        .map(|(index, cue)| TimelineItem {
            id: uid("caption"),
            kind: ItemKind::Text,
            name: cue.text.chars().take(40).collect(),
            start: cue.start,
            duration: cue.duration.max(0.1),
            group_id: Some("captions-generated".into()),
            transform: Some(normalize_transform(Some(&Transform { y: 72.0, ..Default::default() }))),
            // Every sixth caption in the accent colour keeps the rhythm visible.
            text: Some(TextStyle { content: cue.text.clone(), color: if index % 6 == 0 { accent.into() } else { "#ffffff".into() }, ..caption_style() }),
            anim_in: Some(Animation { kind: AnimationInKind::Pop, duration: 0.18 }),
            ..Default::default()
        })
        .collect()
}

/// Deterministic fraction in [0, 1) from a string (FNV-1a).
fn stable_fraction(text: &str) -> f64 {
    let mut hash: u32 = 0x811c9dc5;
    for unit in text.encode_utf16() {
        hash ^= unit as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    (hash % 10000) as f64 / 10000.0
}

/// A stable, interesting section of a long background video.
pub fn background_offset(background: &MediaAsset, voice: &MediaAsset, duration: f64) -> f64 {
    let length = background.duration.unwrap_or(0.0);
    if length <= duration {
        return 0.0;
    }
    let candidate = length * 0.25 + stable_fraction(&format!("{}{}", background.path, voice.path)) * length * 0.4;
    clamp(candidate, 0.0, length - duration)
}

/// Photo motion as editable keyframes: slide in, slow drift, slide out.
pub fn editorial_motion(duration: f64, variant: usize, base: Transform) -> Vec<Keyframe> {
    let directions = [(1.1, -0.45), (-1.1, 0.4), (0.65, 0.8), (-0.7, -0.75)];
    let (dx, dy) = directions[variant % 4];
    let side = if variant % 2 == 0 { 1.0 } else { -1.0 };
    let slide = (duration * 0.24).min(0.38);
    let (sx, sy) = (base.x - dx, base.y - dy);
    let (ex, ey) = (base.x + dx, base.y + dy);
    let end_scale = base.scale * 1.065;
    let frame = |time: f64, x: f64, y: f64, scale: f64, opacity: f64, easing: Easing| Keyframe {
        id: uid("kf"),
        time,
        transform: normalize_transform(Some(&Transform { x, y, scale, rotation: base.rotation, opacity })),
        easing,
        bezier: None,
    };
    vec![
        frame(0.0, sx + side * 26.0, sy, base.scale, 0.0, Easing::Linear),
        frame(slide, sx, sy, base.scale, 1.0, Easing::EaseOut),
        frame((duration - slide).max(slide + 0.05), ex, ey, end_scale, 1.0, Easing::EaseInOut),
        frame(duration, ex - side * 26.0, ey, end_scale, 0.0, Easing::EaseIn),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageEvent {
    pub image_index: usize,
    pub start: f64,
    pub end: f64,
}

/// Keeps AI placements inside sane bounds, or spreads the photos evenly without AI.
pub fn normalize_image_events(events: &[ImageEvent], image_count: usize, duration: f64) -> Vec<ImageEvent> {
    if image_count == 0 || duration <= 1.4 {
        return vec![];
    }
    let intro = (duration * 0.12).min(1.1);
    let max_events = (((duration - intro) / 3.2).floor() as usize).clamp(1, image_count);
    let mut sorted = events.to_vec();
    sorted.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut used = std::collections::HashSet::new();
    let mut result = Vec::new();
    let mut last_end = intro - 0.25;
    for event in sorted {
        if result.len() >= max_events || event.image_index >= image_count || used.contains(&event.image_index) {
            continue;
        }
        let start = intro.max(event.start).max(last_end + 0.25);
        let mut end = duration.min(event.end);
        if end - start < 1.25 {
            end = duration.min(start + 3.4);
        }
        end = end.min(start + 4.8);
        if end - start < 0.8 {
            continue;
        }
        result.push(ImageEvent { image_index: event.image_index, start, end });
        used.insert(event.image_index);
        last_end = end;
    }
    if !result.is_empty() {
        return result;
    }
    let window = (duration - intro) / max_events as f64;
    (0..max_events)
        .map(|index| {
            let start = intro + index as f64 * window;
            ImageEvent { image_index: index, start, end: duration.min(start + (window - 0.25).clamp(1.2, 4.0)) }
        })
        .collect()
}

pub struct AutoEditInput<'a> {
    pub voice: &'a MediaAsset,
    pub background: &'a MediaAsset,
    pub images: &'a [MediaAsset],
    pub events: &'a [ImageEvent],
    pub cues: &'a [CaptionCue],
    pub whoosh_in: Option<&'a MediaAsset>,
    pub whoosh_out: Option<&'a MediaAsset>,
    pub accent: &'a str,
    pub fps: u32,
    pub width: u32,
    pub height: u32,
}

pub fn build_auto_edit_project(input: &AutoEditInput) -> Project {
    let duration = input.voice.duration.unwrap_or(30.0).min(600.0);
    let stem = input.voice.name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(&input.voice.name);
    let mut project = create_project(stem, input.width, input.height, input.fps);
    project.origin = Some("Auto-edit".into());
    project.assets = std::iter::once(input.voice)
        .chain(std::iter::once(input.background))
        .chain(input.images.iter())
        .chain(input.whoosh_in)
        .chain(input.whoosh_out)
        .cloned()
        .collect();

    let mut image_items = Vec::new();
    let mut sfx_items = Vec::new();
    for (index, event) in normalize_image_events(input.events, input.images.len(), duration).into_iter().enumerate() {
        let image = &input.images[event.image_index];
        let length = event.end - event.start;
        let base = Transform { y: 46.0, scale: 0.86, ..Default::default() };
        image_items.push(TimelineItem {
            id: uid("image"),
            kind: ItemKind::Image,
            name: image.name.clone(),
            asset_id: Some(image.id.clone()),
            start: event.start,
            duration: length,
            fit: Some(Fit::Contain),
            radius: Some(28.0),
            transform: Some(base),
            keyframes: editorial_motion(length, index, base),
            ..Default::default()
        });
        let sound = |asset: &MediaAsset, name: &str, start: f64| TimelineItem {
            id: uid("sfx"),
            kind: ItemKind::Audio,
            name: name.into(),
            asset_id: Some(asset.id.clone()),
            start: start.max(0.0),
            duration: asset.duration.unwrap_or(0.4),
            volume: Some(0.8),
            source_in: Some(0.0),
            ..Default::default()
        };
        if let Some(whoosh) = input.whoosh_in {
            sfx_items.push(sound(whoosh, "Whoosh in", event.start - 0.05));
        }
        if let Some(whoosh) = input.whoosh_out.filter(|_| length > 1.2) {
            sfx_items.push(sound(whoosh, "Whoosh out", event.end - whoosh.duration.unwrap_or(0.4)));
        }
    }

    // The background loops when it is shorter than the voiceover.
    let background_length = input.background.duration.unwrap_or(duration);
    let offset = background_offset(input.background, input.voice, duration);
    let mut backgrounds = Vec::new();
    let mut cursor = 0.0;
    for index in 0..50 {
        if cursor >= duration - 0.05 {
            break;
        }
        let source_in = if index == 0 { offset } else { 0.0 };
        let length = (duration - cursor).min(background_length - source_in);
        if length <= 0.05 {
            break;
        }
        backgrounds.push(TimelineItem {
            id: uid("video"),
            kind: ItemKind::Video,
            name: input.background.name.clone(),
            asset_id: Some(input.background.id.clone()),
            start: cursor,
            duration: length,
            source_in: Some(source_in),
            speed: Some(1.0),
            volume: Some(0.0),
            muted: true,
            fit: Some(Fit::Cover),
            transform: Some(Transform::default()),
            ..Default::default()
        });
        cursor += length;
    }
    let voiceover = TimelineItem {
        id: uid("audio"),
        kind: ItemKind::Audio,
        name: input.voice.name.clone(),
        asset_id: Some(input.voice.id.clone()),
        start: 0.0,
        duration,
        source_in: Some(0.0),
        speed: Some(1.0),
        volume: Some(1.0),
        ..Default::default()
    };
    let mut tracks = vec![track(TrackKind::Text, "Captions", caption_items(input.cues, input.accent))];
    if !image_items.is_empty() {
        tracks.push(track(TrackKind::Video, "Photos", image_items));
    }
    tracks.push(track(TrackKind::Video, track_label(TrackKind::Video), backgrounds));
    tracks.push(track(TrackKind::Audio, "Voiceover", vec![voiceover]));
    if !sfx_items.is_empty() {
        tracks.push(track(TrackKind::Audio, "Sound effects", sfx_items));
    }
    project.tracks = tracks;
    project
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(kind: MediaKind, name: &str, duration: f64) -> MediaAsset {
        MediaAsset { id: uid("asset"), kind, name: name.into(), path: format!("C:/media/{name}"), duration: Some(duration), ..Default::default() }
    }

    #[test]
    fn spreads_photos_evenly_without_ai() {
        let events = normalize_image_events(&[], 3, 20.0);
        assert_eq!(events.len(), 3);
        assert!(events.windows(2).all(|pair| pair[0].end <= pair[1].start));
        assert!(events[0].start >= 1.0);
    }

    #[test]
    fn cleans_up_ai_placements() {
        let raw = [
            ImageEvent { image_index: 1, start: 0.0, end: 9.0 },
            ImageEvent { image_index: 1, start: 6.0, end: 8.0 },
            ImageEvent { image_index: 7, start: 10.0, end: 12.0 },
            ImageEvent { image_index: 0, start: 4.0, end: 4.3 },
        ];
        let events = normalize_image_events(&raw, 2, 30.0);
        // Out-of-range and repeated photos go; short and long ones are fitted.
        assert_eq!(events.iter().map(|e| e.image_index).collect::<Vec<_>>(), [1, 0]);
        assert!(events[0].end - events[0].start <= 4.8 + 1e-9);
        assert!(events[1].start >= events[0].end + 0.25 - 1e-9);
    }

    #[test]
    fn builds_an_editable_project() {
        let voice = asset(MediaKind::Audio, "story.mp3", 25.0);
        let background = asset(MediaKind::Video, "gameplay.mp4", 10.0);
        let images = [asset(MediaKind::Image, "a.png", 0.0), asset(MediaKind::Image, "b.png", 0.0)];
        let whoosh = asset(MediaKind::Audio, "whoosh-in.mp3", 0.4);
        let cues = [CaptionCue { text: "Hello there".into(), start: 0.2, duration: 1.0, highlight_index: None }];
        let project = build_auto_edit_project(&AutoEditInput {
            voice: &voice,
            background: &background,
            images: &images,
            events: &[],
            cues: &cues,
            whoosh_in: Some(&whoosh),
            whoosh_out: None,
            accent: "#ffd43b",
            fps: 30,
            width: 1080,
            height: 1920,
        });
        assert_eq!(project.name, "story");
        let names: Vec<&str> = project.tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Captions", "Photos", "Video", "Voiceover", "Sound effects"]);
        // The 10 s background loops to cover the 25 s voiceover.
        let backgrounds = &project.tracks[2].items;
        assert_eq!(backgrounds.len(), 3);
        assert!((backgrounds.iter().map(|i| i.duration).sum::<f64>() - 25.0).abs() < 1e-9);
        assert!(project.tracks.iter().all(|t| t.items.iter().all(|i| i.track_id == t.id)));
        assert_eq!(project.tracks[0].items[0].text.as_ref().unwrap().color, "#ffd43b");
        assert_eq!(project.tracks[1].items[0].keyframes.len(), 4);
        assert_eq!(project.assets.len(), 5);
    }
}
