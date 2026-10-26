//! Access model phase 13: the rules that approve a request without a person.
//!
//! A rule is a closed set of conditions, and every one must hold. Nothing here reads a database. The server gathers the
//! facts of a request and asks `fits`.

use crate::coordinates::{sensitivity_rank, SENSITIVITY_LEVELS};
use crate::reach::Role;
use serde::{Deserialize, Serialize};

/// The longest request any rule or setting allows, in days.
pub const MAX_DAYS: i64 = 365;

/// A visibility that no rule may cover. A request for a restricted workspace always goes to its owner.
pub const NEVER_AUTO: &str = "restricted";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// The highest role the rule approves.
    pub max_role: String,
    /// The highest sensitivity of the object the rule approves.
    pub max_sensitivity: String,
    /// The workspace visibilities the rule covers.
    pub visibilities: Vec<String>,
    /// The object must be in the requester's own unit or below it.
    pub own_unit: bool,
    /// The longest duration the rule approves, in days.
    pub max_days: i64,
    /// When set, the requester must hold one of these affiliations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affiliations: Option<Vec<String>>,
}

/// What is true of one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub role: Role,
    pub sensitivity: String,
    pub visibility: String,
    pub in_own_unit: bool,
    pub days: i64,
    pub affiliations: Vec<String>,
}

/// Whether every condition of `rule` holds for `facts`.
pub fn fits(rule: &Rule, facts: &Facts) -> bool {
    let (Some(max_role), Some(max_rank), Some(rank)) = (Role::parse(&rule.max_role), sensitivity_rank(&rule.max_sensitivity), sensitivity_rank(&facts.sensitivity)) else {
        return false;
    };
    facts.visibility != NEVER_AUTO
        && facts.role <= max_role
        && rank <= max_rank
        && rule.visibilities.contains(&facts.visibility)
        && (!rule.own_unit || facts.in_own_unit)
        && facts.days >= 1
        && facts.days <= rule.max_days
        && rule.affiliations.as_ref().is_none_or(|wanted| wanted.iter().any(|a| facts.affiliations.contains(a)))
}

/// Why a rule may not be saved, one sentence per fault. Empty when it may.
pub fn problems(rule: &Rule) -> Vec<String> {
    let mut out = Vec::new();
    if Role::parse(&rule.max_role).is_none() {
        out.push(format!("max_role '{}' is not one of viewer, commenter, editor, admin, owner.", rule.max_role));
    }
    if sensitivity_rank(&rule.max_sensitivity).is_none() {
        out.push(format!("max_sensitivity '{}' is not one of {}.", rule.max_sensitivity, SENSITIVITY_LEVELS.join(", ")));
    }
    if rule.visibilities.is_empty() {
        out.push("visibilities must name at least one workspace visibility.".to_string());
    }
    for v in &rule.visibilities {
        if v == NEVER_AUTO {
            out.push("A rule can never cover restricted workspaces. A request for one always goes to its owner.".to_string());
        } else if !["departmental", "institutional"].contains(&v.as_str()) {
            out.push(format!("visibility '{v}' is not one of departmental, institutional."));
        }
    }
    if !(1..=MAX_DAYS).contains(&rule.max_days) {
        out.push(format!("max_days must be a whole number of days from 1 to {MAX_DAYS}."));
    }
    if rule.affiliations.as_ref().is_some_and(Vec::is_empty) {
        out.push("affiliations, when given, must name at least one.".to_string());
    }
    out
}

/// The conditions on which `unit` asks for less than `institution` does. Empty when it only adds conditions.
pub fn loosened(institution: &Rule, unit: &Rule) -> Vec<&'static str> {
    let mut out = Vec::new();
    if Role::parse(&unit.max_role) > Role::parse(&institution.max_role) || Role::parse(&unit.max_role).is_none() {
        out.push("max_role");
    }
    if sensitivity_rank(&unit.max_sensitivity) > sensitivity_rank(&institution.max_sensitivity) || sensitivity_rank(&unit.max_sensitivity).is_none() {
        out.push("max_sensitivity");
    }
    if !unit.visibilities.iter().all(|v| institution.visibilities.contains(v)) {
        out.push("visibilities");
    }
    if institution.own_unit && !unit.own_unit {
        out.push("own_unit");
    }
    if unit.max_days > institution.max_days {
        out.push("max_days");
    }
    let affiliations = match (&institution.affiliations, &unit.affiliations) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(a), Some(b)) => b.iter().all(|x| a.contains(x)),
    };
    if !affiliations {
        out.push("affiliations");
    }
    out
}

/// Whether `unit` asks for at least everything `institution` does, so it only adds conditions.
pub fn only_adds_conditions(institution: &Rule, unit: &Rule) -> bool {
    loosened(institution, unit).is_empty()
}

fn list(words: &[String]) -> String {
    match words {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} or {last}", init.join(", ")),
    }
}

