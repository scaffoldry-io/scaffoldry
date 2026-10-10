//! Approvers phase 1: the pure resolver. No database, no clock, no I/O.

use scaffoldry_core::approver::*;
use scaffoldry_core::StepKind;

const NOW: i64 = 1_000_000;

fn unit(id: &str, parent: Option<&str>, org_type: &str) -> Unit {
    Unit { id: id.into(), parent_id: parent.map(Into::into), org_type: org_type.into(), name: id.into() }
}

fn person(eppn: &str) -> Person {
    Person { eppn: eppn.into(), display_name: format!("Dr. {eppn}"), active: true, on_hold: false }
}

fn holds(eppn: &str, unit: &str, key: &str) -> Holding {
    Holding { eppn: eppn.into(), unit_id: unit.into(), position_key: Some(key.into()), is_primary: false }
}

fn primary(eppn: &str, unit: &str) -> Holding {
    Holding { eppn: eppn.into(), unit_id: unit.into(), position_key: None, is_primary: true }
}

fn delegation(from: &str, to: &str, starts: i64, ends: i64) -> Delegation {
    Delegation {
        delegator: from.into(),
        delegate: to.into(),
        position_key: None,
        starts_at: starts,
        ends_at: ends,
        revoked: false,
    }
}

/// Institution > College > Physics and Chemistry. Faculty `sam` works in Physics.
fn base() -> ResolveCtx {
    ResolveCtx {
        units: vec![
            unit("inst", None, "Institution"),
            unit("college", Some("inst"), "College"),
            unit("physics", Some("college"), "Department"),
            unit("chem", Some("college"), "Department"),
        ],
        holdings: vec![
            primary("sam", "physics"),
            holds("rivera", "physics", "chair"),
            holds("okafor", "chem", "chair"),
            holds("dean", "college", "chair"),
        ],
        people: ["sam", "rivera", "okafor", "dean", "temp", "proxy"].iter().map(|e| person(e)).collect(),
        delegations: vec![],
        collaborators: vec![],
        now: NOW,
        submitter: Some("sam".into()),
        starter: None,
        workspace_unit: Some("chem".into()),
    }
}

fn chair(walk_up: bool) -> ApproverSpec {
    ApproverSpec::Position { key: "chair".into(), from: Origin::Submitter, walk_up }
}

fn eppns(r: &Resolution) -> Vec<&str> {
    r.approvers.iter().map(|a| a.eppn.as_str()).collect()
}

// 1
#[test]
fn the_chair_at_the_submitters_department_is_returned_as_a_holder() {
    let r = resolve_approvers(&chair(true), &base());
    assert_eq!(eppns(&r), ["rivera"]);
    assert_eq!(r.approvers[0].via, Via::Holder);
    assert_eq!(r.unit_id.as_deref(), Some("physics"));
    assert_eq!(r.problem, None);
}

// 2
#[test]
fn a_vacant_department_walks_up_to_the_dean_or_reports_vacancy() {
    let mut ctx = base();
    ctx.holdings.retain(|h| h.eppn != "rivera");
    let up = resolve_approvers(&chair(true), &ctx);
    assert_eq!(eppns(&up), ["dean"]);
    assert_eq!(up.unit_id.as_deref(), Some("college"));

    let stay = resolve_approvers(&chair(false), &ctx);
    assert!(stay.approvers.is_empty());
    assert_eq!(stay.problem, Some(NoApprover::PositionVacant));
}

// 3
#[test]
fn the_only_holder_being_the_submitter_moves_up_or_reports_only_submitter() {
    let mut ctx = base();
    ctx.submitter = Some("rivera".into());
    ctx.holdings.push(primary("rivera", "physics"));
    ctx.holdings.retain(|h| !(h.eppn == "sam" && h.is_primary));
    let up = resolve_approvers(&chair(true), &ctx);
    assert_eq!(eppns(&up), ["dean"]);

    let stay = resolve_approvers(&chair(false), &ctx);
    assert!(stay.approvers.is_empty());
    assert_eq!(stay.problem, Some(NoApprover::OnlySubmitter));
}

// 4
#[test]
fn a_holder_on_hold_or_inactive_is_skipped() {
    let mut ctx = base();
    ctx.people.iter_mut().find(|p| p.eppn == "rivera").unwrap().on_hold = true;
    assert_eq!(eppns(&resolve_approvers(&chair(true), &ctx)), ["dean"]);

    let mut ctx = base();
    ctx.people.iter_mut().find(|p| p.eppn == "rivera").unwrap().active = false;
    assert_eq!(eppns(&resolve_approvers(&chair(true), &ctx)), ["dean"]);
}

// 5
#[test]
fn a_delegate_counts_only_inside_the_window_and_only_one_hop() {
    let mut ctx = base();
    ctx.delegations = vec![delegation("rivera", "temp", NOW - 10, NOW + 10)];
    let r = resolve_approvers(&chair(true), &ctx);
    assert_eq!(eppns(&r), ["rivera", "temp"]);
    assert_eq!(r.approvers[1].via, Via::DelegateOf("rivera".into()));

    // Outside the window, either side.
    ctx.delegations = vec![delegation("rivera", "temp", NOW + 1, NOW + 10)];
    assert_eq!(eppns(&resolve_approvers(&chair(true), &ctx)), ["rivera"]);
    ctx.delegations = vec![delegation("rivera", "temp", NOW - 10, NOW - 1)];
    assert_eq!(eppns(&resolve_approvers(&chair(true), &ctx)), ["rivera"]);

    // Revoked.
    let mut revoked = delegation("rivera", "temp", NOW - 10, NOW + 10);
    revoked.revoked = true;
    ctx.delegations = vec![revoked];
    assert_eq!(eppns(&resolve_approvers(&chair(true), &ctx)), ["rivera"]);

    // The delegate is the submitter.
    ctx.delegations = vec![delegation("rivera", "sam", NOW - 10, NOW + 10)];
    assert_eq!(eppns(&resolve_approvers(&chair(true), &ctx)), ["rivera"]);

    // A delegate's own delegate is not followed.
    ctx.delegations = vec![
        delegation("rivera", "temp", NOW - 10, NOW + 10),
        delegation("temp", "proxy", NOW - 10, NOW + 10),
    ];
    assert_eq!(eppns(&resolve_approvers(&chair(true), &ctx)), ["rivera", "temp"]);
}

