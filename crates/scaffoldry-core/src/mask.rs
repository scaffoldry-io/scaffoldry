//! Access model phase 12: masks. A reader who is not cleared for a field may see a safe part of it instead of nothing:
//! the year of a birth date, the last four characters of an identifier, a salary band. The list is closed and the
//! function is pure, so every response applies the same one.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How a value is reduced. `Hide` is what a hidden field has always done.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Mask {
    /// The value is not shown and the field is not in the response.
    #[default]
    Hide,
    /// All but the last four characters replaced with `*`. A value of four or fewer characters is replaced whole.
    Last4,
    /// The first letter of each word, with a full stop after it.
    Initials,
    /// A date reduced to its year.
    Year,
    /// A date reduced to its year and month.
    Month,
    /// A number reduced to the band it falls in. `width` is the band's size.
    Band { width: u64 },
}

/// The value as a reader who is masked sees it. An empty value stays empty: there is nothing to protect, and a blank
/// that turned into a marker would tell the reader a value exists. A value the mask cannot read is hidden, never shown.
pub fn mask(value: &Value, kind: Mask) -> Value {
    if kind == Mask::Hide {
        return Value::Null;
    }
    match value {
        Value::Null => return Value::Null,
        Value::String(s) if s.is_empty() => return value.clone(),
        _ => {}
    }
    match kind {
        Mask::Hide => Value::Null,
        Mask::Last4 => match text_of(value) {
            Some(s) => {
                let chars: Vec<char> = s.chars().collect();
                let keep = if chars.len() > 4 { 4 } else { 0 };
                let shown: String = chars[chars.len() - keep..].iter().collect();
                Value::String(format!("{}{shown}", "*".repeat(chars.len() - keep)))
            }
            None => Value::Null,
        },
        Mask::Initials => match value.as_str() {
            Some(s) => {
                let initials: Vec<String> = s.split_whitespace().filter_map(|w| w.chars().next()).map(|c| format!("{}.", c.to_uppercase())).collect();
                Value::String(initials.join(" "))
            }
            None => Value::Null,
        },
        Mask::Year => date_part(value, false),
        Mask::Month => date_part(value, true),
        Mask::Band { width } => band(value, width),
    }
}

fn text_of(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn days_in(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        _ => 28,
    }
}

/// The year, or the year and month, of a `YYYY-MM-DD` date or an RFC 3339 date-time. The calendar date as written is
/// used: no time zone moves it into another year.
fn date_part(value: &Value, with_month: bool) -> Value {
    let Some(s) = value.as_str() else { return Value::Null };
    let date = s.get(..10).unwrap_or("");
    let digits = |r: std::ops::Range<usize>| date.get(r).filter(|p| p.bytes().all(|b| b.is_ascii_digit())).and_then(|p| p.parse::<u32>().ok());
    let well_formed = date.len() == 10 && date.as_bytes()[4] == b'-' && date.as_bytes()[7] == b'-' && (s.len() == 10 || matches!(s.as_bytes()[10], b'T' | b't' | b' '));
    match (well_formed, digits(0..4), digits(5..7), digits(8..10)) {
        (true, Some(y), Some(m), Some(d)) if (1..=12).contains(&m) && (1..=days_in(y, m)).contains(&d) => {
            Value::String(if with_month { format!("{y:04}-{m:02}") } else { format!("{y:04}") })
        }
        _ => Value::Null,
    }
}

fn band(value: &Value, width: u64) -> Value {
    let n = match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    let (Some(n), true) = (n.filter(|n| n.is_finite()), width > 0) else { return Value::Null };
    let w = width as f64;
    let low = (n / w).floor() * w;
    Value::String(format!("{} to under {}", low as i64, (low + w) as i64))
}

impl Mask {
    /// Whether the field is left out of a response altogether.
    pub fn is_hide(&self) -> bool {
        *self == Mask::Hide
    }

    /// Whether the mask can be applied: a band needs a width.
    pub fn problem(&self) -> Option<&'static str> {
        matches!(self, Mask::Band { width: 0 }).then_some("a band needs a width of at least 1")
    }

    /// Whichever of the two shows less. Two masks that cannot be put in order give `Hide`.
    pub fn stronger(self, other: Mask) -> Mask {
        if self == other || other.weaker_than(self) {
            self
        } else if self.weaker_than(other) && !other.weaker_than(self) {
            other
        } else {
            Mask::Hide
        }
    }

    /// One plain sentence for a reader and for an owner choosing a mask.
    pub fn sentence(&self) -> String {
        match self {
            Mask::Hide => "Not shown.".to_string(),
            Mask::Last4 => "Shown as the last four characters only.".to_string(),
            Mask::Initials => "Shown as initials only.".to_string(),
            Mask::Year => "Shown as the year only.".to_string(),
            Mask::Month => "Shown as the year and month only.".to_string(),
            Mask::Band { width } => format!("Shown as a band of {width}."),
        }
    }

    /// Whether this mask shows more than `other`. `Hide` shows least. A month shows more than a year, and a narrower band
    /// more than a wider one. Two different masks cannot be put in order, so a change from one to the other counts as
    /// showing more, and a person confirms it.
    pub fn weaker_than(self, other: Mask) -> bool {
        match (self, other) {
            (a, b) if a == b => false,
            (Mask::Hide, _) => false,
            (_, Mask::Hide) => true,
            (Mask::Year, Mask::Month) => false,
            (Mask::Band { width: a }, Mask::Band { width: b }) => a < b,
            _ => true,
        }
    }
}
