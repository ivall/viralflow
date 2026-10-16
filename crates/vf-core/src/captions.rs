//! Turning transcripts into caption clips and back.

use crate::model::{CaptionCue, ItemKind, Project};
use crate::timeline::MIN_CLIP_DURATION;

#[derive(Debug, Clone, PartialEq)]
pub struct TimedWord {
    pub word: String,
    pub start: f64,
    pub end: f64,
}

fn normalize_word(word: &str) -> String {
    word.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Splits every caption into one cue per spoken word: the text stays the same
/// and only the highlighted word moves on. Words are timed by the transcript
/// where its words match the caption, and estimated from their length where
/// they do not (sentence-level transcripts, edited captions).
pub fn highlight_cues(cues: &[CaptionCue], words: &[TimedWord]) -> Vec<CaptionCue> {
    let mut result = Vec::new();
    let mut cursor = 0usize;
    for cue in cues {
        let parts: Vec<&str> = cue.text.split_whitespace().collect();
        let wanted: Vec<String> = parts.iter().map(|p| normalize_word(p)).collect();
        let mut found = None;
        let mut index = cursor;
        while !parts.is_empty() && index + parts.len() <= words.len() && index <= cursor + 8 {
            if wanted.iter().enumerate().all(|(offset, word)| normalize_word(&words[index + offset].word) == *word) {
                found = Some(index);
                break;
            }
            index += 1;
        }
        let end = cue.start + cue.duration;
        let starts: Vec<f64> = match found {
            Some(found) => {
                cursor = found + parts.len();
                // Each word is lit from its own start until the next word starts.
                words[found..found + parts.len()].iter().enumerate().map(|(i, w)| if i == 0 { cue.start } else { end.min(cue.start.max(w.start)) }).collect()
            }
            None => estimated_starts(cue, &parts),
        };
        push_word_cues(&mut result, cue, starts);
    }
    result
}

/// Word start times spread over the cue in proportion to the words' length.
pub fn estimated_starts(cue: &CaptionCue, parts: &[&str]) -> Vec<f64> {
    let weights: Vec<f64> = parts.iter().map(|p| p.chars().count() as f64 + 1.0).collect();
    let total: f64 = weights.iter().sum::<f64>().max(1.0);
    let mut at = cue.start;
    weights
        .iter()
        .map(|w| {
            let start = at;
            at += cue.duration * w / total;
            start
        })
        .collect()
}

fn push_word_cues(result: &mut Vec<CaptionCue>, cue: &CaptionCue, mut starts: Vec<f64>) {
    let end = cue.start + cue.duration;
    if starts.len() <= 1 {
        result.push(CaptionCue { highlight_index: Some(0), ..cue.clone() });
        return;
    }
    for i in 1..starts.len() {
        starts[i] = starts[i].max(starts[i - 1] + MIN_CLIP_DURATION);
    }
    for i in 0..starts.len() {
        let start = starts[i];
        let next = if i + 1 < starts.len() { starts[i + 1] } else { end };
        let stop = if next > end - MIN_CLIP_DURATION { end } else { next };
        if stop - start < MIN_CLIP_DURATION {
            break;
        }
        result.push(CaptionCue { text: cue.text.clone(), start, duration: stop - start, highlight_index: Some(i as i64) });
        if stop >= end {
            break;
        }
    }
}

/// Captions on caption tracks, one cue per caption: word-highlight clips of the
/// same caption are merged back into one cue.
pub fn caption_cues(project: &Project) -> Vec<CaptionCue> {
    let mut cues: Vec<CaptionCue> = project
        .tracks
        .iter()
        .filter(|track| track.kind == crate::model::TrackKind::Text && !track.hidden)
        .flat_map(|track| {
            track.items.iter().filter(move |item| {
                item.kind == ItemKind::Text
                    && !item.hidden
                    && item.text.as_ref().is_some_and(|t| !t.content.trim().is_empty())
                    && (item.group_id.as_deref().is_some_and(|g| g.starts_with("captions-")) || track.name.starts_with("Captions"))
            })
        })
        .map(|item| {
            let text = item.text.as_ref().unwrap();
            CaptionCue { text: text.content.clone(), start: item.start, duration: item.duration, highlight_index: Some(text.highlight_index) }
        })
        .collect();
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut merged: Vec<CaptionCue> = Vec::new();
    for cue in cues {
        if let Some(previous) = merged.last_mut() {
            if cue.highlight_index.unwrap_or(-1) > 0 && previous.text == cue.text && (previous.start + previous.duration - cue.start).abs() < 0.02 {
                previous.duration = cue.start + cue.duration - previous.start;
                continue;
            }
        }
        merged.push(CaptionCue { highlight_index: None, ..cue });
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_captions_into_word_cues() {
        let cues = vec![CaptionCue { text: "one two three".into(), start: 0.0, duration: 1.5, highlight_index: None }];
        let words = vec![
            TimedWord { word: "One".into(), start: 0.0, end: 0.4 },
            TimedWord { word: "two,".into(), start: 0.5, end: 0.9 },
            TimedWord { word: "three".into(), start: 1.0, end: 1.4 },
        ];
        let split = highlight_cues(&cues, &words);
        assert_eq!(split.len(), 3);
        assert_eq!(split[1].highlight_index, Some(1));
        assert!((split[1].start - 0.5).abs() < 1e-9);
        assert!((split[2].start + split[2].duration - 1.5).abs() < 1e-9);
    }

    #[test]
    fn untimed_words_are_estimated_per_caption() {
        let cues = vec![
            CaptionCue { text: "hello there".into(), start: 0.0, duration: 1.0, highlight_index: None },
            CaptionCue { text: "one two".into(), start: 1.0, duration: 1.0, highlight_index: None },
        ];
        // Only the second caption's words are in the transcript.
        let words = vec![TimedWord { word: "one".into(), start: 1.0, end: 1.3 }, TimedWord { word: "two".into(), start: 1.4, end: 2.0 }];
        let split = highlight_cues(&cues, &words);
        assert_eq!(split.iter().map(|c| c.highlight_index.unwrap()).collect::<Vec<_>>(), [0, 1, 0, 1], "every caption lights word by word");
        // "hello" and "there" are as long: half each.
        assert!((split[1].start - 0.5).abs() < 1e-9, "{:?}", split[1]);
        assert!((split[3].start - 1.4).abs() < 1e-9, "timed where the transcript matches");
        // Without any word timings at all.
        assert_eq!(highlight_cues(&cues, &[]).len(), 4);
    }
}
