//! Detectors for sensitive content. Pure: text in, offsets out, no input or output.
//!
//! A match is a pair of byte offsets into the caller's text. It never holds the matched value,
//! because copying a value out would put the sensitive data in a second place.
//!
//! Standards used: SSA number assignment rules (an SSN never has area 000, 666 or 900 to 999,
//! group 00, or serial 0000), and ISO/IEC 7812-1 (a card number is 13 to 19 digits that pass the
//! Luhn check). Every detector makes one left-to-right pass, so time grows with the text.

use serde::{Deserialize, Serialize};

/// The longest custom shape, in characters.
pub const MAX_SHAPE_LEN: usize = 64;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DetectorKind {
    UsSsn,
    PaymentCard,
    Email,
    Shape,
}

/// What a detector does on a match outside an authorized store. All three are always available.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Flag,
    Warn,
    Block,
}

fn default_action() -> Action {
    Action::Flag
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DetectorSpec {
    pub id: String,
    pub kind: DetectorKind,
    #[serde(default = "default_action")]
    pub action: Action,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// For `shape`: `#` a digit, `A` a letter, `?` a letter or digit, anything else literal.
    #[serde(default)]
    pub shape: Option<String>,
    /// For `shape`: `luhn` requires the digits of a match to pass the Luhn check.
    #[serde(default)]
    pub checksum: Option<String>,
    /// For `us_ssn`: also match nine digits with no separators.
    #[serde(default)]
    pub allow_unseparated: bool,
}

impl DetectorSpec {
    pub fn new(id: impl Into<String>, kind: DetectorKind) -> Self {
        Self {
            id: id.into(),
            kind,
            action: Action::Flag,
            enabled: true,
            shape: None,
            checksum: None,
            allow_unseparated: false,
        }
    }
}

/// Where a match sits in the scanned text, as byte offsets. `text[start..end]` is the match.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
}

/// Finds every match of one detector. Whether the detector is enabled is the caller's question.
pub fn scan(text: &str, detector: &DetectorSpec) -> Vec<Match> {
    let bytes = text.as_bytes();
    match detector.kind {
        DetectorKind::UsSsn => scan_ssn(bytes, detector.allow_unseparated),
        DetectorKind::PaymentCard => scan_card(bytes),
        DetectorKind::Email => scan_email(bytes),
        DetectorKind::Shape => match detector.shape.as_deref() {
            Some(shape) if validate_shape(shape).is_ok() => {
                scan_shape(bytes, shape.as_bytes(), detector.checksum.as_deref() == Some("luhn"))
            }
            _ => Vec::new(),
        },
    }
}

/// Checks a custom shape: 1 to 64 ASCII characters.
pub fn validate_shape(shape: &str) -> Result<(), String> {
    if shape.is_empty() {
        return Err("a shape cannot be empty, because it would match everywhere".to_string());
    }
    if shape.chars().count() > MAX_SHAPE_LEN {
        return Err(format!("a shape has at most {MAX_SHAPE_LEN} characters"));
    }
    if !shape.is_ascii() {
        return Err("a shape uses ASCII characters only".to_string());
    }
    Ok(())
}

/// The Luhn check over the ASCII digits of `digits`.
pub fn luhn_valid(digits: &[u8]) -> bool {
    let mut sum = 0u32;
    let mut double = false;
    let mut count = 0;
    for &b in digits.iter().rev().filter(|b| b.is_ascii_digit()) {
        let mut d = u32::from(b - b'0');
        if double {
            d *= 2;
            if d > 9 {
                d -= 9;
            }
        }
        sum += d;
        double = !double;
        count += 1;
    }
    count > 1 && sum.is_multiple_of(10)
}

fn is_digit_at(b: &[u8], i: usize) -> bool {
    b.get(i).is_some_and(u8::is_ascii_digit)
}

/// The SSA assignment rules for the three parts of a number.
fn ssn_parts_valid(area: &[u8], group: &[u8], serial: &[u8]) -> bool {
    let area_n = area.iter().fold(0u32, |n, d| n * 10 + u32::from(d - b'0'));
    area_n != 0 && area_n != 666 && area_n < 900 && group != b"00" && serial != b"0000"
}

