//! Duration parsing and formatting.
//! Whole seconds stored in numbers. Supported formats: "h:mm", "h:mm:ss", "d h", "days".

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DurationFormat {
    #[default]
    #[serde(rename = "h:mm")]
    HMm,
    #[serde(rename = "h:mm:ss")]
    HMmSs,
    #[serde(rename = "d h")]
    DH,
    #[serde(rename = "days")]
    Days,
}

impl DurationFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::HMm => "h:mm",
            Self::HMmSs => "h:mm:ss",
            Self::DH => "d h",
            Self::Days => "days",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "h:mm" => Some(Self::HMm),
            "h:mm:ss" => Some(Self::HMmSs),
            "d h" => Some(Self::DH),
            "days" => Some(Self::Days),
            _ => None,
        }
    }
}

/// Parses user input string into whole seconds.
/// Accepts:
/// - Plain seconds, e.g. "5400"
/// - Colon format: "1:30", "01:30:00"
/// - Unit formats: "90m", "1.5h", "2d", "1d 2h", "30s"
///
/// Returns None if input is invalid.
pub fn parse_duration(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Negative durations are rejected
    if s.starts_with('-') {
        return None;
    }

    // If pure integer number
    if let Ok(n) = s.parse::<i64>() {
        return if n >= 0 { Some(n) } else { None };
    }

    // If colon format: h:mm or h:mm:ss
    if s.contains(':') {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() == 2 {
            let h = parts[0].trim().parse::<i64>().ok()?;
            let m = parts[1].trim().parse::<i64>().ok()?;
            if h >= 0 && (0..=59).contains(&m) {
                return Some(h * 3600 + m * 60);
            }
        } else if parts.len() == 3 {
            let h = parts[0].trim().parse::<i64>().ok()?;
            let m = parts[1].trim().parse::<i64>().ok()?;
            let sec = parts[2].trim().parse::<i64>().ok()?;
            if h >= 0 && (0..=59).contains(&m) && (0..=59).contains(&sec) {
                return Some(h * 3600 + m * 60 + sec);
            }
        }
        return None;
    }

    // Parse tokens with units: e.g. "1.5h", "90m", "2d", "1d 2h", "30s"
    let mut total_secs: f64 = 0.0;
    let mut matched_any = false;

    // Tokenize by space or boundary between unit and number
    let mut chars = s.chars().peekable();
    while chars.peek().is_some() {
        // Skip whitespace
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() {
                chars.next();
            } else {
                break;
            }
        }
        if chars.peek().is_none() {
            break;
        }

        // Collect number (including decimal)
        let mut num_str = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() || c == '.' {
                num_str.push(c);
                chars.next();
            } else {
                break;
            }
        }

        if num_str.is_empty() {
            return None;
        }
        let num: f64 = num_str.parse().ok()?;

        // Skip whitespace before unit
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() {
                chars.next();
            } else {
                break;
            }
        }

        // Collect unit
        let mut unit_str = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_alphabetic() {
                unit_str.push(c.to_ascii_lowercase());
                chars.next();
            } else {
                break;
            }
        }

        if unit_str.is_empty() {
            return None;
        }

        let mult = match unit_str.as_str() {
            "s" | "sec" | "secs" | "second" | "seconds" => 1.0,
            "m" | "min" | "mins" | "minute" | "minutes" => 60.0,
            "h" | "hr" | "hrs" | "hour" | "hours" => 3600.0,
            "d" | "day" | "days" => 86400.0,
            _ => return None,
        };

        total_secs += num * mult;
        matched_any = true;
    }

    if matched_any && total_secs >= 0.0 {
        Some(total_secs.round() as i64)
    } else {
        None
    }
}

/// Formats whole seconds into standard string representation according to the format.
pub fn format_duration(secs: i64, format: &str) -> String {
    let secs = secs.max(0);
    match format {
        "h:mm:ss" => {
            let h = secs / 3600;
            let m = (secs % 3600) / 60;
            let s = secs % 60;
            format!("{h}:{m:02}:{s:02}")
        }
        "d h" => {
            let d = secs / 86400;
            let h = (secs % 86400) / 3600;
            format!("{d}d {h}h")
        }
        "days" => {
            if secs % 86400 == 0 {
                let d = secs / 86400;
                format!("{d} {}", if d == 1 { "day" } else { "days" })
            } else {
                let d = (secs as f64) / 86400.0;
                let formatted = format!("{:.2}", d);
                let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
                format!("{trimmed} days")
            }
        }
        // default "h:mm"
        _ => {
            let h = secs / 3600;
            let m = (secs % 3600) / 60;
            format!("{h}:{m:02}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse() {
        assert_eq!(parse_duration("90m"), Some(5400));
        assert_eq!(parse_duration("1.5h"), Some(5400));
        assert_eq!(parse_duration("1:30"), Some(5400));
        assert_eq!(parse_duration("2d"), Some(172800));
        assert_eq!(parse_duration("1d 2h"), Some(93600));
        assert_eq!(parse_duration("5400"), Some(5400));
        assert_eq!(parse_duration("invalid"), None);
        assert_eq!(parse_duration("-5m"), None);
    }

    #[test]
    fn test_format() {
        assert_eq!(format_duration(5400, "h:mm"), "1:30");
        assert_eq!(format_duration(5400, "h:mm:ss"), "1:30:00");
        assert_eq!(format_duration(5400, "d h"), "0d 1h");
        assert_eq!(format_duration(93600, "d h"), "1d 2h");
        assert_eq!(format_duration(172800, "days"), "2 days");
        assert_eq!(format_duration(86400, "days"), "1 day");
        assert_eq!(format_duration(129600, "days"), "1.5 days");
    }
}
