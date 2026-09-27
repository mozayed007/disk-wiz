//! Shared helpers: size parsing and formatting, counts, time, text width.

use std::borrow::Cow;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Current unix time in seconds.
pub fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Parse a human-readable size such as `512`, `10K`, `1.5MiB`, `2 GB`.
///
/// Units without `i` are decimal (`1 KB` = 1000), units with `i` are binary
/// (`1 KiB` = 1024), and a bare `K`/`M`/`G` is binary, matching GNU coreutils.
pub fn parse_size(input: &str) -> Result<u64, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("empty size".to_string());
    }
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let value: f64 = num
        .parse()
        .map_err(|_| format!("invalid size: {input:?}"))?;
    if !value.is_finite() || value < 0.0 {
        return Err(format!("invalid size: {input:?}"));
    }
    let unit = unit.trim().to_ascii_lowercase();
    let mult: f64 = match unit.as_str() {
        "" | "b" => 1.0,
        "k" | "ki" | "kib" => 1024.0,
        "kb" => 1000.0,
        "m" | "mi" | "mib" => 1024.0 * 1024.0,
        "mb" => 1e6,
        "g" | "gi" | "gib" => 1024.0f64.powi(3),
        "gb" => 1e9,
        "t" | "ti" | "tib" => 1024.0f64.powi(4),
        "tb" => 1e12,
        "p" | "pi" | "pib" => 1024.0f64.powi(5),
        "pb" => 1e15,
        _ => return Err(format!("unknown size unit in {input:?}")),
    };
    Ok((value * mult).round() as u64)
}

/// Format bytes with binary units: `881 GiB`, `9.7 GiB`, `512 B`.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i + 1 < UNITS.len() {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else if v >= 10.0 {
        format!("{v:.0} {}", UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// Format a count with k/M/B suffixes: `3.9M`, `499.4k`, `812`.
pub fn format_count(n: u64) -> String {
    const UNITS: [&str; 4] = ["", "k", "M", "B"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1000.0 && i + 1 < UNITS.len() {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        n.to_string()
    } else {
        format!("{v:.1}{}", UNITS[i])
    }
}

/// Format a scan duration: `250ms`, `1.0s`, `2m 3s`.
pub fn format_duration(d: Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        format!("{ms}ms")
    } else if d.as_secs() < 60 {
        format!("{:.1}s", d.as_secs_f64())
    } else {
        let m = d.as_secs() / 60;
        let s = d.as_secs() % 60;
        format!("{m}m {s}s")
    }
}

/// Format an mtime as a relative age: `32 minutes ago`, `2 days ago`.
pub fn format_age(epoch: u64) -> String {
    let secs = now_epoch().saturating_sub(epoch);
    match secs {
        0..=59 => "just now".to_string(),
        60..=3599 => plural(secs / 60, "minute"),
        3600..=86_399 => plural(secs / 3600, "hour"),
        86_400..=2_591_999 => plural(secs / 86_400, "day"),
        2_592_000..=31_535_999 => plural(secs / 2_592_000, "month"),
        _ => plural(secs / 31_536_000, "year"),
    }
}

fn plural(n: u64, unit: &str) -> String {
    if n == 1 {
        format!("{n} {unit} ago")
    } else {
        format!("{n} {unit}s ago")
    }
}

/// Display width of a string in terminal cells.
pub fn display_width(s: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    UnicodeWidthStr::width(s)
}

/// Truncate a string to a display width, appending an ellipsis when cut.
pub fn truncate_to_width(s: &str, max: usize) -> Cow<'_, str> {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    if UnicodeWidthStr::width(s) <= max {
        return Cow::Borrowed(s);
    }
    if max == 0 {
        return Cow::Borrowed("");
    }
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > max.saturating_sub(1) {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sizes() {
        assert_eq!(parse_size("512").unwrap(), 512);
        assert_eq!(parse_size("1K").unwrap(), 1024);
        assert_eq!(parse_size("1KB").unwrap(), 1000);
        assert_eq!(parse_size("1KiB").unwrap(), 1024);
        assert_eq!(parse_size("1.5M").unwrap(), 1024 * 1024 * 3 / 2);
        assert_eq!(parse_size("2 GiB").unwrap(), 2 * 1024 * 1024 * 1024);
        assert_eq!(parse_size("1t").unwrap(), 1024u64.pow(4));
        assert!(parse_size("").is_err());
        assert!(parse_size("abc").is_err());
        assert!(parse_size("10XB").is_err());
        assert!(parse_size("-5M").is_err());
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1.0 KiB");
        assert_eq!(format_size(1536), "1.5 KiB");
        assert_eq!(format_size(10 * 1024), "10 KiB");
        assert_eq!(format_size(881 * 1024 * 1024 * 1024), "881 GiB");
        assert_eq!(format_size(96 * 1024 * 1024 * 1024), "96 GiB");
    }

    #[test]
    fn formats_counts() {
        assert_eq!(format_count(812), "812");
        assert_eq!(format_count(3900), "3.9k");
        assert_eq!(format_count(499_400), "499.4k");
        assert_eq!(format_count(3_900_000), "3.9M");
    }

    #[test]
    fn truncates_by_width() {
        assert_eq!(truncate_to_width("hello", 10), "hello");
        assert_eq!(truncate_to_width("hello", 5), "hello");
        assert_eq!(truncate_to_width("hello", 4), "hel…");
        assert_eq!(truncate_to_width("hello", 1), "…");
        assert_eq!(truncate_to_width("hello", 0), "");
        assert_eq!(truncate_to_width("日本語テスト", 4), "日…");
    }

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(Duration::from_millis(250)), "250ms");
        assert_eq!(format_duration(Duration::from_millis(1500)), "1.5s");
        assert_eq!(format_duration(Duration::from_secs(123)), "2m 3s");
    }
}
