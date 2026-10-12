//! Views phase 2: filter trees, the new operators, and relative dates in the institution's zone.

use chrono::{DateTime, Utc};
use scaffoldry_engine::{
    check_filter, matches_filter, relative_range, resolve_relative_dates, CompoundFilter, FilterClause, FilterConjunction, FilterOperator,
};
use serde_json::json;

fn clause(field: &str, op: FilterOperator, value: &str) -> FilterClause {
    FilterClause { id: format!("{field}-{op:?}"), field_name: field.to_string(), operator: op, value: value.to_string() }
}

fn group(conjunction: FilterConjunction, clauses: Vec<FilterClause>, groups: Vec<CompoundFilter>) -> CompoundFilter {
    CompoundFilter { conjunction, clauses, groups }
}

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

// A nested filter, (a = 1 OR a = 2) AND b > 5, keeps the right rows.
#[test]
fn a_nested_filter_keeps_the_right_rows() {
    let tree = group(
        FilterConjunction::And,
        vec![clause("b", FilterOperator::GreaterThan, "5")],
        vec![group(FilterConjunction::Or, vec![clause("a", FilterOperator::Equals, "1"), clause("a", FilterOperator::Equals, "2")], vec![])],
    );
    assert!(matches_filter(&json!({ "a": 1, "b": 9 }), &tree));
    assert!(matches_filter(&json!({ "a": 2, "b": 6 }), &tree));
    assert!(!matches_filter(&json!({ "a": 3, "b": 9 }), &tree), "a is neither");
    assert!(!matches_filter(&json!({ "a": 1, "b": 5 }), &tree), "b is not above 5");
}

// A group may nest two levels below the root and no more.
#[test]
fn a_filter_nests_two_levels_and_no_more() {
    let leaf = || group(FilterConjunction::And, vec![clause("a", FilterOperator::Equals, "1")], vec![]);
    let two = group(FilterConjunction::And, vec![], vec![group(FilterConjunction::Or, vec![], vec![leaf()])]);
    assert!(check_filter(&two).is_ok());
    let three = group(FilterConjunction::And, vec![], vec![group(FilterConjunction::Or, vec![], vec![group(FilterConjunction::And, vec![], vec![leaf()])])]);
    let err = check_filter(&three).unwrap_err();
    assert!(err.contains("two levels"), "{err}");
}

#[test]
fn in_between_before_and_after_read_values_and_dates() {
    let rec = json!({ "status": "S2", "amount": 15, "due": "2026-03-10T12:00:00Z" });
    assert!(matches_filter(&rec, &group(FilterConjunction::And, vec![clause("status", FilterOperator::In, r#"["s1","s2"]"#)], vec![])));
    assert!(matches_filter(&rec, &group(FilterConjunction::And, vec![clause("status", FilterOperator::In, "s1, s2")], vec![])), "a comma list also reads");
    assert!(!matches_filter(&rec, &group(FilterConjunction::And, vec![clause("status", FilterOperator::In, r#"["s3"]"#)], vec![])));
    assert!(matches_filter(&rec, &group(FilterConjunction::And, vec![clause("amount", FilterOperator::Between, "10..20")], vec![])));
    assert!(matches_filter(&rec, &group(FilterConjunction::And, vec![clause("amount", FilterOperator::Between, "15..15")], vec![])), "both ends are inside");
    assert!(!matches_filter(&rec, &group(FilterConjunction::And, vec![clause("amount", FilterOperator::Between, "16..20")], vec![])));
    assert!(matches_filter(&rec, &group(FilterConjunction::And, vec![clause("due", FilterOperator::Before, "2026-03-11")], vec![])));
    assert!(!matches_filter(&rec, &group(FilterConjunction::And, vec![clause("due", FilterOperator::Before, "2026-03-10")], vec![])));
    assert!(matches_filter(&rec, &group(FilterConjunction::And, vec![clause("due", FilterOperator::After, "2026-03-10")], vec![])));
    assert!(matches_filter(&rec, &group(FilterConjunction::And, vec![clause("due", FilterOperator::Between, "2026-03-10..2026-03-10T23:00:00Z")], vec![])));
}

// last_7_days at 00:30 and at 23:30 in the institution's zone is the same seven days. A calculation in UTC
// would move the window by a day for one of them.
#[test]
fn last_7_days_holds_the_same_days_at_00_30_and_23_30_local_time() {
    let tz = "America/New_York";
    // 2026-03-10 00:30 and 23:30 in New York, which is on daylight time (UTC-4) from 8 March.
    for now in [at("2026-03-10T04:30:00Z"), at("2026-03-11T03:30:00Z")] {
        let (start, end) = relative_range("last_7_days", now, tz).expect("a range");
        assert_eq!(start, at("2026-03-04T05:00:00Z"), "the window opens at midnight on 4 March, local time, which is still on standard time");
        assert_eq!(end, at("2026-03-11T04:00:00Z"), "and closes at the end of 10 March, local time");
    }
    // The day before the window is out, the first day in it is in.
    let (start, _) = relative_range("last_7_days", at("2026-03-10T04:30:00Z"), tz).unwrap();
    assert!(at("2026-03-04T05:00:00Z") >= start && at("2026-03-04T04:59:59Z") < start);
}

#[test]
fn the_other_relative_dates_are_whole_local_days() {
    let tz = "America/New_York";
    let now = at("2026-03-11T15:00:00Z"); // Wednesday 11 March, 11:00 local
    assert_eq!(relative_range("today", now, tz).unwrap(), (at("2026-03-11T04:00:00Z"), at("2026-03-12T04:00:00Z")));
    assert_eq!(relative_range("this_week", now, tz).unwrap(), (at("2026-03-09T04:00:00Z"), at("2026-03-16T04:00:00Z")), "a week opens on Monday");
    assert_eq!(relative_range("last_30_days", now, tz).unwrap().0, at("2026-02-10T05:00:00Z"), "29 days before today, before daylight time began");
    assert_eq!(relative_range("next_30_days", now, tz).unwrap(), (at("2026-03-11T04:00:00Z"), at("2026-04-10T04:00:00Z")));
    assert!(relative_range("yesterday", now, tz).is_none());
}

// A relative value becomes a plain range before the filter is compiled, so both compilers read dates only.
#[test]
fn a_relative_value_is_resolved_into_a_range() {
    let f = group(
        FilterConjunction::And,
        vec![clause("due", FilterOperator::Between, "today")],
        vec![group(FilterConjunction::Or, vec![clause("due", FilterOperator::Between, "2026-01-01..2026-01-31")], vec![])],
    );
    let resolved = resolve_relative_dates(&f, at("2026-03-11T15:00:00Z"), "America/New_York");
    assert_eq!(resolved.clauses[0].value, "2026-03-11T04:00:00.000000Z..2026-03-12T03:59:59.999999Z");
    assert_eq!(resolved.groups[0].clauses[0].value, "2026-01-01..2026-01-31", "a range that was given stays");
}
