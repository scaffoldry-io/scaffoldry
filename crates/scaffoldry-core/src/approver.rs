//! Who decides a step. A pure resolver over units, holdings, people, delegations and a time.
//!
//! Standard: W3C Organization Ontology (`org:Post`, `org:Role`, `org:Membership`, `org:heldBy`).
//! Separation of duties: NIST SP 800-53 AC-5. The submitter and the starter never decide.
//! Ids are strings and times are Unix seconds, so the function needs no other crate.

use serde::{Deserialize, Serialize};

/// The walk up the org tree stops after this many steps. A cycle ends the walk sooner.
const MAX_WALK: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Origin {
    Submitter,
    Workspace,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ApproverSpec {
    Position { key: String, from: Origin, walk_up: bool },
    Person { eppn: String },
    Role { name: String },
}

/// A rule written before approvers existed carries only `role`. It still means the same thing.
pub fn effective_approver(role: &str, approver: &Option<ApproverSpec>) -> ApproverSpec {
    approver.clone().unwrap_or_else(|| ApproverSpec::Role { name: role.to_string() })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Via {
    Holder,
    DelegateOf(String),
    Named,
    Role,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NoApprover {
    PositionVacant,
    OnlySubmitter,
    PersonUnavailable,
    RoleEmpty,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Approver {
    pub eppn: String,
    pub display_name: String,
    pub via: Via,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Resolution {
    pub approvers: Vec<Approver>,
    /// The unit a position was found at.
    pub unit_id: Option<String>,
    /// Why the list is empty.
    pub problem: Option<NoApprover>,
}

#[derive(Debug, Clone)]
pub struct Unit {
    pub id: String,
    pub parent_id: Option<String>,
    pub org_type: String,
    pub name: String,
}

/// One `roles` row. `position_key` is set for a position holding. A primary row places a person.
#[derive(Debug, Clone)]
pub struct Holding {
    pub eppn: String,
    pub unit_id: String,
    pub position_key: Option<String>,
    pub is_primary: bool,
}

#[derive(Debug, Clone)]
pub struct Person {
    pub eppn: String,
    pub display_name: String,
    pub active: bool,
    pub on_hold: bool,
}

#[derive(Debug, Clone)]
pub struct Delegation {
    pub delegator: String,
    pub delegate: String,
    /// `None` means every position the delegator holds.
    pub position_key: Option<String>,
    pub starts_at: i64,
    pub ends_at: i64,
    pub revoked: bool,
}

#[derive(Debug, Clone)]
pub struct Collaborator {
    pub eppn: String,
    pub role: String,
    pub affiliation: String,
}

#[derive(Debug, Clone)]
pub struct ResolveCtx {
    pub units: Vec<Unit>,
    pub holdings: Vec<Holding>,
    pub people: Vec<Person>,
    pub delegations: Vec<Delegation>,
    pub collaborators: Vec<Collaborator>,
    pub now: i64,
    pub submitter: Option<String>,
    pub starter: Option<String>,
    pub workspace_unit: Option<String>,
}

impl ResolveCtx {
    fn person(&self, eppn: &str) -> Option<&Person> {
        self.people.iter().find(|p| p.eppn.eq_ignore_ascii_case(eppn))
    }

    fn available(&self, eppn: &str) -> bool {
        self.person(eppn).is_some_and(|p| p.active && !p.on_hold)
    }

    fn excluded(&self, eppn: &str) -> bool {
        [&self.submitter, &self.starter]
            .iter()
            .any(|x| x.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(eppn)))
    }

    fn approver(&self, eppn: &str, via: Via) -> Approver {
        let display_name = self.person(eppn).map(|p| p.display_name.clone()).unwrap_or_else(|| eppn.to_string());
        Approver { eppn: eppn.to_string(), display_name, via }
    }

    fn parent_of(&self, unit: &str) -> Option<&str> {
        self.units.iter().find(|u| u.id == unit).and_then(|u| u.parent_id.as_deref())
    }

    fn start_unit(&self, from: &Origin) -> Option<String> {
        if *from == Origin::Submitter {
            if let Some(s) = &self.submitter {
                let own = self
                    .holdings
                    .iter()
                    .find(|h| h.eppn.eq_ignore_ascii_case(s) && h.is_primary && h.position_key.is_none());
                if let Some(h) = own {
                    return Some(h.unit_id.clone());
                }
            }
        }
        self.workspace_unit.clone()
    }

    /// Delegates of `holder` who are in their window and may stand in for `key`.
    fn delegates_of(&self, holder: &str, key: Option<&str>) -> Vec<Approver> {
        self.delegations
            .iter()
            .filter(|d| {
                !d.revoked
                    && d.delegator.eq_ignore_ascii_case(holder)
                    && d.starts_at <= self.now
                    && self.now < d.ends_at
                    && (d.position_key.is_none() || d.position_key.as_deref() == key)
                    && self.available(&d.delegate)
                    && !self.excluded(&d.delegate)
            })
            .map(|d| self.approver(&d.delegate, Via::DelegateOf(holder.to_string())))
            .collect()
    }
}

fn push_unique(list: &mut Vec<Approver>, a: Approver) {
    if !list.iter().any(|x| x.eppn.eq_ignore_ascii_case(&a.eppn)) {
        list.push(a);
    }
}

fn resolve_position(key: &str, from: &Origin, walk_up: bool, ctx: &ResolveCtx) -> Resolution {
    let mut unit = ctx.start_unit(from);
    let mut seen: Vec<String> = Vec::new();
    let mut only_submitter = false;

    for _ in 0..MAX_WALK {
        let Some(current) = unit else { break };
        if seen.contains(&current) {
            break; // a cycle ends the walk
        }
        seen.push(current.clone());

        let holders: Vec<&Holding> = ctx
            .holdings
            .iter()
            .filter(|h| h.unit_id == current && h.position_key.as_deref() == Some(key) && ctx.available(&h.eppn))
            .collect();

        let mut approvers: Vec<Approver> = Vec::new();
        for h in &holders {
            if ctx.excluded(&h.eppn) {
                only_submitter = true;
                continue;
            }
            push_unique(&mut approvers, ctx.approver(&h.eppn, Via::Holder));
        }
        // A holder who may not decide can still have a delegate who may.
        for h in &holders {
            for d in ctx.delegates_of(&h.eppn, Some(key)) {
                push_unique(&mut approvers, d);
            }
        }

        if !approvers.is_empty() {
            return Resolution { approvers, unit_id: Some(current), problem: None };
        }
        if !walk_up {
            break;
        }
        unit = ctx.parent_of(&current).map(str::to_string);
    }

    let problem = if only_submitter { NoApprover::OnlySubmitter } else { NoApprover::PositionVacant };
    Resolution { approvers: vec![], unit_id: None, problem: Some(problem) }
}

fn resolve_person(eppn: &str, ctx: &ResolveCtx) -> Resolution {
    if !ctx.available(eppn) {
        return Resolution { approvers: vec![], unit_id: None, problem: Some(NoApprover::PersonUnavailable) };
    }
    let mut approvers = Vec::new();
    if !ctx.excluded(eppn) {
        approvers.push(ctx.approver(eppn, Via::Named));
    }
    for d in ctx.delegates_of(eppn, None) {
        push_unique(&mut approvers, d);
    }
    let problem = approvers.is_empty().then_some(NoApprover::OnlySubmitter);
    Resolution { approvers, unit_id: None, problem }
}

fn resolve_role(name: &str, ctx: &ResolveCtx) -> Resolution {
    let mut approvers = Vec::new();
    for c in &ctx.collaborators {
        let matches = c.role.eq_ignore_ascii_case(name) || c.affiliation.eq_ignore_ascii_case(name);
        if matches && !ctx.excluded(&c.eppn) {
            push_unique(&mut approvers, ctx.approver(&c.eppn, Via::Role));
        }
    }
    let problem = approvers.is_empty().then_some(NoApprover::RoleEmpty);
    Resolution { approvers, unit_id: None, problem }
}

pub fn resolve_approvers(spec: &ApproverSpec, ctx: &ResolveCtx) -> Resolution {
    match spec {
        ApproverSpec::Position { key, from, walk_up } => resolve_position(key, from, *walk_up, ctx),
        ApproverSpec::Person { eppn } => resolve_person(eppn, ctx),
        ApproverSpec::Role { name } => resolve_role(name, ctx),
    }
}

/// Whether `user` may decide the waiting step. The submitter and the starter never may. When an
/// administrator assigned the instance, only the assignee may. Otherwise the user must be among
/// the resolved approvers. A rule written with only a role also accepts a caller whose own
/// affiliation equals the role name, as it always did.
pub fn can_decide(
    user: &str,
    user_affiliation: &str,
    assigned_to: Option<&str>,
    spec: &ApproverSpec,
    ctx: &ResolveCtx,
) -> bool {
    if ctx.excluded(user) {
        return false;
    }
    if let Some(assignee) = assigned_to {
        return assignee.eq_ignore_ascii_case(user);
    }
    if resolve_approvers(spec, ctx).approvers.iter().any(|a| a.eppn.eq_ignore_ascii_case(user)) {
        return true;
    }
    matches!(spec, ApproverSpec::Role { name } if user_affiliation.eq_ignore_ascii_case(name))
}

/// What the platform knows, for checking a spec when a rule is saved.
#[derive(Debug, Clone, Default)]
pub struct KnownSets {
    pub position_keys: Vec<String>,
    pub retired_keys: Vec<String>,
    pub eppns: Vec<String>,
}

pub fn validate_approver(spec: &ApproverSpec, known: &KnownSets) -> Result<(), String> {
    match spec {
        ApproverSpec::Position { key, .. } => {
            if known.retired_keys.iter().any(|k| k == key) {
                Err(format!("Position '{key}' is retired"))
            } else if !known.position_keys.iter().any(|k| k == key) {
                Err(format!("Position '{key}' does not exist"))
            } else {
                Ok(())
            }
        }
        ApproverSpec::Person { eppn } => {
            if known.eppns.iter().any(|e| e.eq_ignore_ascii_case(eppn)) {
                Ok(())
            } else {
                Err(format!("Person '{eppn}' does not exist"))
            }
        }
        ApproverSpec::Role { name } => {
            if name.trim().is_empty() {
                Err("A role name is required".to_string())
            } else {
                Ok(())
            }
        }
    }
}
