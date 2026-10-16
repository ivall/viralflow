//! SubRip (.srt) and simple WebVTT subtitles.

use crate::model::CaptionCue;

fn parse_timestamp(value: &str) -> Option<f64> {
    let value = value.trim();
    // WebVTT allows mm:ss.mmm without hours.
    let value = if value.matches(':').count() == 1 { format!("0:{value}") } else { value.to_string() };
    let (clock, millis) = value.split_once([',', '.'])?;
    let parts: Vec<&str> = clock.split(':').collect();
    if parts.len() != 3 || millis.is_empty() || millis.len() > 3 || !millis.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let number = |text: &str, max_len: usize| {
        (!text.is_empty() && text.len() <= max_len && text.chars().all(|c| c.is_ascii_digit())).then(|| text.parse::<f64>().ok()).flatten()
    };
    let hours = number(parts[0], 9)?;
    let minutes = number(parts[1], 2)?;
    let seconds = number(parts[2], 2)?;
    let millis: f64 = format!("{millis:0<3}").parse().ok()?;
    Some(hours * 3600.0 + minutes * 60.0 + seconds + millis / 1000.0)
}

fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = false;
    for c in text.chars() {
        match c {
            '<' => depth = true,
            '>' if depth => depth = false,
            _ if !depth => out.push(c),
            _ => {}
        }
    }
    out
}

/// Parses SubRip and simple WebVTT text.
pub fn parse_subtitles(text: &str) -> Vec<CaptionCue> {
    let normalized = text.replace('\r', "");
    let body = match normalized.strip_prefix("WEBVTT") {
        Some(rest) => rest.split_once('\n').map(|(_, rest)| rest).unwrap_or(""),
        None => normalized.as_str(),
    };
    let mut cues = Vec::new();
    for block in body.split("\n\n") {
        let lines: Vec<&str> = block.lines().filter(|line| !line.trim().is_empty()).collect();
        let Some(timing) = lines.iter().position(|line| line.contains("-->")) else { continue };
        let (from, to) = lines[timing].split_once("-->").unwrap_or(("", ""));
        let first_word = |part: &str| part.split_whitespace().next().unwrap_or("").to_string();
        let (Some(start), Some(end)) = (parse_timestamp(&first_word(from)), parse_timestamp(&first_word(to))) else { continue };
        let content = strip_tags(&lines[timing + 1..].join("\n")).trim().to_string();
        if end <= start || content.is_empty() {
            continue;
        }
        cues.push(CaptionCue { text: content, start, duration: end - start, highlight_index: None });
    }
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    cues
}

fn timestamp(seconds: f64) -> String {
    let total = (seconds * 1000.0).round().max(0.0) as u64;
    format!("{:02}:{:02}:{:02},{:03}", total / 3_600_000, (total % 3_600_000) / 60_000, (total % 60_000) / 1000, total % 1000)
}

pub fn format_srt(cues: &[CaptionCue]) -> String {
    let mut sorted: Vec<&CaptionCue> = cues.iter().collect();
    sorted.sort_by(|a, b| a.start.total_cmp(&b.start));
    sorted
        .iter()
        .enumerate()
        .map(|(index, cue)| format!("{}\n{} --> {}\n{}\n", index + 1, timestamp(cue.start), timestamp(cue.start + cue.duration), cue.text))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_srt() {
        let cues = vec![
            CaptionCue { text: "Hello there".into(), start: 1.2, duration: 1.5, highlight_index: None },
            CaptionCue { text: "Second\nline".into(), start: 3.0, duration: 0.75, highlight_index: None },
        ];
        let parsed = parse_subtitles(&format_srt(&cues));
        assert_eq!(parsed.len(), 2);
        assert!((parsed[0].start - 1.2).abs() < 1e-9);
        assert!((parsed[0].duration - 1.5).abs() < 1e-9);
        assert_eq!(parsed[1].text, "Second\nline");
    }

    #[test]
    fn reads_webvtt_with_short_timestamps_and_tags() {
        let parsed = parse_subtitles("WEBVTT\n\n00:01.500 --> 00:03.000 align:start\n<b>Bold</b> words\n");
        assert_eq!(parsed, vec![CaptionCue { text: "Bold words".into(), start: 1.5, duration: 1.5, highlight_index: None }]);
    }
}
