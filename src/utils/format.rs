use std::time::UNIX_EPOCH;

pub fn format_size(bytes: u64) -> String {
    if bytes == 0 {
        return String::from("--");
    }
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

pub fn format_timestamp(secs: u64) -> String {
    if secs == 0 {
        return String::from("--");
    }
    let duration = std::time::Duration::from_secs(secs);
    let datetime = UNIX_EPOCH + duration;
    match datetime.duration_since(UNIX_EPOCH) {
        Ok(d) => {
            let secs = d.as_secs();
            let days = secs / 86400;
            let remaining = secs % 86400;
            let hours = remaining / 3600;
            let minutes = (remaining % 3600) / 60;

            let mut year = 1970_i64;
            let mut days_left = days;
            loop {
                let days_in_year = if is_leap(year) { 366 } else { 365 };
                if days_left < days_in_year {
                    break;
                }
                days_left -= days_in_year;
                year += 1;
            }
            let month_days = if is_leap(year) {
                [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
            } else {
                [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
            };
            let mut month = 1u32;
            let mut remaining_days = days_left;
            for &md in &month_days {
                if remaining_days < md as u64 {
                    break;
                }
                remaining_days -= md as u64;
                month += 1;
            }
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}",
                year,
                month,
                remaining_days + 1,
                hours,
                minutes
            )
        }
        Err(_) => String::from("--"),
    }
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}