fn scan_ssn(b: &[u8], allow_unseparated: bool) -> Vec<Match> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        // A match starts at the start of a run of digits, never inside a longer number.
        if !b[i].is_ascii_digit() || (i > 0 && b[i - 1].is_ascii_digit()) {
            i += 1;
            continue;
        }
        let separated = i + 11 <= b.len()
            && b[i..i + 3].iter().all(u8::is_ascii_digit)
            && (b[i + 3] == b'-' || b[i + 3] == b' ')
            && b[i + 6] == b[i + 3]
            && b[i + 4..i + 6].iter().all(u8::is_ascii_digit)
            && b[i + 7..i + 11].iter().all(u8::is_ascii_digit)
            && !is_digit_at(b, i + 11)
            && ssn_parts_valid(&b[i..i + 3], &b[i + 4..i + 6], &b[i + 7..i + 11]);
        if separated {
            out.push(Match { start: i, end: i + 11 });
            i += 11;
            continue;
        }
        if allow_unseparated
            && i + 9 <= b.len()
            && b[i..i + 9].iter().all(u8::is_ascii_digit)
            && !is_digit_at(b, i + 9)
            && ssn_parts_valid(&b[i..i + 3], &b[i + 3..i + 5], &b[i + 5..i + 9])
        {
            out.push(Match { start: i, end: i + 9 });
            i += 9;
            continue;
        }
        // Nothing starts here. Skip the rest of this run of digits.
        while is_digit_at(b, i) {
            i += 1;
        }
    }
    out
}

fn scan_card(b: &[u8]) -> Vec<Match> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        // One run: digits, with a single space or hyphen allowed between digits.
        let start = i;
        let mut end = i;
        let mut digits = 0usize;
        let mut j = i;
        while j < b.len() {
            if b[j].is_ascii_digit() {
                digits += 1;
                end = j + 1;
                j += 1;
            } else if (b[j] == b' ' || b[j] == b'-') && is_digit_at(b, j + 1) && end == j {
                j += 1;
            } else {
                break;
            }
        }
        if (13..=19).contains(&digits) && luhn_valid(&b[start..end]) {
            out.push(Match { start, end });
        }
        i = end.max(i + 1);
    }
    out
}

fn is_local_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'%' | b'+' | b'-')
}

fn is_domain_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'.' || c == b'-'
}

fn scan_email(b: &[u8]) -> Vec<Match> {
    let mut out = Vec::new();
    let mut floor = 0; // nothing to the left of the last match is read again
    for at in 0..b.len() {
        if b[at] != b'@' || at < floor {
            continue;
        }
        let mut start = at;
        while start > floor && is_local_char(b[start - 1]) {
            start -= 1;
        }
        let mut end = at + 1;
        while end < b.len() && is_domain_char(b[end]) {
            end += 1;
        }
        // A domain does not end in a dot or a hyphen.
        while end > at + 1 && matches!(b[end - 1], b'.' | b'-') {
            end -= 1;
        }
        let domain = &b[at + 1..end];
        let tld_ok = domain
            .iter()
            .rposition(|&c| c == b'.')
            .is_some_and(|dot| domain.len() - dot > 2 && domain[dot + 1..].iter().all(u8::is_ascii_alphabetic));
        if start < at && !domain.is_empty() && domain[0] != b'.' && tld_ok {
            out.push(Match { start, end });
            floor = end;
        }
    }
    out
}

fn shape_matches_at(text: &[u8], shape: &[u8]) -> bool {
    text.len() >= shape.len()
        && shape.iter().zip(text).all(|(&s, &c)| match s {
            b'#' => c.is_ascii_digit(),
            b'A' => c.is_ascii_alphabetic(),
            b'?' => c.is_ascii_alphanumeric(),
            literal => c == literal,
        })
}

fn scan_shape(b: &[u8], shape: &[u8], luhn: bool) -> Vec<Match> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + shape.len() <= b.len() {
        let window = &b[i..i + shape.len()];
        if shape_matches_at(window, shape) && (!luhn || luhn_valid(window)) {
            out.push(Match { start: i, end: i + shape.len() });
            i += shape.len();
        } else {
            i += 1;
        }
    }
    out
}
