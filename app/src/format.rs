//! Human-readable times and sizes.

/// "mm:ss", or "mm:ss:ff" with frames when `fps` is given.
pub fn time(seconds: f64, fps: Option<u32>) -> String {
    let safe = seconds.max(0.0);
    let minutes = (safe / 60.0).floor() as u64;
    let secs = (safe % 60.0).floor() as u64;
    match fps {
        Some(fps) => {
            let frames = ((safe % 1.0) * fps as f64 + 1e-6).floor() as u64;
            format!("{minutes:02}:{secs:02}:{frames:02}")
        }
        None => format!("{minutes:02}:{secs:02}"),
    }
}

/// "4.5s" under a minute, "mm:ss" above.
pub fn duration(seconds: f64) -> String {
    if seconds < 60.0 {
        if seconds < 10.0 {
            format!("{seconds:.1}s")
        } else {
            format!("{seconds:.0}s")
        }
    } else {
        time(seconds, None)
    }
}

pub fn bytes(value: u64) -> String {
    let value = value as f64;
    if value < 1024.0 * 1024.0 {
        format!("{} KB", (value / 1024.0).round().max(1.0))
    } else if value < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB", value / 1024.0 / 1024.0)
    } else {
        format!("{:.2} GB", value / 1024.0 / 1024.0 / 1024.0)
    }
}

/// "Today, 14:05", "Yesterday", "3 days ago" or a date.
pub fn edited(iso: &str) -> String {
    let Ok(date) = chrono::DateTime::parse_from_rfc3339(iso) else { return String::new() };
    let local = date.with_timezone(&chrono::Local);
    let now = chrono::Local::now();
    let days = (now.date_naive() - local.date_naive()).num_days();
    match days {
        ..=0 => format!("Today, {}", local.format("%H:%M")),
        1 => "Yesterday".into(),
        2..=6 => format!("{days} days ago"),
        _ => local.format("%Y-%m-%d").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_times() {
        assert_eq!(time(75.5, None), "01:15");
        assert_eq!(time(1.5, Some(30)), "00:01:15");
        assert_eq!(duration(4.25), "4.2s");
        assert_eq!(duration(90.0), "01:30");
        assert_eq!(bytes(5 * 1024 * 1024), "5.0 MB");
    }
}