// 6
#[test]
fn the_starter_is_excluded_even_when_they_hold_the_position() {
    let mut ctx = base();
    ctx.starter = Some("rivera".into());
    assert_eq!(eppns(&resolve_approvers(&chair(true), &ctx)), ["dean"]);
}

// 7
#[test]
fn a_parent_cycle_returns_empty_and_does_not_loop() {
    let mut ctx = base();
    ctx.units = vec![unit("a", Some("b"), "Department"), unit("b", Some("a"), "College")];
    ctx.holdings = vec![primary("sam", "a")];
    let r = resolve_approvers(&chair(true), &ctx);
    assert!(r.approvers.is_empty());
    assert_eq!(r.problem, Some(NoApprover::PositionVacant));
}

// 8
#[test]
fn a_role_returns_the_workspace_collaborators_with_that_role_minus_the_submitter() {
    let mut ctx = base();
    ctx.collaborators = vec![
        Collaborator { eppn: "sam".into(), role: "reviewer".into(), affiliation: "faculty".into() },
        Collaborator { eppn: "okafor".into(), role: "reviewer".into(), affiliation: "faculty".into() },
        Collaborator { eppn: "dean".into(), role: "viewer".into(), affiliation: "staff".into() },
    ];
    let by_role = resolve_approvers(&ApproverSpec::Role { name: "reviewer".into() }, &ctx);
    assert_eq!(eppns(&by_role), ["okafor"]);
    assert_eq!(by_role.approvers[0].via, Via::Role);

    let by_affiliation = resolve_approvers(&ApproverSpec::Role { name: "staff".into() }, &ctx);
    assert_eq!(eppns(&by_affiliation), ["dean"]);

    let none = resolve_approvers(&ApproverSpec::Role { name: "nobody".into() }, &ctx);
    assert_eq!(none.problem, Some(NoApprover::RoleEmpty));
}

// 9
#[test]
fn an_old_rule_with_only_a_role_resolves_as_a_role() {
    let old = r#"{"UserTask":{"role":"reviewer","prompt":"Approve?","approve":[],"reject":[]}}"#;
    let StepKind::UserTask { role, approver, .. } = serde_json::from_str::<StepKind>(old).unwrap() else {
        panic!("a user task");
    };
    assert_eq!(approver, None);
    assert_eq!(effective_approver(&role, &approver), ApproverSpec::Role { name: "reviewer".into() });

    // The new shape uses the default external tag, like every other enum.
    let wire = serde_json::to_value(chair(true)).unwrap();
    assert_eq!(wire["Position"]["key"], "chair");
    assert_eq!(wire["Position"]["from"], "Submitter");
    assert_eq!(serde_json::from_value::<ApproverSpec>(wire).unwrap(), chair(true));
}

// A person approver follows the same exclusion and availability rules.
#[test]
fn a_named_person_must_be_available_and_is_not_the_submitter() {
    let ctx = base();
    let ok = resolve_approvers(&ApproverSpec::Person { eppn: "okafor".into() }, &ctx);
    assert_eq!(eppns(&ok), ["okafor"]);
    assert_eq!(ok.approvers[0].via, Via::Named);

    let mut held = base();
    held.people.iter_mut().find(|p| p.eppn == "okafor").unwrap().on_hold = true;
    let r = resolve_approvers(&ApproverSpec::Person { eppn: "okafor".into() }, &held);
    assert_eq!(r.problem, Some(NoApprover::PersonUnavailable));

    let me = resolve_approvers(&ApproverSpec::Person { eppn: "sam".into() }, &ctx);
    assert_eq!(me.problem, Some(NoApprover::OnlySubmitter));
}

// The start unit: the workspace's unit when the origin is the workspace.
#[test]
fn the_workspace_origin_starts_at_the_workspace_unit() {
    let spec = ApproverSpec::Position { key: "chair".into(), from: Origin::Workspace, walk_up: false };
    let r = resolve_approvers(&spec, &base());
    assert_eq!(eppns(&r), ["okafor"]);
    assert_eq!(r.unit_id.as_deref(), Some("chem"));
}

#[test]
fn validate_approver_checks_keys_people_and_role_names() {
    let known = KnownSets { position_keys: vec!["chair".into()], retired_keys: vec!["old".into()], eppns: vec!["rivera".into()] };
    assert!(validate_approver(&chair(true), &known).is_ok());
    assert!(validate_approver(&ApproverSpec::Position { key: "nope".into(), from: Origin::Submitter, walk_up: true }, &known).is_err());
    assert!(validate_approver(&ApproverSpec::Position { key: "old".into(), from: Origin::Submitter, walk_up: true }, &known).is_err());
    assert!(validate_approver(&ApproverSpec::Person { eppn: "rivera".into() }, &known).is_ok());
    assert!(validate_approver(&ApproverSpec::Person { eppn: "ghost".into() }, &known).is_err());
    assert!(validate_approver(&ApproverSpec::Role { name: "  ".into() }, &known).is_err());
}
