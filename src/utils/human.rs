// src/utils/human.rs
//
// Human-readable formatting utilities for sizes, dates, and durations.

use std::time::{SystemTime, UNIX_EPOCH};
use humansize::{format_size, BINARY};

/// Format a byte count as a human-readable string (e.g. "1.2 MiB", "930 B").
pub fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return "—".to_string(); // directories show dash
    }
    format_size(bytes, BINARY)
}

/// Format a SystemTime as a relative or absolute date string.
/// Recent times (within 7 days) show relative ("2 hours ago", "3 days ago").
/// Older times show absolute ("2024-05-12").
pub fn format_modified(time: Option<SystemTime>) -> String {
    let Some(t) = time else { return "—".to_string() };

    let secs = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let diff = now_secs.saturating_sub(secs);

    if diff < 60 {
        return "just now".to_string();
    } else if diff < 3600 {
        let m = diff / 60;
        return format!("{} min ago", m);
    } else if diff < 86400 {
        let h = diff / 3600;
        return format!("{} hr ago", h);
    } else if diff < 86400 * 7 {
        let d = diff / 86400;
        return format!("{} day{} ago", d, if d == 1 { "" } else { "s" });
    }

    // Absolute date for older files
    use chrono::{DateTime, Local};
    let dt: DateTime<Local> = DateTime::from(t);
    dt.format("%Y-%m-%d").to_string()
}

/// Returns a "recency" value [0.0 .. 1.0] for heat-map coloring.
/// 1.0 = modified just now, 0.0 = modified more than 30 days ago.
pub fn recency_score(time: Option<SystemTime>) -> f32 {
    let Some(t) = time else { return 0.0 };
    let secs = SystemTime::now()
        .duration_since(t)
        .unwrap_or_default()
        .as_secs();
    let max_secs: f32 = 86400.0 * 30.0; // 30 days
    (1.0 - (secs as f32 / max_secs)).clamp(0.0, 1.0)
}
