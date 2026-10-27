//! Decision authority: who must say yes to a change. A change has a tier. A matrix says what each tier needs.
//!
//! Standard: FIPS 199 names the tiers (Low, Moderate, High). NIST SP 800-53 CM-3 (change control), CM-4 (impact
//! analysis), AC-5 (separation of duties) and RA-3 (the assessment a matrix records) are the controls it implements.
//!
//! `required` is pure. It does no I/O. The tables it reads are small and are passed in.

use crate::approver::ApproverSpec;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Tier {
    Low,
    Moderate,
    High,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Low => "Low",
            Tier::Moderate => "Moderate",
            Tier::High => "High",
        }
    }

    pub fn parse(s: &str) -> Option<Tier> {
        match s {
            "Low" => Some(Tier::Low),
            "Moderate" => Some(Tier::Moderate),
            "High" => Some(Tier::High),
            _ => None,
        }
    }
}

/// In order of strictness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Path {
    /// An owner or admin of the workspace decides, and may decide their own change.
    Direct,
    /// An owner or admin other than the proposer.
    SecondPerson,
    /// The people a named approver spec resolves to, other than the proposer.
    Approver,
}

impl Path {
    pub fn as_str(self) -> &'static str {
        match self {
            Path::Direct => "direct",
            Path::SecondPerson => "second_person",
            Path::Approver => "approver",
        }
    }

    pub fn parse(s: &str) -> Option<Path> {
        match s {
            "direct" => Some(Path::Direct),
            "second_person" => Some(Path::SecondPerson),
            "approver" => Some(Path::Approver),
            _ => None,
        }
    }
}

/// The checks that need a second person when they fail, and the sentence that says why. The floor: no table lowers it.
/// The server's `FLAGGED_CHECKS` is this list, and a test keeps the two equal.
pub const FLAGGED: &[(&str, &str)] = &[
    ("sensitivity", "The change adds or exposes a field that holds sensitive data."),
    ("data_loss", "The change removes data or a field that holds data."),
    ("page_sensitivity", "A custom page reads a field that holds sensitive data."),
    ("page_dependencies", "A custom page depends on something the change removes or alters."),
    ("rule_effects", "An automation rule would change records or send data."),
    ("access_loosened", "The change gives more people access than before."),
    ("coordinates_widened", "The change lowers an object's sensitivity or moves it up the unit tree."),
    ("mask_weakened", "The change shows more of a masked field than before."),
    ("request_write", "The request asks for a role that can change data."),
    ("request_sensitive", "The request asks for access to sensitive data."),
    ("request_outside_unit", "The request is for something outside the requester's own unit."),
];

/// What would avoid each flagged check, as one sentence. `None` where there is honestly nothing to change but the proposal
/// itself. A test keeps this list in step with `FLAGGED`.
pub const REMEDIES: &[(&str, Option<&str>)] = &[
    ("sensitivity", Some("Leave the sensitive field out of this change, or mark it not sensitive if it holds no sensitive data.")),
    ("data_loss", Some("Hide the field instead of removing it.")),
    ("page_sensitivity", Some("Have the page read only fields that are not sensitive.")),
    ("page_dependencies", Some("Change the page in the same proposal, or keep what it depends on.")),
    ("rule_effects", Some("Make the rule notify only, without changing records or sending data.")),
    ("access_loosened", Some("Keep access as it is, or narrow it.")),
    ("coordinates_widened", None),
    ("mask_weakened", None),
    ("request_write", Some("Ask for the viewer role if reading is enough.")),
    ("request_sensitive", None),
    ("request_outside_unit", None),
];

/// What would avoid the flagged check `name`, if anything.
pub fn remedy_for(name: &str) -> Option<&'static str> {
    REMEDIES.iter().find(|(n, _)| *n == name).and_then(|(_, r)| *r)
}

/// What every flagged check raises to, before any table.
pub const BUILT_IN_CHECK_TIER: Tier = Tier::Moderate;

pub fn built_in_path(tier: Tier) -> Path {
    match tier {
        Tier::Low => Path::Direct,
        Tier::Moderate | Tier::High => Path::SecondPerson,
    }
}

/// A unit's own tier for a check. It can only raise the built-in one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalRow {
    pub organization_id: Uuid,
    pub check_name: String,
    pub tier: Tier,
}

