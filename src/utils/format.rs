use chrono::{Datelike, Local, TimeZone};

pub fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{} B", bytes)
    } else {
        format!("{:.1} {}", size, UNITS[unit_idx])
    }
}

/// Formats a Unix timestamp in the local time zone, the way file managers do:
/// "Today 14:05", "Yesterday 09:30", "14 Sep 18:02" (this year), "2 Mar 2025" (older).
/// Returns an empty string for unknown (0) timestamps.
pub fn format_timestamp(secs: u64) -> String {
    if secs == 0 {
        return String::new();
    }
    let dt = match Local.timestamp_opt(secs as i64, 0).single() {
        Some(dt) => dt,
        None => return String::new(),
    };
    let today = Local::now().date_naive();
    let date = dt.date_naive();
    if date == today {
        dt.format("Today %H:%M").to_string()
    } else if today.pred_opt() == Some(date) {
        dt.format("Yesterday %H:%M").to_string()
    } else if date.year() == today.year() && date <= today {
        dt.format("%-d %b %H:%M").to_string()
    } else {
        dt.format("%-d %b %Y").to_string()
    }
}

/// Full local date and time, for places with room for detail (Properties).
pub fn format_timestamp_full(secs: u64) -> String {
    if secs == 0 {
        return String::new();
    }
    match Local.timestamp_opt(secs as i64, 0).single() {
        Some(dt) => dt.format("%-d %B %Y, %H:%M:%S").to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1536), "1.5 KiB");
    }

    #[test]
    fn timestamps() {
        assert_eq!(format_timestamp(0), "");
        let now = Local::now().timestamp() as u64;
        assert!(format_timestamp(now).starts_with("Today "));
        assert!(format_timestamp(now - 86_400).starts_with("Yesterday ") || format_timestamp(now - 86_400).starts_with("Today "));
        // 1 Jan 2001 is always "older than this year".
        assert!(format_timestamp(978_350_400).ends_with("2001"));
    }
}
