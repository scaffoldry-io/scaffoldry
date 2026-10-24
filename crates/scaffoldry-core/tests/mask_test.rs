//! Access model phase 12: a closed list of masks, one pure function.

use scaffoldry_core::mask::{mask, Mask};
use serde_json::{json, Value};

fn all(kind: Mask, cases: &[(Value, Value)]) {
    for (input, want) in cases {
        assert_eq!(&mask(input, kind), want, "{kind:?} of {input}");
    }
}

// 1. Each mask kind on a table of values, including empty values, short strings for `last4`, and dates at the edges of a year.
#[test]
fn hide_shows_nothing() {
    all(Mask::Hide, &[(json!("anything"), Value::Null), (json!(42), Value::Null), (Value::Null, Value::Null), (json!(""), Value::Null)]);
}

#[test]
fn last4_keeps_the_last_four_characters_of_a_longer_value_and_nothing_of_a_short_one() {
    all(
        Mask::Last4,
        &[
            (json!("123456789"), json!("*****6789")),
            (json!("12345"), json!("*2345")),
            (json!("1234"), json!("****")),
            (json!("12"), json!("**")),
            (json!(""), json!("")),
            (Value::Null, Value::Null),
            (json!(123456789), json!("*****6789")),
            (json!("ünï-1234"), json!("****1234")),
        ],
    );
}

#[test]
fn initials_keep_the_first_letter_of_each_word() {
    all(
        Mask::Initials,
        &[
            (json!("Ada Lovelace"), json!("A. L.")),
            (json!("  grace   brewster murray hopper "), json!("G. B. M. H.")),
            (json!("Madonna"), json!("M.")),
            (json!("Émile Zola"), json!("É. Z.")),
            (json!(""), json!("")),
            (Value::Null, Value::Null),
        ],
    );
}

#[test]
fn year_and_month_reduce_a_date_and_fail_closed_on_anything_else() {
    all(
        Mask::Year,
        &[
            (json!("2004-01-01"), json!("2004")),
            (json!("2004-12-31"), json!("2004")),
            (json!("2004-12-31T23:59:59Z"), json!("2004")),
            (json!("2004-12-31T23:59:59-08:00"), json!("2004")),
            (json!("2004-02-30"), Value::Null),
            (json!("not a date"), Value::Null),
            (json!(""), json!("")),
            (Value::Null, Value::Null),
        ],
    );
    all(
        Mask::Month,
        &[(json!("2004-01-01"), json!("2004-01")), (json!("2004-12-31T23:59:59Z"), json!("2004-12")), (json!("2004-13-01"), Value::Null), (json!(""), json!(""))],
    );
}

#[test]
fn band_reduces_a_number_to_the_band_it_is_in() {
    let band = Mask::Band { width: 10_000 };
    all(
        band,
        &[
            (json!(0), json!("0 to under 10000")),
            (json!(9_999), json!("0 to under 10000")),
            (json!(10_000), json!("10000 to under 20000")),
            (json!(55_250.5), json!("50000 to under 60000")),
            (json!(-1), json!("-10000 to under 0")),
            (json!("61000"), json!("60000 to under 70000")),
            (json!("n/a"), Value::Null),
            (json!(""), json!("")),
            (Value::Null, Value::Null),
        ],
    );
    all(Mask::Band { width: 0 }, &[(json!(5), Value::Null)]);
}

// 7. Every mask has a sentence.
#[test]
fn every_mask_has_a_sentence() {
    for (kind, want) in [
        (Mask::Hide, "Not shown."),
        (Mask::Last4, "Shown as the last four characters only."),
        (Mask::Initials, "Shown as initials only."),
        (Mask::Year, "Shown as the year only."),
        (Mask::Month, "Shown as the year and month only."),
        (Mask::Band { width: 5000 }, "Shown as a band of 5000."),
    ] {
        assert_eq!(kind.sentence(), want);
    }
}

// 3. A mask that shows more than the category's default is weaker.
#[test]
fn a_mask_is_weaker_when_it_shows_more_than_the_default() {
    assert!(!Mask::Hide.weaker_than(Mask::Hide));
    assert!(Mask::Year.weaker_than(Mask::Hide), "anything shows more than hide");
    assert!(!Mask::Hide.weaker_than(Mask::Year), "hide shows less than anything");
    assert!(Mask::Month.weaker_than(Mask::Year), "the month is more than the year");
    assert!(!Mask::Year.weaker_than(Mask::Month));
    assert!(Mask::Band { width: 1000 }.weaker_than(Mask::Band { width: 10_000 }), "a narrower band shows more");
    assert!(!Mask::Band { width: 10_000 }.weaker_than(Mask::Band { width: 1000 }));
    assert!(Mask::Last4.weaker_than(Mask::Initials), "two different masks are not comparable, so the change is flagged");
    assert!(!Mask::Last4.weaker_than(Mask::Last4));
}

#[test]
fn a_mask_reads_and_writes_as_json() {
    for (text, kind) in [(r#"{"kind":"hide"}"#, Mask::Hide), (r#"{"kind":"year"}"#, Mask::Year), (r#"{"kind":"band","width":5000}"#, Mask::Band { width: 5000 })] {
        assert_eq!(serde_json::from_str::<Mask>(text).unwrap(), kind);
        assert_eq!(serde_json::to_string(&kind).unwrap(), text);
    }
    assert!(serde_json::from_str::<Mask>(r#"{"kind":"rot13"}"#).is_err(), "the list is closed");
}
