//! Filter trees and relative dates (views phase 2).
//!
//! A filter is a group: a conjunction over clauses and nested groups. A group nests two levels below the root.
//! A relative date such as `last_7_days` is resolved into a plain range, on the server, in the institution's
//! zone, before either compiler reads the filter. After that every date in a filter is a date.

use crate::{CompoundFilter, FilterClause, FilterOperator};
use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};
use serde_json::Value;

/// How many levels of group a filter may nest below its root.
pub const MAX_FILTER_DEPTH: usize = 2;
/// How many clauses a filter may hold in all.
pub const MAX_FILTER_CLAUSES: usize = 50;

fn depth(f: &CompoundFilter) -> usize {
    f.groups.iter().map(|g| 1 + depth(g)).max().unwrap_or(0)
}

/// Every clause in the tree, in order.
pub fn clauses_of(f: &CompoundFilter) -> Vec<&FilterClause> {
    let mut out: Vec<&FilterClause> = f.clauses.iter().collect();
    for g in &f.groups {
        out.extend(clauses_of(g));
    }
    out
}

/// Refuses a filter that nests too deep or holds too many clauses.
pub fn check_filter(f: &CompoundFilter) -> Result<(), String> {
    if depth(f) > MAX_FILTER_DEPTH {
        return Err(format!("A filter may nest groups two levels deep. This one nests {}.", depth(f)));
    }
    let n = clauses_of(f).len();
    if n > MAX_FILTER_CLAUSES {
        return Err(format!("A filter may hold {MAX_FILTER_CLAUSES} clauses. This one holds {n}."));
    }
    Ok(())
}

/// True when the filter is what the index compilers read before this phase: one level of clauses, joined with AND
/// (or a single clause), using only the older operators.
pub fn is_flat(f: &CompoundFilter) -> bool {
    f.groups.is_empty()
        && (f.conjunction == crate::FilterConjunction::And || f.clauses.len() <= 1)
        && f.clauses.iter().all(|c| !matches!(c.operator, FilterOperator::In | FilterOperator::Between | FilterOperator::Before | FilterOperator::After))
}

/// The values of an `In` clause: a JSON array of strings or numbers, or else a comma list.
pub fn list_of(value: &str) -> Vec<String> {
    if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(value) {
        return items
            .iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(s.trim().to_string()),
                Value::Number(n) => Some(n.to_string()),
                Value::Bool(b) => Some(b.to_string()),
                _ => None,
            })
            .collect();
    }
    value.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// The two ends of a `Between` value, `low..high`. Both ends are inside.
pub fn range_of(value: &str) -> Option<(&str, &str)> {
    let (a, b) = value.split_once("..")?;
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() || b.is_empty() {
        None
    } else {
        Some((a, b))
    }
}

/// A date or date and time, as an instant.
pub fn instant_of(s: &str) -> Option<DateTime<Utc>> {
    let p = crate::parse_instant(s.trim())?;
    DateTime::parse_from_rfc3339(&p).ok().map(|d| d.with_timezone(&Utc))
}

fn zone(tz: &str) -> chrono_tz::Tz {
    tz.parse().unwrap_or(chrono_tz::UTC)
}

fn midnight(tz: chrono_tz::Tz, day: NaiveDate) -> DateTime<Utc> {
    let local = day.and_hms_opt(0, 0, 0).expect("midnight exists");
    match tz.from_local_datetime(&local) {
        chrono::LocalResult::Single(t) => t.with_timezone(&Utc),
        chrono::LocalResult::Ambiguous(first, _) => first.with_timezone(&Utc),
        // A zone whose day starts in a gap: the first instant after it.
        chrono::LocalResult::None => Utc.from_utc_datetime(&local) + Duration::hours(1),
    }
}

/// The range a relative date names, as whole days in the zone: it opens at the first instant and closes before
/// the last, which is the first instant of the next day. `None` for a word that is not a relative date.
pub fn relative_range(keyword: &str, now: DateTime<Utc>, tz: &str) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let tz = zone(tz);
    let today = now.with_timezone(&tz).date_naive();
    let (first, after) = match keyword.trim() {
        "today" => (today, today + Duration::days(1)),
        "this_week" => {
            let monday = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
            (monday, monday + Duration::days(7))
        }
        "last_7_days" => (today - Duration::days(6), today + Duration::days(1)),
        "last_30_days" => (today - Duration::days(29), today + Duration::days(1)),
        "next_30_days" => (today, today + Duration::days(30)),
        _ => return None,
    };
    Some((midnight(tz, first), midnight(tz, after)))
}

fn micro(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

/// The filter with each relative date in a `Between` clause replaced by the range it names, at `now`.
pub fn resolve_relative_dates(f: &CompoundFilter, now: DateTime<Utc>, tz: &str) -> CompoundFilter {
    let mut out = f.clone();
    resolve_in_place(&mut out, now, tz);
    out
}

fn resolve_in_place(f: &mut CompoundFilter, now: DateTime<Utc>, tz: &str) {
    for c in &mut f.clauses {
        if c.operator == FilterOperator::Between {
            if let Some((start, end)) = relative_range(&c.value, now, tz) {
                c.value = format!("{}..{}", micro(start), micro(end - Duration::microseconds(1)));
            }
        }
    }
    for g in &mut f.groups {
        resolve_in_place(g, now, tz);
    }
}
