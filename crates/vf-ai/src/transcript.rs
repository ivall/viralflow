//! Turns timed words or segments into short caption cues (same grouping rules
//! as the Tauri app).

pub use vf_core::captions::TimedWord;
use vf_core::model::CaptionCue;

const CAPTION_VISUAL_LIMIT: f64 = 13.0;

#[derive(Debug, Clone, PartialEq)]
pub struct TimedSegment {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone)]
pub struct Transcript {
    pub text: String,
    pub cues: Vec<CaptionCue>,
    pub words: Vec<TimedWord>,
    /// "word", "segment" or "estimated".
    pub alignment: String,
}

#[derive(Debug, Clone, Copy)]
pub struct CaptionOptions {
    /// Maximum words per caption (1 = word by word).
    pub max_words: usize,
}

impl Default for CaptionOptions {
    fn default() -> Self {
        CaptionOptions { max_words: 3 }
    }
}

fn clean(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn visual_length(value: &str) -> f64 {
    value
        .chars()
        .map(|c| match c {
            'M' | 'W' | 'm' | 'w' => 1.35,
            'i' | 'l' | 'I' | '.' | ',' | '\'' => 0.55,
            ' ' => 0.5,
            _ => 1.0,
        })
        .sum()
}

fn fits(words: &[String], max_words: usize) -> bool {
    words.len() <= max_words && (words.len() <= 1 || visual_length(&words.join(" ")) <= CAPTION_VISUAL_LIMIT * (max_words as f64 / 2.0).max(1.0))
}

fn ends_sentence(word: &str) -> bool {
    word.ends_with(['.', '!', '?', ',', ';', ':'])
}

pub fn cues_from_words(words: &[TimedWord], duration: f64, options: CaptionOptions) -> Vec<CaptionCue> {
    let max_words = options.max_words.clamp(1, 12);
    let mut items = words.to_vec();
    items.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    let mut result: Vec<CaptionCue> = Vec::new();
    let mut chunk: Vec<String> = Vec::new();
    let (mut chunk_start, mut chunk_end) = (0.0f64, 0.0f64);
    let flush = |chunk: &mut Vec<String>, start: f64, end: f64, result: &mut Vec<CaptionCue>| {
        if chunk.is_empty() {
            return;
        }
        let start = (start - 0.02).max(0.0);
        let end = duration.min((start + 0.08).max(end));
        if end - start >= 0.06 {
            result.push(CaptionCue { text: chunk.join(" "), start, duration: end - start, highlight_index: None });
        }
        chunk.clear();
    };
    for item in items {
        let word = clean(&item.word);
        if word.is_empty() {
            continue;
        }
        let start = item.start.max(0.0);
        let end = (start + 0.04).max(item.end);
        if !chunk.is_empty() && start - chunk_end > 0.35 {
            flush(&mut chunk, chunk_start, chunk_end, &mut result);
        }
        if !chunk.is_empty() {
            let mut candidate = chunk.clone();
            candidate.push(word.clone());
            if !fits(&candidate, max_words) {
                flush(&mut chunk, chunk_start, chunk_end, &mut result);
            }
        }
        if chunk.is_empty() {
            chunk_start = start;
            chunk_end = 0.0;
        }
        chunk.push(word.clone());
        chunk_end = chunk_end.max(end);
        if chunk.len() >= max_words || chunk_end - chunk_start >= 0.4 * max_words as f64 + 0.6 || ends_sentence(&word) {
            flush(&mut chunk, chunk_start, chunk_end, &mut result);
        }
    }
    flush(&mut chunk, chunk_start, chunk_end, &mut result);
    finalize(result, duration)
}

fn group_words(words: &[String], max_words: usize) -> Vec<Vec<String>> {
    let mut groups = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for word in words {
        let mut candidate = current.clone();
        candidate.push(word.clone());
        if !current.is_empty() && !fits(&candidate, max_words) {
            groups.push(std::mem::take(&mut current));
        }
        current.push(word.clone());
        if current.len() >= max_words || ends_sentence(word) {
            groups.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

/// Spreads the words of each segment proportionally to their length.
pub fn words_from_segments(segments: &[TimedSegment]) -> Vec<TimedWord> {
    let mut words = Vec::new();
    for segment in segments {
        let parts: Vec<String> = clean(&segment.text).split(' ').filter(|w| !w.is_empty()).map(String::from).collect();
        if parts.is_empty() {
            continue;
        }
        let total: f64 = parts.iter().map(|w| visual_length(w) + 0.6).sum();
        let span = (segment.end - segment.start).max(0.05);
        let mut cursor = segment.start;
        for word in parts {
            let share = (visual_length(&word) + 0.6) / total * span;
            words.push(TimedWord { word, start: cursor, end: cursor + share });
            cursor += share;
        }
    }
    words
}

pub fn estimated_cues(transcript: &str, duration: f64, options: CaptionOptions) -> Vec<CaptionCue> {
    let words: Vec<String> = clean(transcript).split(' ').filter(|w| !w.is_empty()).map(String::from).collect();
    if words.is_empty() || duration <= 0.0 {
        return vec![];
    }
    let groups = group_words(&words, options.max_words.clamp(1, 12));
    let step = duration / groups.len() as f64;
    finalize(
        groups
            .into_iter()
            .enumerate()
            .map(|(index, group)| CaptionCue { text: group.join(" "), start: index as f64 * step, duration: step.max(0.08), highlight_index: None })
            .collect(),
        duration,
    )
}

/// Clamps cues to the clip and trims overlaps without shifting later cues.
fn finalize(mut cues: Vec<CaptionCue>, duration: f64) -> Vec<CaptionCue> {
    cues.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    let mut result: Vec<CaptionCue> = Vec::new();
    for mut cue in cues {
        cue.text = clean(&cue.text);
        if cue.text.is_empty() || cue.start >= duration {
            continue;
        }
        cue.start = cue.start.max(0.0);
        let end = duration.min(cue.start + cue.duration);
        if end - cue.start < 0.06 {
            continue;
        }
        cue.duration = end - cue.start;
        result.push(cue);
    }
    for index in 0..result.len().saturating_sub(1) {
        let next = result[index + 1].start;
        let current_end = result[index].start + result[index].duration;
        if current_end > next && next - result[index].start >= 0.06 {
            result[index].duration = next - result[index].start;
        }
    }
    result
}

/// Aligns token timings (which lost their spaces) to the words of the segment
/// transcript. Returns None when the two transcripts disagree.
pub fn align_tokens(segments: &[TimedSegment], tokens: &[TimedSegment]) -> Option<Vec<TimedWord>> {
    let words: Vec<String> = segments.iter().flat_map(|s| clean(&s.text).split(' ').filter(|w| !w.is_empty()).map(String::from).collect::<Vec<_>>()).collect();
    let mut token_chars: Vec<(char, f64, f64)> = Vec::new();
    for token in tokens {
        for c in token.text.chars().filter(|c| !c.is_whitespace()) {
            token_chars.push((c, token.start, token.end));
        }
    }
    let mut cursor = 0usize;
    let mut result = Vec::with_capacity(words.len());
    for word in words {
        let mut start = None;
        let mut end = 0.0f64;
        for c in word.chars() {
            let (token_char, token_start, token_end) = *token_chars.get(cursor)?;
            if token_char != c {
                return None;
            }
            start.get_or_insert(token_start);
            end = end.max(token_end);
            cursor += 1;
        }
        let start = start?;
        result.push(TimedWord { word, start, end: end.max(start + 0.04) });
    }
    Some(result)
}

pub fn build_transcript(segments: Vec<TimedSegment>, words: Option<Vec<TimedWord>>, duration: f64, options: CaptionOptions) -> Transcript {
    let text = clean(&segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" "));
    if let Some(words) = words.filter(|w| !w.is_empty()) {
        let cues = cues_from_words(&words, duration, options);
        return Transcript { text, cues, words, alignment: "word".into() };
    }
    let words = words_from_segments(&segments);
    if !words.is_empty() {
        let cues = cues_from_words(&words, duration, options);
        return Transcript { text, cues, words, alignment: "segment".into() };
    }
    let cues = estimated_cues(&text, duration, options);
    Transcript { text, cues, words: vec![], alignment: "estimated".into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(text: &str, start: f64, end: f64) -> TimedSegment {
        TimedSegment { text: text.into(), start, end }
    }

    #[test]
    fn aligns_split_tokens_to_words() {
        let segments = vec![seg("Jakie jest najlepszy?", 0.0, 2.0)];
        let tokens = vec![
            seg("Jak", 0.07, 0.16),
            seg("ie", 0.16, 0.26),
            seg("jest", 0.26, 0.47),
            seg("najle", 0.47, 0.74),
            seg("ps", 0.74, 0.84),
            seg("zy", 0.84, 0.94),
            seg("?", 0.94, 1.1),
        ];
        let words = align_tokens(&segments, &tokens).expect("aligned");
        assert_eq!(words.len(), 3);
        assert_eq!(words[0].word, "Jakie");
        assert!((words[0].start - 0.07).abs() < 1e-9 && (words[0].end - 0.26).abs() < 1e-9);
        assert!((words[2].end - 1.1).abs() < 1e-9);
    }

    #[test]
    fn groups_words_into_short_cues() {
        let words: Vec<TimedWord> = ["one", "two", "three", "four"]
            .iter()
            .enumerate()
            .map(|(i, w)| TimedWord { word: w.to_string(), start: i as f64 * 0.3, end: i as f64 * 0.3 + 0.25 })
            .collect();
        let cues = cues_from_words(&words, 5.0, CaptionOptions { max_words: 2 });
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "one two");
    }
}