/// A unit's requirement for a tier. It can only tighten the built-in path.
#[derive(Debug, Clone, PartialEq)]
pub struct MatrixRow {
    pub organization_id: Uuid,
    pub tier: Tier,
    pub path: Path,
    pub approver: Option<ApproverSpec>,
    pub control_id: Option<String>,
    pub library_policy_key: Option<String>,
    pub library_rule_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reason {
    pub check: Option<String>,
    pub organization_id: Option<Uuid>,
    /// One sentence. Where a unit is named it reads `{unit}`: `Requirement::sentences` puts the name there.
    pub sentence: String,
    pub control_id: Option<String>,
    pub library_rule: Option<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Requirement {
    pub tier: Tier,
    pub path: Path,
    pub approver: Option<ApproverSpec>,
    /// One entry per raised check and per row that tightened the answer.
    pub reasons: Vec<Reason>,
}

impl Requirement {
    /// What the path asks for, in one sentence.
    pub fn path_sentence(&self) -> String {
        format!("This needs {}.", path_words(self.path))
    }

    /// The reasons as sentences, with each `{unit}` replaced by the unit's name.
    pub fn sentences(&self, name_of: &dyn Fn(Uuid) -> String) -> Vec<String> {
        self.reasons
            .iter()
            .map(|r| match r.organization_id {
                Some(id) => r.sentence.replace("{unit}", &name_of(id)),
                None => r.sentence.clone(),
            })
            .collect()
    }
}

fn path_words(path: Path) -> &'static str {
    match path {
        Path::Direct => "an owner or admin of the workspace",
        Path::SecondPerson => "a second owner or admin, other than the proposer",
        Path::Approver => "a named approver, other than the proposer",
    }
}

/// What a change needs.
///
/// `raised` holds the names of the flagged checks that failed. `unit_path` is the workspace's unit first, then each unit
/// above it, ending at the root. The answer is never looser than the built-in one: the tier is the highest and the path
/// is the strictest of the built-in value and every row on the path.
pub fn required(raised: &[String], unit_path: &[Uuid], signals: &[SignalRow], rows: &[MatrixRow]) -> Requirement {
    let on_path = |org: &Uuid| unit_path.contains(org);
    let mut reasons = Vec::new();

    let mut tier = Tier::Low;
    for name in raised {
        let Some((_, sentence)) = FLAGGED.iter().find(|(n, _)| n == name) else { continue };
        let check_tier = signals
            .iter()
            .filter(|s| s.check_name == *name && on_path(&s.organization_id))
            .map(|s| s.tier)
            .fold(BUILT_IN_CHECK_TIER, Tier::max);
        tier = tier.max(check_tier);
        reasons.push(Reason { check: Some(name.clone()), organization_id: None, sentence: (*sentence).to_string(), control_id: None, library_rule: None });
    }

    let built_in = built_in_path(tier);
    let applies = |r: &&MatrixRow| on_path(&r.organization_id) && r.tier <= tier;
    let path = rows.iter().filter(applies).map(|r| r.path).fold(built_in, Path::max);

    // Nearest means the workspace's own unit first. At one unit, the row for the highest tier that applies wins.
    let approver = if path == Path::Approver {
        unit_path.iter().find_map(|unit| {
            rows.iter()
                .filter(|r| r.organization_id == *unit && r.path == Path::Approver && r.tier <= tier)
                .max_by_key(|r| r.tier)
                .and_then(|r| r.approver.clone())
        })
    } else {
        None
    };

    // A row is named when it made the answer stricter than the built-in one.
    for unit in unit_path {
        for r in rows.iter().filter(|r| r.organization_id == *unit && r.tier <= tier && r.path > built_in) {
            reasons.push(Reason {
                check: None,
                organization_id: Some(*unit),
                sentence: format!("{{unit}} requires {} for {} changes.", path_words(r.path), r.tier.as_str()),
                control_id: r.control_id.clone(),
                library_rule: r.library_policy_key.clone().zip(r.library_rule_key.clone()),
            });
        }
    }

    Requirement { tier, path, approver, reasons }
}

/// What one tier needs at a unit, and where that comes from.
#[derive(Debug, Clone, PartialEq)]
pub struct Effective {
    pub path: Path,
    pub approver: Option<ApproverSpec>,
    /// The unit whose row makes the path stricter than the built-in one. `None` when the built-in answer stands.
    pub source: Option<Uuid>,
}

/// The answer for a change of `tier`, as `required` would give it: the built-in path, or the strictest row on the path.
pub fn effective_for_tier(tier: Tier, unit_path: &[Uuid], rows: &[MatrixRow]) -> Effective {
    let built_in = built_in_path(tier);
    let applicable = |r: &&MatrixRow| unit_path.contains(&r.organization_id) && r.tier <= tier;
    let path = rows.iter().filter(applicable).map(|r| r.path).fold(built_in, Path::max);
    if path == built_in {
        return Effective { path, approver: None, source: None };
    }
    // The nearest unit with a row at the winning path.
    let winner = unit_path.iter().find_map(|unit| rows.iter().filter(applicable).filter(|r| r.organization_id == *unit && r.path == path).max_by_key(|r| r.tier));
    Effective { path, approver: winner.and_then(|r| r.approver.clone()), source: winner.map(|r| r.organization_id) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn every_flagged_check_has_a_remedy_or_an_honest_none_and_no_other_check_does() {
        let flagged: Vec<&str> = FLAGGED.iter().map(|(n, _)| *n).collect();
        let remedied: Vec<&str> = REMEDIES.iter().map(|(n, _)| *n).collect();
        assert_eq!(flagged, remedied);
        assert_eq!(remedy_for("coordinates_widened"), None);
        assert!(remedy_for("sensitivity").is_some());
        assert_eq!(remedy_for("nothing"), None);
    }

    #[test]
    fn the_path_is_said_in_one_sentence() {
        let r = required(&raised(&["sensitivity"]), &[], &[], &[]);
        assert_eq!(r.path_sentence(), "This needs a second owner or admin, other than the proposer.");
    }

    fn raised(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn row(org: Uuid, tier: Tier, path: Path, approver: Option<ApproverSpec>) -> MatrixRow {
        MatrixRow { organization_id: org, tier, path, approver, control_id: None, library_policy_key: None, library_rule_key: None }
    }

    fn officer() -> ApproverSpec {
        ApproverSpec::Person { eppn: "privacy@state.edu".to_string() }
    }

    // department, college, root
    fn tree() -> Vec<Uuid> {
        vec![id(3), id(2), id(1)]
    }

    #[test]
    fn nothing_raised_is_low_and_direct() {
        let r = required(&[], &tree(), &[], &[]);
        assert_eq!((r.tier, r.path), (Tier::Low, Path::Direct));
        assert!(r.approver.is_none() && r.reasons.is_empty());
    }

    #[test]
    fn one_flagged_check_is_moderate_and_needs_a_second_person() {
        let r = required(&raised(&["sensitivity"]), &tree(), &[], &[]);
        assert_eq!((r.tier, r.path), (Tier::Moderate, Path::SecondPerson));
        assert_eq!(r.reasons.len(), 1);
        assert_eq!(r.reasons[0].check.as_deref(), Some("sensitivity"));
    }

    #[test]
    fn a_check_that_is_not_flagged_has_no_tier() {
        let r = required(&raised(&["stale"]), &tree(), &[], &[]);
        assert_eq!((r.tier, r.path), (Tier::Low, Path::Direct));
    }

    #[test]
    fn a_signal_at_the_college_raises_a_departments_check_to_high() {
        let signals = [SignalRow { organization_id: id(2), check_name: "sensitivity".into(), tier: Tier::High }];
        assert_eq!(required(&raised(&["sensitivity"]), &tree(), &signals, &[]).tier, Tier::High);
        // A signal on a unit that is not on the path changes nothing.
        let elsewhere = [SignalRow { organization_id: id(9), check_name: "sensitivity".into(), tier: Tier::High }];
        assert_eq!(required(&raised(&["sensitivity"]), &tree(), &elsewhere, &[]).tier, Tier::Moderate);
        // A signal cannot lower the built-in tier.
        let low = [SignalRow { organization_id: id(2), check_name: "sensitivity".into(), tier: Tier::Low }];
        assert_eq!(required(&raised(&["sensitivity"]), &tree(), &low, &[]).tier, Tier::Moderate);
    }

    #[test]
    fn a_requirement_set_for_moderate_also_holds_for_high() {
        let signals = [SignalRow { organization_id: id(2), check_name: "sensitivity".into(), tier: Tier::High }];
        let rows = [row(id(1), Tier::Moderate, Path::Approver, Some(officer()))];
        let r = required(&raised(&["sensitivity"]), &tree(), &signals, &rows);
        assert_eq!((r.tier, r.path), (Tier::High, Path::Approver));
        assert_eq!(r.approver, Some(officer()));
        // A Moderate proposal in any unit is held by it too.
        assert_eq!(required(&raised(&["data_loss"]), &[id(7), id(1)], &[], &rows).path, Path::Approver);
        // A High row does not hold a Moderate proposal.
        let high_only = [row(id(1), Tier::High, Path::Approver, Some(officer()))];
        assert_eq!(required(&raised(&["data_loss"]), &tree(), &[], &high_only).path, Path::SecondPerson);
    }

    #[test]
    fn a_looser_row_changes_nothing() {
        let rows = [row(id(3), Tier::Moderate, Path::Direct, None)];
        let r = required(&raised(&["sensitivity"]), &tree(), &[], &rows);
        assert_eq!(r.path, Path::SecondPerson);
        assert_eq!(r.reasons.len(), 1, "a row that did not tighten anything is not a reason");
        // Not even for a Low proposal.
        let low = [row(id(3), Tier::Low, Path::Direct, None)];
        assert_eq!(required(&[], &tree(), &[], &low).path, Path::Direct);
    }

    #[test]
    fn the_nearer_unit_names_the_approver() {
        let near = ApproverSpec::Person { eppn: "chair@state.edu".to_string() };
        let rows = [row(id(1), Tier::Moderate, Path::Approver, Some(officer())), row(id(3), Tier::Moderate, Path::Approver, Some(near.clone()))];
        assert_eq!(required(&raised(&["sensitivity"]), &tree(), &[], &rows).approver, Some(near));
        // The same rows, with the workspace in a unit that has no row of its own: the root's spec is used.
        assert_eq!(required(&raised(&["sensitivity"]), &[id(4), id(1)], &[], &rows).approver, Some(officer()));
    }

    #[test]
    fn a_row_that_tightened_the_answer_is_a_reason_that_names_its_unit() {
        let mut r = row(id(2), Tier::Moderate, Path::Approver, Some(officer()));
        r.control_id = Some("CM-03".into());
        r.library_policy_key = Some("change".into());
        r.library_rule_key = Some("privacy".into());
        let need = required(&raised(&["sensitivity"]), &tree(), &[], &[r]);
        assert_eq!(need.reasons.len(), 2);
        let unit = need.reasons.iter().find(|x| x.organization_id.is_some()).unwrap();
        assert_eq!(unit.control_id.as_deref(), Some("CM-03"));
        assert_eq!(unit.library_rule, Some(("change".to_string(), "privacy".to_string())));
        let sentences = need.sentences(&|u| if u == id(2) { "College of Science".to_string() } else { "?".to_string() });
        assert!(sentences.iter().any(|s| s.starts_with("College of Science requires a named approver")), "{sentences:?}");
    }

    #[test]
    fn the_effective_answer_for_a_tier_names_where_it_comes_from() {
        let rows = [row(id(1), Tier::Moderate, Path::Approver, Some(officer())), row(id(3), Tier::Moderate, Path::SecondPerson, None)];
        let high = effective_for_tier(Tier::High, &tree(), &rows);
        assert_eq!((high.path, high.source, high.approver), (Path::Approver, Some(id(1)), Some(officer())));
        // Nothing set: the built-in answer, from nowhere.
        let low = effective_for_tier(Tier::Low, &tree(), &rows);
        assert_eq!((low.path, low.source), (Path::Direct, None));
        // A row for a higher tier does not hold a lower one.
        assert_eq!(effective_for_tier(Tier::Moderate, &tree(), &[row(id(1), Tier::High, Path::Approver, Some(officer()))]).path, Path::SecondPerson);
        // Looser than the built-in answer never shows as its source.
        let loose = [row(id(3), Tier::Moderate, Path::Direct, None)];
        assert_eq!(effective_for_tier(Tier::Moderate, &tree(), &loose).source, None);
    }

    #[test]
    fn every_flagged_check_has_a_sentence() {
        for (name, sentence) in FLAGGED {
            assert!(!sentence.trim().is_empty() && sentence.ends_with('.'), "{name}");
            let r = required(&[name.to_string()], &[], &[], &[]);
            assert_eq!(r.reasons[0].sentence, *sentence);
        }
    }

    #[test]
    fn no_table_lowers_the_floor() {
        // Rows of every kind on every unit, all at the loosest setting.
        let rows: Vec<MatrixRow> = tree().iter().flat_map(|u| [Tier::Low, Tier::Moderate, Tier::High].map(|t| row(*u, t, Path::Direct, None))).collect();
        let signals: Vec<SignalRow> = tree().iter().map(|u| SignalRow { organization_id: *u, check_name: "sensitivity".into(), tier: Tier::Low }).collect();
        let r = required(&raised(&["sensitivity"]), &tree(), &signals, &rows);
        assert_eq!((r.tier, r.path), (Tier::Moderate, Path::SecondPerson));
    }
}
