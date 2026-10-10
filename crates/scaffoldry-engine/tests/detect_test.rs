//! Sensitive content phase 1: the detectors. They are pure: text in, offsets out.

use scaffoldry_engine::detect::{scan, validate_shape, Action, DetectorKind, DetectorSpec};
use std::time::Instant;

fn spec(kind: DetectorKind) -> DetectorSpec {
    DetectorSpec::new("test", kind)
}

fn hits(d: &DetectorSpec, text: &str) -> usize {
    scan(text, d).len()
}

fn shape(shape: &str, checksum: Option<&str>) -> DetectorSpec {
    let mut d = spec(DetectorKind::Shape);
    d.shape = Some(shape.to_string());
    d.checksum = checksum.map(str::to_string);
    d
}

// 1. A card number passes the Luhn check.
#[test]
fn a_card_number_must_pass_the_luhn_check() {
    let d = spec(DetectorKind::PaymentCard);
    assert_eq!(hits(&d, "pay with 4111 1111 1111 1111 today"), 1);
    assert_eq!(hits(&d, "4111 1111 1111 1112"), 0, "fails Luhn");
    assert_eq!(hits(&d, "4111-1111-1111-1111"), 1, "hyphens separate groups");
    assert_eq!(hits(&d, "4111111111111111"), 1, "no separators");
    assert_eq!(hits(&d, "4111  1111  1111  1111"), 0, "only a single space between groups");
    assert_eq!(hits(&d, "411111111111"), 0, "12 digits is too short");
    assert_eq!(hits(&d, "order 12345678"), 0);
}

// 2. SSN by shape and by the SSA assignment rules.
#[test]
fn an_ssn_follows_the_ssa_assignment_rules() {
    let d = spec(DetectorKind::UsSsn);
    assert_eq!(hits(&d, "ssn 123-45-6789 on file"), 1);
    assert_eq!(hits(&d, "123 45 6789"), 1, "spaces also separate");
    for bad in ["000-12-3456", "666-12-3456", "912-12-3456", "999-12-3456", "123-00-4567", "123-45-0000"] {
        assert_eq!(hits(&d, bad), 0, "{bad} cannot be an SSN");
    }
    assert_eq!(hits(&d, "123-45 6789"), 0, "mixed separators are not an SSN");
    assert_eq!(hits(&d, "9123-45-6789"), 0, "not inside a longer number");
}

// 3. Nine unseparated digits only when allowed.
#[test]
fn nine_digits_without_separators_match_only_when_allowed() {
    let mut d = spec(DetectorKind::UsSsn);
    assert_eq!(hits(&d, "order 123456789"), 0);
    d.allow_unseparated = true;
    assert_eq!(hits(&d, "order 123456789"), 1);
    assert_eq!(hits(&d, "order 1234567890"), 0, "ten digits are not nine");
    assert_eq!(hits(&d, "order 000456789"), 0, "the SSA rules still hold");
}

#[test]
fn an_email_has_a_local_part_an_at_and_a_dotted_domain() {
    let d = spec(DetectorKind::Email);
    assert_eq!(hits(&d, "write to jo.lee+x@state.edu now"), 1);
    assert_eq!(hits(&d, "a@b.edu and c@d.org"), 2);
    assert_eq!(hits(&d, "no-at-sign.edu"), 0);
    assert_eq!(hits(&d, "name@localhost"), 0, "the domain needs a dot");
}

// 4. A custom shape.
#[test]
fn a_shape_marks_digits_letters_and_literals() {
    let d = shape("AA-####", None);
    assert_eq!(hits(&d, "course CS-1042 opens"), 1);
    assert_eq!(hits(&d, "course C-1042 opens"), 0);
    assert_eq!(hits(&shape("?#?", None), "a1b x2y"), 2);
    assert_eq!(hits(&shape("?#?", None), "a1b 2c3"), 1, "2c3 has a letter where a digit is required");
    assert_eq!(hits(&shape("####", Some("luhn")), "1234 0018"), 1, "only 0018 passes Luhn");
}

#[test]
fn a_shape_over_64_characters_is_rejected() {
    assert!(validate_shape(&"#".repeat(64)).is_ok());
    assert!(validate_shape(&"#".repeat(65)).is_err());
    assert!(validate_shape("").is_err(), "an empty shape matches everything");
}

// 5. Time grows with the text.
#[test]
fn a_million_characters_scan_in_time_proportional_to_the_length() {
    let unit = "word 4111 1111 1111 1111 and 123-45-6789 then a@b.edu CS-1042 ";
    let build = |chars: usize| unit.repeat(chars / unit.len() + 1);
    let (small, large) = (build(250_000), build(1_000_000));
    assert!(large.len() >= 1_000_000);

    let mut ssn_unsep = spec(DetectorKind::UsSsn);
    ssn_unsep.allow_unseparated = true;
    let detectors = [
        spec(DetectorKind::UsSsn),
        ssn_unsep,
        spec(DetectorKind::PaymentCard),
        spec(DetectorKind::Email),
        shape("AA-####", None),
    ];
    for d in &detectors {
        let t = Instant::now();
        let n_small = scan(&small, d).len();
        let small_time = t.elapsed();
        let t = Instant::now();
        let n_large = scan(&large, d).len();
        let large_time = t.elapsed();
        println!(
            "detect: {:?} found {n_small} in {} chars ({small_time:?}) and {n_large} in {} chars ({large_time:?})",
            d.kind,
            small.len(),
            large.len()
        );
        assert!(n_large > n_small, "the larger text has more matches");
        assert!(large_time.as_millis() < 3000, "{:?} took {large_time:?} on a million characters", d.kind);
        // 4x the text. Linear is about 4x. Quadratic would be about 16x.
        let ratio = large_time.as_secs_f64() / small_time.as_secs_f64().max(0.000_001);
        assert!(ratio < 10.0, "{:?}: 4x the text took {ratio:.1}x the time", d.kind);
    }
}

#[test]
fn text_built_to_stall_a_matcher_does_not() {
    // Long runs that almost match, repeated.
    let digits = "1".repeat(1_000_000);
    let separated = "1-".repeat(500_000);
    let ats = "a@".repeat(500_000);
    for kind in [DetectorKind::UsSsn, DetectorKind::PaymentCard, DetectorKind::Email] {
        let d = spec(kind);
        for text in [&digits, &separated, &ats] {
            let t = Instant::now();
            let _ = scan(text, &d);
            assert!(t.elapsed().as_secs() < 3, "{kind:?} stalled on {} characters", text.len());
        }
    }
}

// 9. What a detector returns holds offsets, never the matched text.
#[test]
fn detect_output_holds_offsets_and_never_the_matched_value() {
    let planted = "123-45-6789";
    let text = format!("filler {planted} filler");
    let found = scan(&text, &spec(DetectorKind::UsSsn));
    assert_eq!(found.len(), 1);
    assert_eq!(&text[found[0].start..found[0].end], planted, "the offsets locate it in the caller's text");

    let debug = format!("{found:?}");
    let json = serde_json::to_string(&found).unwrap();
    for rendered in [debug, json] {
        assert!(!rendered.contains(planted), "the match leaked the value: {rendered}");
        assert!(!rendered.contains("6789"), "or part of it: {rendered}");
    }
}

#[test]
fn the_default_action_is_flag() {
    assert_eq!(spec(DetectorKind::UsSsn).action, Action::Flag);
    assert!(spec(DetectorKind::UsSsn).enabled);
}