/// The rule as a sentence a person can read.
pub fn sentence(rule: &Rule) -> String {
    let article = if rule.max_role.starts_with(['a', 'e', 'i', 'o', 'u']) { "An" } else { "A" };
    let mut s = format!("{article} {} request for an app at {} or below", rule.max_role, rule.max_sensitivity);
    if rule.own_unit {
        s.push_str(", in the requester's own unit or below");
    }
    s.push_str(&format!(", in a workspace whose visibility is {}, for up to {} days", list(&rule.visibilities), rule.max_days));
    if let Some(a) = &rule.affiliations {
        s.push_str(&format!(", from people affiliated as {}", list(a)));
    }
    s.push_str(", is approved automatically.");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_rule() -> Rule {
        Rule { max_role: "viewer".into(), max_sensitivity: "Internal".into(), visibilities: vec!["departmental".into(), "institutional".into()], own_unit: true, max_days: 30, affiliations: None }
    }

    fn facts() -> Facts {
        Facts { role: Role::Viewer, sensitivity: "Internal".into(), visibility: "departmental".into(), in_own_unit: true, days: 14, affiliations: vec!["faculty".into()] }
    }

    #[test]
    fn the_default_rule_fits_a_viewer_request_for_an_internal_app_in_the_own_unit() {
        assert!(fits(&default_rule(), &facts()));
    }

    #[test]
    fn each_condition_alone_can_stop_a_request() {
        let r = default_rule();
        assert!(!fits(&r, &Facts { role: Role::Editor, ..facts() }), "role");
        assert!(!fits(&r, &Facts { sensitivity: "Restricted".into(), ..facts() }), "sensitivity");
        assert!(!fits(&r, &Facts { visibility: "restricted".into(), ..facts() }), "visibility");
        assert!(!fits(&r, &Facts { in_own_unit: false, ..facts() }), "unit");
        assert!(!fits(&r, &Facts { days: 31, ..facts() }), "days");
        assert!(fits(&r, &Facts { days: 30, ..facts() }), "the limit itself fits");
        assert!(fits(&r, &Facts { sensitivity: "Public".into(), ..facts() }), "lower sensitivity fits");
    }

    #[test]
    fn affiliations_limit_a_rule_only_when_set() {
        let mut r = default_rule();
        r.affiliations = Some(vec!["staff".into()]);
        assert!(!fits(&r, &facts()), "a faculty member is not staff");
        assert!(fits(&r, &Facts { affiliations: vec!["staff".into(), "faculty".into()], ..facts() }));
    }

    #[test]
    fn a_rule_that_covers_restricted_is_refused() {
        let mut r = default_rule();
        r.visibilities.push("restricted".into());
        let p = problems(&r);
        assert!(p.iter().any(|s| s.contains("restricted")), "{p:?}");
        assert!(problems(&default_rule()).is_empty());
    }

    #[test]
    fn a_rule_with_a_value_off_its_ladder_is_refused() {
        let mut r = default_rule();
        r.max_role = "emperor".into();
        r.max_sensitivity = "Secret".into();
        r.max_days = 0;
        r.visibilities.clear();
        assert_eq!(problems(&r).len(), 4, "{:?}", problems(&r));
        assert!(problems(&Rule { max_days: MAX_DAYS + 1, ..default_rule() }).iter().any(|s| s.contains("days")));
    }

    #[test]
    fn a_unit_rule_may_add_conditions_and_may_not_remove_one() {
        let inst = default_rule();
        assert!(only_adds_conditions(&inst, &inst), "the same rule adds nothing and removes nothing");
        assert!(only_adds_conditions(&inst, &Rule { max_days: 7, ..default_rule() }));
        assert!(only_adds_conditions(&inst, &Rule { visibilities: vec!["departmental".into()], ..default_rule() }));
        assert!(only_adds_conditions(&inst, &Rule { affiliations: Some(vec!["staff".into()]), ..default_rule() }));
        assert!(!only_adds_conditions(&inst, &Rule { own_unit: false, ..default_rule() }), "own unit removed");
        assert!(!only_adds_conditions(&inst, &Rule { max_days: 90, ..default_rule() }), "longer");
        assert!(!only_adds_conditions(&inst, &Rule { max_role: "editor".into(), ..default_rule() }), "higher role");
        assert!(!only_adds_conditions(&inst, &Rule { max_sensitivity: "Restricted".into(), ..default_rule() }), "higher sensitivity");
        assert!(!only_adds_conditions(&inst, &Rule { visibilities: vec!["departmental".into(), "institutional".into(), "restricted".into()], ..default_rule() }), "wider visibilities");
        // An institution that limits affiliations cannot have them dropped.
        let limited = Rule { affiliations: Some(vec!["staff".into()]), ..default_rule() };
        assert!(!only_adds_conditions(&limited, &default_rule()));
        assert!(!only_adds_conditions(&limited, &Rule { affiliations: Some(vec!["staff".into(), "faculty".into()]), ..default_rule() }));
    }

    #[test]
    fn the_default_rule_reads_as_the_sentence_the_brief_gives() {
        assert_eq!(
            sentence(&default_rule()),
            "A viewer request for an app at Internal or below, in the requester's own unit or below, in a workspace whose visibility is departmental or institutional, for up to 30 days, is approved automatically."
        );
        let _ = SENSITIVITY_LEVELS;
        let _ = sensitivity_rank("Public");
    }
}
