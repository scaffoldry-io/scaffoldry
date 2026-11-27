//! Access model phase 13: asking for access.
//!
//! A refused person can ask. A rule approves the routine asks alone (`auto`). An AI tool working inside a narrowed
//! session asks for what the person may already do (`self`), and the person confirms. Everything else goes to the people
//! `authority::required` names (`approver`). The requester never decides their own request, and a request an AI tool
//! filed waits for the person first.

use crate::repository_access_requests::{Approved, Effects, RequestGrant, RequestRow, RuleRow};
use crate::service::admin::admin_write;
use crate::service::authority;
use crate::service::history::Origin;
use crate::service::library::Problems;
use crate::service::members::{self, Role};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState, WorkspaceRecord};
use scaffoldry_core::access_request::{self as rules, Facts, Rule};
use scaffoldry_core::coordinates::{sensitivity_rank, SENSITIVITY_LEVELS};
use scaffoldry_core::ledger::DecisionType;
use serde_json::{json, Value};
use uuid::Uuid;

const OSCAL_CONTROL: &str = "AC-02";
const WAITING: &str = "Waiting for person";

fn internal(e: impl ToString) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

/// The longest request, in days: the setting `access.request_max_days`.
pub(crate) fn max_days(state: &SharedState) -> i64 {
    state.settings.read().ok().and_then(|s| s.get("access.request_max_days").and_then(Value::as_i64)).filter(|d| (1..=rules::MAX_DAYS).contains(d)).unwrap_or(90)
}

/// What a request names, checked.
struct Input {
    kind: String,
    id: String,
    role: Role,
    clearance: Option<String>,
    reason: String,
    days: i64,
}

/// The thing asked for, found.
struct Object {
    workspace: WorkspaceRecord,
    app: Option<scaffoldry_engine::AppManifest>,
}

impl Object {
    fn anchor(&self, input: &Input) -> String {
        match &self.app {
            Some(_) => format!("app:{}", input.id),
            None => format!("workspace:{}", self.workspace.id),
        }
    }

    fn sentence(&self) -> String {
        match &self.app {
            Some(app) => format!("the app '{}'", app.title),
            None => format!("the workspace '{}'", self.workspace.name),
        }
    }
}

fn parse(state: &SharedState, body: &Value) -> Result<Input, ServiceError> {
    let mut p = Problems::default();
    let kind = p.text(body, "object_kind", 16, true);
    let id = p.text(body, "object_id", 255, true);
    let role = p.text(body, "role", 16, true);
    let reason = p.text(body, "reason", 500, true);
    let clearance = p.text(body, "clearance", 16, false);
    let days = body.get("days").and_then(Value::as_i64);
    let limit = max_days(state);
    if !days.is_some_and(|d| (1..=limit).contains(&d)) {
        p.add("days", format!("days is a whole number of days from 1 to {limit}"));
    }
    if kind.as_deref().is_some_and(|k| k != "app" && k != "workspace") {
        p.add("object_kind", "object_kind is 'app' or 'workspace'");
    }
    let role = role.and_then(|r| {
        let parsed = Role::parse(&r);
        if parsed.is_none() {
            p.add("role", "role is one of viewer, commenter, editor, admin, owner");
        }
        parsed
    });
    if let Some(c) = &clearance {
        if sensitivity_rank(c).is_none() {
            p.add("clearance", format!("clearance is one of {}", SENSITIVITY_LEVELS.join(", ")));
        }
    }
    p.done()?;
    match (kind, id, role, reason, days) {
        (Some(kind), Some(id), Some(role), Some(reason), Some(days)) => Ok(Input { kind, id, role, clearance, reason, days }),
        _ => Err(ServiceError::bad_request("The request is not complete")),
    }
}

fn find(state: &SharedState, input: &Input) -> Result<Object, ServiceError> {
    use scaffoldry_engine::HostRouter;
    let missing = |what: &str| {
        let mut p = Problems::default();
        p.add("object_id", format!("{what} '{}' was not found", input.id));
        p.done().err().unwrap_or_else(|| ServiceError::not_found(what))
    };
    let (ws_id, app) = if input.kind == "app" {
        let app = state.engine.read().map_err(internal)?.resolve_by_slug(&input.id).cloned().ok_or_else(|| missing("The app"))?;
        (app.workspace_id.clone().ok_or_else(|| missing("The app"))?, Some(app))
    } else {
        (input.id.clone(), None)
    };
    let workspace = state.workspaces.read().map_err(internal)?.get(&ws_id).cloned().ok_or_else(|| missing("The workspace"))?;
    Ok(Object { workspace, app })
}

/// The person's role on the thing, from the memberships and grants they hold now.
fn held_role(state: &SharedState, caller: &AuthUser, obj: &Object, input: &Input) -> Option<Role> {
    match &obj.app {
        Some(_) => members::member_role_in_app(caller, &obj.workspace.id, &input.id, state),
        None => members::member_role(caller, &obj.workspace.id, state),
    }
}

/// How sensitive the thing is, as a level on the ladder.
fn sensitivity_of(state: &SharedState, obj: &Object) -> String {
    let (_, rank) = crate::service::coordinates::cedar_place(state, &obj.workspace, obj.app.as_ref());
    SENSITIVITY_LEVELS[usize::try_from(rank).unwrap_or(0).min(SENSITIVITY_LEVELS.len() - 1)].to_string()
}

struct Assessed {
    facts: Facts,
    /// The signals that failed, as the authority matrix names them.
    raised: Vec<&'static str>,
}

fn assess(state: &SharedState, eppn: &str, affiliation: &str, obj: &Object, role: Role, clearance: Option<&str>, days: i64) -> Assessed {
    let roles = state.roles_of(eppn);
    let path = authority::unit_path(state, obj.workspace.organization_id);
    let in_own_unit = roles.iter().any(|r| path.contains(&r.organization_id));
    let mut affiliations: Vec<String> = roles.iter().map(|r| r.scoped_affiliation.clone()).collect();
    affiliations.push(affiliation.to_string());
    // A clearance above what the thing carries is part of what is asked for.
    let object_level = sensitivity_of(state, obj);
    let level = match clearance {
        Some(c) if sensitivity_rank(c) > sensitivity_rank(&object_level) => c.to_string(),
        _ => object_level,
    };
    let mut raised = Vec::new();
    if role >= Role::Editor {
        raised.push("request_write");
    }
    if sensitivity_rank(&level) > sensitivity_rank("Internal") {
        raised.push("request_sensitive");
    }
    if !in_own_unit {
        raised.push("request_outside_unit");
    }
    Assessed { facts: Facts { role, sensitivity: level, visibility: obj.workspace.visibility.clone(), in_own_unit, days, affiliations }, raised }
}

/// The institution's rule that approves this request, if one does and every unit above the workspace that has rules of
/// its own agrees. A unit's rules only add conditions, so they can stop an approval and never make one.
fn approving_rule(state: &SharedState, obj: &Object, facts: &Facts) -> Result<Option<Uuid>, ServiceError> {
    state.repository.ensure_default_request_rule().map_err(internal)?;
    let all = state.repository.request_rules().map_err(internal)?;
    let root = Uuid::parse_str("00000000-0000-0000-0000-000000000001").map_err(internal)?;
    let path = authority::unit_path(state, obj.workspace.organization_id);
    let active = |r: &&RuleRow| r.retired_at.is_none();
    let parsed = |r: &RuleRow| serde_json::from_value::<Rule>(r.rule.clone()).ok();
    let institution = all.iter().filter(active).filter(|r| r.organization_id == root).find(|r| parsed(r).is_some_and(|rule| rules::fits(&rule, facts)));
    let Some(institution) = institution else { return Ok(None) };
    for unit in path.iter().filter(|u| **u != root) {
        let own: Vec<&RuleRow> = all.iter().filter(active).filter(|r| r.organization_id == *unit).collect();
        if !own.is_empty() && !own.iter().any(|r| parsed(r).is_some_and(|rule| rules::fits(&rule, facts))) {
            return Ok(None);
        }
    }
    Ok(Some(institution.id))
}

/// The checks of a request, in the shape `authority` reads.
fn checks_of(raised: &[&str]) -> Value {
    json!({ "checks": raised.iter().map(|n| json!({ "name": n, "outcome": "fail", "messages": [] })).collect::<Vec<_>>() })
}

fn need_for(state: &SharedState, obj: &Object, requester: &str, raised: &[&str]) -> Result<authority::Need, ServiceError> {
    authority::requirement_for(state, &obj.workspace.id, &checks_of(raised), requester)
}

/// Where a request made from the catalog goes (data layer phase 2e).
enum Where {
    /// The people who hold the dataset's steward position now, found from the dataset's unit up. The requester is never one.
    Steward { key: String, name: String, holders: Vec<String> },
    /// The dataset has no steward position, or nobody holds it. The access rules route the request as for any other, and the
    /// reason is shown to the requester.
    Fallback(String),
}

/// Where a catalog request goes. `None` for a request that was not made from the catalog.
fn routed(state: &SharedState, req: &RequestRow) -> Option<Where> {
    use scaffoldry_core::approver::{ApproverSpec, Origin};
    let id = req.dataset_id.as_deref()?;
    let Some(row) = state.repository.catalog_row(id).ok().flatten() else { return Some(Where::Fallback("The dataset is no longer in the catalog, so the access rules route this request".to_string())) };
    let Some(key) = row.steward_position.clone() else { return Some(Where::Fallback("The dataset has no steward position, so the access rules route this request".to_string())) };
    let name = state.position_types.read().ok().and_then(|p| p.get(&key).map(|t| t.name.clone())).unwrap_or_else(|| key.clone());
    let Some(unit) = crate::service::catalog::unit_of(state, &row) else {
        return Some(Where::Fallback(format!("The dataset sits at no unit, so no holder of '{name}' can be found and the access rules route this request")));
    };
    let spec = ApproverSpec::Position { key: key.clone(), from: Origin::Workspace, walk_up: true };
    let mut holders: Vec<String> = crate::service::approvers::resolved_approvers(state, &spec, unit, &[], Some(&req.requester));
    holders.retain(|h| !h.eq_ignore_ascii_case(&req.requester));
    holders.sort();
    holders.dedup();
    if holders.is_empty() {
        return Some(Where::Fallback(format!("Nobody holds the steward position '{name}' for this dataset now, so the access rules route this request")));
    }
    Some(Where::Steward { key, name, holders })
}

/// The people whose decision the request waits for.
fn waiting_on(state: &SharedState, req: &RequestRow, obj: &Object, need: &authority::Need) -> Vec<String> {
    use scaffoldry_core::authority::Path;
    if let Some(Where::Steward { holders, .. }) = routed(state, req) {
        return holders;
    }
    let requester = req.requester.as_str();
    let mut people: Vec<String> = match need.requirement.path {
        Path::Approver => need.resolved.iter().map(|a| a.eppn.clone()).collect(),
        _ => members::people_with_roles(&obj.workspace.id, state).into_iter().filter(|(_, role)| *role >= Role::Admin).map(|(eppn, _)| eppn).collect(),
    };
    people.retain(|p| !p.eq_ignore_ascii_case(requester));
    people.sort();
    people.dedup();
    people
}

/// Whether `caller` may decide a request, as the matrix stands now. A catalog request that goes to a steward position is
/// decided by its holders and by nobody else.
fn may_decide(state: &SharedState, caller: &AuthUser, req: &RequestRow, obj: &Object, need: &authority::Need) -> bool {
    use scaffoldry_core::authority::Path;
    if req.requester.eq_ignore_ascii_case(&caller.eppn) {
        return false;
    }
    if let Some(Where::Steward { holders, .. }) = routed(state, req) {
        return holders.iter().any(|h| h.eq_ignore_ascii_case(&caller.eppn));
    }
    match need.requirement.path {
        Path::Approver => need.resolved.iter().any(|a| a.eppn.eq_ignore_ascii_case(&caller.eppn)),
        _ => crate::service::access::authorize_workspace(caller, &obj.workspace.id, "manage_workspace", state).is_ok(),
    }
}

fn input_of(req: &RequestRow) -> Result<Input, ServiceError> {
    Ok(Input {
        kind: req.object_kind.clone(),
        id: req.object_id.clone(),
        role: Role::parse(&req.role).ok_or_else(|| internal("The stored request has no role"))?,
        clearance: req.clearance.clone(),
        reason: req.reason.clone(),
        days: i64::from(req.days),
    })
}

fn view(state: &SharedState, req: &RequestRow) -> Result<Value, ServiceError> {
    let mut out = req.json();
    if matches!(req.status.as_str(), "Pending" | "Waiting for person") {
        let input = input_of(req)?;
        if let Ok(obj) = find(state, &input) {
            let a = assess(state, &req.requester, "", &obj, input.role, input.clearance.as_deref(), input.days);
            let need = need_for(state, &obj, &req.requester, &a.raised)?;
            out["waiting_on"] = json!(waiting_on(state, req, &obj, &need));
            out["object"] = json!(obj.sentence());
            match routed(state, req) {
                Some(Where::Steward { key, name, holders }) => {
                    out["routing"] = json!({ "to": "steward", "position": key, "position_name": name, "holders": holders, "sentence": format!("Waiting for {}: {}.", name, holders.join(", ")) });
                }
                Some(Where::Fallback(why)) => {
                    out["routing"] = json!({ "to": "approver", "fallback": true, "sentence": why });
                    out["authority"] = need.to_json(state);
                }
                None => out["authority"] = need.to_json(state),
            }
        }
    }
    if let Some(name) = req.dataset_id.as_deref().and_then(|d| crate::service::catalog::name_of(state, d)) {
        out["object"] = json!(format!("the dataset '{name}'"));
    }
    Ok(out)
}

fn ledger_payload(req: &RequestRow, outcome: &str, route: &str, rule: Option<Uuid>, by: &str, grant: Option<&RequestGrant>) -> Value {
    json!({
        "action": "access_request_decided", "request_id": req.id, "outcome": outcome, "route": route, "rule_id": rule, "decided_by": by,
        "requester": req.requester, "object_kind": req.object_kind, "object_id": req.object_id, "role": req.role, "clearance": req.clearance,
        "days": req.days, "filed_by_client": req.filed_by_client, "dataset": req.dataset_id, "answers": req.answers, "grant": grant.map(|g| json!({ "holder": g.holder, "role": g.role, "anchor": g.anchor_id, "days": g.days })),
    })
}

/// Approves `req` through `route`. The ledger entry is written first.
/// How an approval comes about: the states it may leave, the route, the rule behind it, and the note for the ledger.
struct Way<'a> {
    from: &'a [&'a str],
    route: &'a str,
    rule: Option<Uuid>,
    note: &'a str,
}

fn approve(state: &SharedState, actor: &AuthUser, req: &RequestRow, obj: &Object, way: Way<'_>) -> Result<Value, ServiceError> {
    let Way { from, route, rule, note } = way;
    let input = input_of(req)?;
    let (grant, widen) = if route == "self" {
        // The person already holds the role. Only a session that was limited below writing needs widening.
        let widen = (input.role >= Role::Editor).then_some(req.session_id).flatten().map(|s| (s, "write"));
        (None, widen)
    } else {
        crate::service::access_grants::check_request_conflict(state, &req.requester, input.role, &obj.anchor(&input))?;
        let reason = format!("Access request {}{}: {}", req.id, rule.map(|r| format!(" under rule {r}")).unwrap_or_default(), req.reason);
        (
            Some(RequestGrant { holder: req.requester.clone(), role: req.role.clone(), anchor_id: obj.anchor(&input), clearance: req.clearance.clone(), days: req.days, reason, created_by: actor.eppn.clone() }),
            None,
        )
    };
    let payload = ledger_payload(req, "Approved", route, rule, &actor.eppn, grant.as_ref());
    let outcome = admin_write(state, actor, DecisionType::AccessRequestDecided, OSCAL_CONTROL, note, &payload, || {
        state.repository.approve_request(req.id, from, route, rule, &actor.eppn, Effects { note, grant: grant.as_ref(), widen }).map_err(internal)
    })?;
    match outcome {
        Approved::Done { .. } => {
            let _ = state.refresh();
            let stored = state.repository.get_request(req.id).map_err(internal)?.ok_or_else(|| ServiceError::not_found("Request not found"))?;
            view(state, &stored)
        }
        Approved::Not(status) => Err(ServiceError::conflict(format!("That request is {status}"))),
        Approved::Missing => Err(ServiceError::not_found("Request not found")),
    }
}

/// What a request made from the catalog carries beyond an ordinary request.
struct Catalog {
    dataset: String,
    answers: Value,
}

/// The default length of a catalog request whose form has no duration, in days. It is the default rule's longest ask.
const DEFAULT_DAYS: i64 = 30;

/// A request on `object_kind: "dataset"` is an ordinary request on the dataset's app, with the form the dataset lists.
/// `object_id` is the dataset id. A dataset the caller may not discover is the same not found as one that does not exist.
/// A published dataset with no app is refused: it holds no rows to grant. Any other body is returned as it came.
fn catalog_input(state: &SharedState, caller: &AuthUser, body: &Value) -> Result<(Value, Option<Catalog>), ServiceError> {
    if body.get("object_kind").and_then(Value::as_str) != Some("dataset") {
        return Ok((body.clone(), None));
    }
    let mut p = Problems::default();
    let id = p.text(body, "object_id", 64, true);
    p.done()?;
    let row = crate::service::catalog::discoverable(caller, &id.unwrap_or_default(), state)?;
    if row.published_id.is_some() {
        return Err(ServiceError::bad_request(
            "This dataset is published on its own, and no app holds its rows, so there is no access to grant. Ask its steward or a Platform Admin to share it",
        ));
    }
    let listed = |f: &str| row.request_fields.iter().any(|x| x == f);
    let limit = max_days(state);
    let mut p = Problems::default();
    let reason = p.text(body, "reason", 500, listed("reason"));
    let project = p.text(body, "project", 200, listed("project"));
    let agreement = p.text(body, "agreement_reference", 200, listed("agreement_reference"));
    for f in ["project", "agreement_reference"] {
        if !listed(f) && body.get(f).is_some_and(|v| !v.is_null()) {
            p.add(f, format!("{f} is not on this dataset's request form"));
        }
    }
    let asked = body.get("duration").or_else(|| body.get("days")).filter(|v| !v.is_null());
    let days = asked.and_then(Value::as_i64).filter(|d| (1..=limit).contains(d));
    match (asked, days) {
        (None, _) if listed("duration") => p.add("duration", format!("duration is required: a whole number of days from 1 to {limit}")),
        (Some(_), None) => p.add("duration", format!("duration is a whole number of days from 1 to {limit}")),
        _ => {}
    }
    p.done()?;
    let mut answers = serde_json::Map::new();
    for (key, value) in [("reason", reason.clone().map(Value::from)), ("project", project.map(Value::from)), ("duration", days.map(Value::from)), ("agreement_reference", agreement.map(Value::from))] {
        if let Some(v) = value {
            answers.insert(key.to_string(), v);
        }
    }
    let name = crate::service::catalog::name_of(state, &row.id).unwrap_or_else(|| "a dataset".to_string());
    let mapped = json!({
        "object_kind": "app", "object_id": row.app_slug,
        "role": body.get("role").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!("viewer")),
        "clearance": body.get("clearance").cloned().unwrap_or(Value::Null),
        "reason": reason.unwrap_or_else(|| format!("Access to {name}, asked for from the catalog")),
        "days": days.unwrap_or_else(|| DEFAULT_DAYS.min(limit)),
    });
    Ok((mapped, Some(Catalog { dataset: row.id, answers: Value::Object(answers) })))
}

/// How long a catalog request waits before the steward is reminded, and again between reminders.
const REMIND_AFTER_DAYS: i64 = 3;

/// Tells the people a catalog request waits for, through the inbox and, as each person chose, email. A request that was not
/// made from the catalog tells nobody here. A failure to tell never undoes the request: the request is the record.
fn tell_waiting(state: &SharedState, req: &RequestRow, reminder: bool) {
    use crate::notifications::{notify, DatasetRef, Flag, Notice, PersonRef, Recipients};
    let Some(dataset) = req.dataset_id.clone() else { return };
    let Ok(input) = input_of(req) else { return };
    let Ok(obj) = find(state, &input) else { return };
    let a = assess(state, &req.requester, "", &obj, input.role, input.clearance.as_deref(), input.days);
    let Ok(need) = need_for(state, &obj, &req.requester, &a.raised) else { return };
    let people = waiting_on(state, req, &obj, &need);
    let notice = Notice::AccessRequestWaiting { by: PersonRef(req.requester.clone()), dataset: DatasetRef(dataset), reminder: Flag(reminder) };
    if let Err(e) = notify(state, &Recipients::People(people), Some(&req.requester), &notice) {
        eprintln!("Warning: the people a request waits for could not be told: {e}");
    }
}

/// Reminds the people each waiting catalog request waits for, once for each `REMIND_AFTER_DAYS` it has waited. Returns how
/// many requests were reminded.
pub fn remind_due(state: &SharedState, now: chrono::DateTime<chrono::Utc>) -> Result<usize, String> {
    let ids = state.repository.claim_requests_to_remind(now - chrono::Duration::days(REMIND_AFTER_DAYS), now, 200).map_err(|e| e.to_string())?;
    for id in &ids {
        if let Some(req) = state.repository.get_request(*id).map_err(|e| e.to_string())? {
            tell_waiting(state, &req, true);
        }
    }
    Ok(ids.len())
}

/// The `remind_requests` job, run every hour.
pub fn remind_job(ctx: &crate::jobs::JobCtx, _payload: Value) -> Result<Value, crate::jobs::JobError> {
    let state = ctx.state().ok_or_else(|| crate::jobs::JobError::Permanent("the reminders need the server's state, and this runner has none".to_string()))?;
    let reminded = remind_due(state, chrono::Utc::now()).map_err(crate::jobs::JobError::Retry)?;
    Ok(json!({ "reminded": reminded }))
}

/// Files a request for `caller`. Returns the request as it stands: approved at once when a rule allows it.
pub fn file(state: &SharedState, caller: &AuthUser, origin: &Origin, body: &Value) -> Result<Value, ServiceError> {
    let (body, catalog) = catalog_input(state, caller, body)?;
    let input = parse(state, &body)?;
    let obj = find(state, &input)?;
    let a = assess(state, &caller.eppn, &caller.affiliation, &obj, input.role, input.clearance.as_deref(), input.days);

    let (client, task) = match origin.session {
        Some(session) => state.repository.session_client_and_task(session).map_err(internal)?.map_or((None, None), |(c, t)| (Some(c), t)),
        None => (None, None),
    };
    let by_tool = origin.session.is_some();
    // A tool asking for what the person already may do asks to widen its own session. Anything else is the person's to ask.
    let route = if by_tool && held_role(state, caller, &obj, &input).is_some_and(|mine| mine >= input.role) { "self" } else { "approver" };
    let row = RequestRow {
        id: Uuid::new_v4(),
        requester: caller.eppn.clone(),
        filed_by_client: client,
        session_id: origin.session,
        task,
        object_kind: input.kind.clone(),
        object_id: input.id.clone(),
        role: input.role.as_str().to_string(),
        clearance: input.clearance.clone(),
        reason: input.reason.clone(),
        days: i16::try_from(input.days).map_err(internal)?,
        route: route.to_string(),
        rule_id: None,
        status: if by_tool { WAITING } else { "Pending" }.to_string(),
        decided_by: None,
        decided_at: None,
        grant_id: None,
        created_at: String::new(),
        grant_ends: None,
        grant_ended: false,
        dataset_id: catalog.as_ref().map(|c| c.dataset.clone()),
        answers: catalog.map_or_else(|| json!({}), |c| c.answers),
        decision_note: None,
    };
    state.repository.insert_request(&row).map_err(internal)?;
    if by_tool {
        // The request waits for the person. The link opens their requests in the desk, where they confirm it.
        let mut waiting = view(state, &row_of(state, row.id)?)?;
        waiting["desk_link"] = json!(format!("/me/access/requests?request={}", row.id));
        return Ok(waiting);
    }
    settle(state, caller, &row, &obj, &a)
}

/// The refusal for a request that names another person. Only the caller's own access is explained. No steward role
/// exists in the code yet, so nobody may ask about a dataset on another person's behalf. It names no one.
pub fn own_only(caller: &AuthUser, body: &Value) -> Result<(), ServiceError> {
    match body.get("person").and_then(Value::as_str) {
        Some(p) if !p.trim().eq_ignore_ascii_case(&caller.eppn) => Err(ServiceError::forbidden(
            "request_access explains and files for you only. Another person's access is not explained, because it would show their attributes, and no steward role exists yet to allow it for a dataset. Ask the administrators of the unit",
        )),
        _ => Ok(()),
    }
}

/// Why the caller can or cannot do what a request would ask for, in words, with the rule and the facts used. It files
/// nothing and holds no record value: it is built from roles, rules and the names of the things asked about. The
/// grants and policy come from the access explanation of phase 7 (`access_review::explain`).
pub fn explain(state: &SharedState, caller: &AuthUser, body: &Value) -> Result<Value, ServiceError> {
    own_only(caller, body)?;
    let (body, _) = catalog_input(state, caller, body)?;
    let mut filled = body;
    for (key, fill) in [("reason", json!("explain")), ("days", json!(1))] {
        if filled.get(key).is_none_or(Value::is_null) {
            filled[key] = fill;
        }
    }
    let input = parse(state, &filled)?;
    let obj = find(state, &input)?;
    let review = crate::service::access_review::explain(state, caller, &caller.eppn, &obj.anchor(&input))?;
    let reached = review["role"].as_str().and_then(Role::parse);
    let held = held_role(state, caller, &obj, &input).into_iter().chain(reached).max();
    let allowed = held.is_some_and(|mine| mine >= input.role);
    let policy = &review["policy"];
    let deciding = serde_json::from_value::<scaffoldry_policy::PolicyRef>(policy["deciding"].clone()).ok();
    let reasons: Vec<String> = policy["reasons"].as_array().map(|r| r.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let rule = if allowed {
        json!({ "id": "role_held", "description": "You hold this role, or a higher one, through a grant or a membership." })
    } else if policy["allowed"] == json!(false) {
        super::forbidden_rule(&reasons, deciding.as_ref(), true)
    } else {
        json!({ "id": "role_required", "description": "The role you hold is lower than the one asked about." })
    };
    let mut axes: Vec<&str> = review["missed"].as_array().map(|m| m.iter().filter_map(|g| g["axis"].as_str()).collect()).unwrap_or_default();
    axes.sort_unstable();
    axes.dedup();
    let a = assess(state, &caller.eppn, &caller.affiliation, &obj, input.role, input.clearance.as_deref(), input.days);
    let would = match approving_rule(state, &obj, &a.facts)? {
        Some(id) => json!({ "route": "auto", "rule_id": id }),
        None => json!({ "route": "approver", "checks": a.raised }),
    };
    let on = obj.sentence();
    let mut words = match (allowed, held) {
        (true, Some(mine)) => format!("You hold {} on {on}, which covers {}. You can do this without asking.", mine.as_str(), input.role.as_str()),
        (_, Some(mine)) => format!("You hold {} on {on}, and {} is more. {}", mine.as_str(), input.role.as_str(), rule["description"].as_str().unwrap_or_default()),
        (_, None) => format!("You hold no role on {on}. {}", rule["description"].as_str().unwrap_or_default()),
    };
    if !allowed {
        if !axes.is_empty() {
            words.push_str(&format!(" Grants you hold did not reach it: {}.", axes.join(", ")));
        }
        words.push_str(" To ask for it, call request_access again without explain, with a reason and days. A person confirms it, then a rule or an approver decides.");
    }
    Ok(json!({
        "allowed": allowed, "filed": false, "object": on, "asked_role": input.role.as_str(), "held_role": held.map(Role::as_str),
        "rule": rule, "explanation": words,
        "facts": { "sensitivity": a.facts.sensitivity, "visibility": a.facts.visibility, "in_own_unit": a.facts.in_own_unit, "days": a.facts.days, "grants_missed_on": axes },
        "would_route": would,
        "guidance": { "next": "Tell the person why, in these words. To ask for the access, call request_access again without explain.", "avoid": "Do not explain another person's access, and do not read this as a grant." },
    }))
}

fn row_of(state: &SharedState, id: Uuid) -> Result<RequestRow, ServiceError> {
    state.repository.get_request(id).map_err(internal)?.ok_or_else(|| ServiceError::not_found("Request not found"))
}

/// The person's own request, now that a person is behind it: a rule may approve it, or it goes to the people who decide.
fn settle(state: &SharedState, caller: &AuthUser, req: &RequestRow, obj: &Object, a: &Assessed) -> Result<Value, ServiceError> {
    let from = [req.status.as_str()];
    match approving_rule(state, obj, &a.facts)? {
        Some(rule) => approve(state, caller, req, obj, Way { from: &from, route: "auto", rule: Some(rule), note: &format!("Approved by rule {rule}: {}", req.reason) }),
        None => {
            if req.status == WAITING {
                state.repository.confirm_request(req.id, "approver", "Pending").map_err(internal)?;
            }
            let stored = row_of(state, req.id)?;
            tell_waiting(state, &stored, false);
            view(state, &stored)
        }
    }
}

/// The person confirms a request an AI tool filed in their name. Only the person can: not a tool, and not someone else.
pub fn confirm(state: &SharedState, caller: &AuthUser, origin: &Origin, id: Uuid) -> Result<Value, ServiceError> {
    let req = row_of(state, id).ok().filter(|r| r.requester.eq_ignore_ascii_case(&caller.eppn)).ok_or_else(|| ServiceError::not_found("Request not found"))?;
    if origin.session.is_some() || origin.kind == crate::repository::ActorKind::Agent {
        return Err(ServiceError::forbidden("A tool cannot confirm a request for the person. Confirm it in the desk"));
    }
    if req.status != WAITING {
        return Err(ServiceError::conflict(format!("That request is {}", req.status)));
    }
    let input = input_of(&req)?;
    let obj = find(state, &input)?;
    if req.route == "self" {
        // The role may have gone since the tool asked. If it has, the request is an ordinary one.
        if held_role(state, caller, &obj, &input).is_some_and(|mine| mine >= input.role) {
            return approve(state, caller, &req, &obj, Way { from: &[WAITING], route: "self", rule: None, note: &format!("Confirmed by the person: {}", req.reason) });
        }
    }
    let a = assess(state, &caller.eppn, &caller.affiliation, &obj, input.role, input.clearance.as_deref(), input.days);
    settle(state, caller, &req, &obj, &a)
}

/// An approver decides. The requester never does, and the people the matrix names are the only ones who can.
pub fn decide(state: &SharedState, caller: &AuthUser, id: Uuid, body: &Value) -> Result<Value, ServiceError> {
    let req = row_of(state, id).map_err(|_| ServiceError::not_found("Request not found"))?;
    if req.requester.eq_ignore_ascii_case(&caller.eppn) {
        return Err(ServiceError::forbidden("You cannot decide your own request"));
    }
    let input = input_of(&req)?;
    let obj = find(state, &input)?;
    let a = assess(state, &req.requester, "", &obj, input.role, input.clearance.as_deref(), input.days);
    let need = need_for(state, &obj, &req.requester, &a.raised)?;
    if !may_decide(state, caller, &req, &obj, &need) {
        let who = waiting_on(state, &req, &obj, &need);
        return Err(ServiceError::forbidden(if who.is_empty() { "Nobody can decide this request yet: the approver it needs resolves to no one".to_string() } else { format!("You are not one of the people who decide this request: {}", who.join(", ")) }));
    }
    let decision = match body.get("decision").and_then(Value::as_str) {
        Some("approve") => true,
        Some("reject") => false,
        _ => return Err(ServiceError::bad_request("decision must be 'approve' or 'reject'")),
    };
    let mut p = Problems::default();
    let note = p.text(body, "note", 500, true);
    p.done()?;
    let note = note.unwrap_or_default();
    if req.status != "Pending" {
        return Err(ServiceError::conflict(format!("That request is {}", req.status)));
    }
    if decision {
        return approve(state, caller, &req, &obj, Way { from: &["Pending"], route: "approver", rule: None, note: &note });
    }
    let payload = ledger_payload(&req, "Rejected", "approver", None, &caller.eppn, None);
    admin_write(state, caller, DecisionType::AccessRequestDecided, OSCAL_CONTROL, &note, &payload, || {
        state.repository.close_request(req.id, &["Pending"], "Rejected", &caller.eppn, &note).map_err(internal)
    })?
    .then_some(())
    .ok_or_else(|| ServiceError::conflict("That request has already been decided"))?;
    view(state, &row_of(state, id)?)
}

/// The requester withdraws a request that is still open.
pub fn withdraw(state: &SharedState, caller: &AuthUser, id: Uuid) -> Result<Value, ServiceError> {
    let req = row_of(state, id).ok().filter(|r| r.requester.eq_ignore_ascii_case(&caller.eppn)).ok_or_else(|| ServiceError::not_found("Request not found"))?;
    let payload = ledger_payload(&req, "Withdrawn", &req.route, None, &caller.eppn, None);
    let closed = admin_write(state, caller, DecisionType::AccessRequestDecided, OSCAL_CONTROL, "The requester withdrew the request", &payload, || {
        state.repository.close_request(req.id, &["Pending", WAITING], "Withdrawn", &caller.eppn, "The requester withdrew the request").map_err(internal)
    })?;
    if !closed {
        return Err(ServiceError::conflict(format!("That request is {}", req.status)));
    }
    view(state, &row_of(state, id)?)
}

/// A person's own requests, newest first.
pub fn mine(state: &SharedState, caller: &AuthUser) -> Result<Value, ServiceError> {
    let rows = state.repository.requests_of(&caller.eppn).map_err(internal)?;
    Ok(json!({ "requests": rows.iter().map(|r| view(state, r)).collect::<Result<Vec<_>, _>>()? }))
}

/// The open requests the caller may decide, as the matrix stands now.
pub fn to_decide(state: &SharedState, caller: &AuthUser) -> Result<Value, ServiceError> {
    let mut out = Vec::new();
    for req in state.repository.pending_requests(500).map_err(internal)? {
        let Ok(input) = input_of(&req) else { continue };
        let Ok(obj) = find(state, &input) else { continue };
        let a = assess(state, &req.requester, "", &obj, input.role, input.clearance.as_deref(), input.days);
        let need = need_for(state, &obj, &req.requester, &a.raised)?;
        if may_decide(state, caller, &req, &obj, &need) {
            out.push(view(state, &req)?);
        }
    }
    Ok(json!({ "requests": out }))
}

// --- The rules ---------------------------------------------------------------------------------------------------

const ROOT: &str = "00000000-0000-0000-0000-000000000001";

fn root() -> Uuid {
    Uuid::parse_str(ROOT).unwrap_or_default()
}

/// Whether the caller may set rules for `unit`: a Platform Admin anywhere, an Org Unit Admin inside their own subtree.
fn may_set_rules(state: &SharedState, caller: &AuthUser, unit: Uuid) -> bool {
    let r = crate::service::access_grants::rights(state, caller);
    r.platform || (unit != root() && authority::unit_path(state, Some(unit)).iter().any(|u| r.admin_units.contains(u)))
}

fn rule_json(r: &RuleRow, state: &SharedState) -> Value {
    let name = state.organizations.read().ok().and_then(|o| o.get(&r.organization_id).map(|u| u.name.clone())).unwrap_or_default();
    let sentence = serde_json::from_value::<Rule>(r.rule.clone()).map(|rule| rules::sentence(&rule)).unwrap_or_default();
    json!({ "id": r.id, "organization_id": r.organization_id, "organization": name, "rule": r.rule, "sentence": sentence, "created_by": r.created_by, "created_at": r.created_at, "retired_at": r.retired_at })
}

/// The rules in force, and those retired.
pub fn list_rules(state: &SharedState, caller: &AuthUser) -> Result<Value, ServiceError> {
    let r = crate::service::access_grants::rights(state, caller);
    if !r.platform && r.admin_units.is_empty() {
        return Err(ServiceError::forbidden("Platform Admin, compliance, or Org Unit Admin required"));
    }
    state.repository.ensure_default_request_rule().map_err(internal)?;
    let all = state.repository.request_rules().map_err(internal)?;
    let (retired, active): (Vec<&RuleRow>, Vec<&RuleRow>) = all.iter().partition(|r| r.retired_at.is_some());
    Ok(json!({
        "rules": active.iter().map(|r| rule_json(r, state)).collect::<Vec<_>>(),
        "retired": retired.iter().map(|r| rule_json(r, state)).collect::<Vec<_>>(),
    }))
}

/// Why `rule` may not be set at `unit`: the faults of the rule itself, and for a unit, each condition it removes.
fn rule_faults(state: &SharedState, unit: Uuid, rule: &Rule) -> Result<(), ServiceError> {
    let mut p = Problems::default();
    for why in rules::problems(rule) {
        p.add("rule", why);
    }
    if unit != root() {
        let all = state.repository.request_rules().map_err(internal)?;
        let institution: Vec<Rule> = all.iter().filter(|r| r.retired_at.is_none() && r.organization_id == root()).filter_map(|r| serde_json::from_value(r.rule.clone()).ok()).collect();
        let nearest = institution.iter().map(|i| (rules::loosened(i, rule), i)).min_by_key(|(faults, _)| faults.len());
        match nearest {
            None => p.add("rule", "The institution has no rule in force, so a unit has nothing to add conditions to"),
            Some((faults, _)) => {
                for f in faults {
                    p.add(f, format!("A unit rule may only add conditions. {f} asks for more than the institution's rule allows"));
                }
            }
        }
    }
    p.done()
}

/// Checks a rule change as `caller` may make it and shapes the change. Nothing is stored.
pub(crate) fn build_rule_change(state: &SharedState, caller: &AuthUser, body: &Value) -> Result<(Value, String, String), ServiceError> {
    let rights = crate::service::access_grants::rights(state, caller);
    if !rights.platform && rights.admin_units.is_empty() {
        return Err(ServiceError::forbidden("Platform Admin, compliance, or Org Unit Admin required"));
    }
    let mut p = Problems::default();
    let unit = p.uuid_field(body, "organization_id");
    let reason = p.text(body, "reason", 500, true);
    p.done()?;
    let (unit, reason) = (unit.unwrap_or_default(), reason.unwrap_or_default());
    if !state.organizations.read().map_err(internal)?.contains_key(&unit) {
        return Err(ServiceError::not_found("Organization not found"));
    }
    if !may_set_rules(state, caller, unit) {
        return Err(ServiceError::GrantOutsideScope("You may not set request rules for this unit".to_string()));
    }
    state.repository.ensure_default_request_rule().map_err(internal)?;
    let (change, sentence) = if let Some(rule_id) = body.get("retire").and_then(Value::as_str) {
        let rule_id = Uuid::parse_str(rule_id).map_err(|_| ServiceError::bad_request("retire is a rule id"))?;
        let found = state.repository.request_rules().map_err(internal)?.into_iter().find(|r| r.id == rule_id && r.organization_id == unit && r.retired_at.is_none());
        let Some(found) = found else { return Err(ServiceError::not_found("Rule not found")) };
        let sentence = serde_json::from_value::<Rule>(found.rule.clone()).map(|r| format!("Retire this rule: {}", rules::sentence(&r))).unwrap_or_else(|_| "Retire a request rule.".to_string());
        (json!({ "kind": "request_rule", "op": "retire", "organization_id": unit, "rule_id": rule_id }), sentence)
    } else {
        let rule: Rule = serde_json::from_value(body.get("rule").cloned().unwrap_or(Value::Null)).map_err(|e| {
            let mut p = Problems::default();
            p.add("rule", format!("rule is not a request rule: {e}"));
            p.done().err().unwrap_or_else(|| ServiceError::bad_request("rule is not a request rule"))
        })?;
        rule_faults(state, unit, &rule)?;
        let sentence = format!("Add this rule: {}", rules::sentence(&rule));
        (json!({ "kind": "request_rule", "op": "add", "organization_id": unit, "rule": rule }), sentence)
    };
    Ok((change, reason, sentence))
}

/// A rule change waits for a second person, like every change to who may see what.
pub fn propose_rule(state: &SharedState, caller: &AuthUser, body: &Value) -> Result<Value, ServiceError> {
    let (change, reason, _) = build_rule_change(state, caller, body)?;
    let id = state.repository.propose_access_change(&change, true, &reason, &caller.eppn).map_err(internal)?;
    Ok(json!({ "change_id": id, "status": "Pending", "message": "A different person with the same right must confirm this rule change before it takes effect." }))
}

/// Confirms or rejects a stored rule change. The proposer cannot confirm. The rule is checked again against the rules in
/// force now.
pub fn decide_rule_change(state: &SharedState, caller: &AuthUser, id: Uuid, change: &Value, proposed_by: &str, confirm: bool, reason: &str) -> Result<Value, ServiceError> {
    let unit = change["organization_id"].as_str().and_then(|u| Uuid::parse_str(u).ok()).ok_or_else(|| internal("The stored change has no unit"))?;
    if !may_set_rules(state, caller, unit) {
        return Err(ServiceError::forbidden("You do not hold the right to decide this change"));
    }
    if confirm && proposed_by.eq_ignore_ascii_case(&caller.eppn) {
        return Err(ServiceError::forbidden("The person who proposed a change cannot confirm it"));
    }
    if confirm && change["op"] == "add" {
        let rule: Rule = serde_json::from_value(change["rule"].clone()).map_err(internal)?;
        rule_faults(state, unit, &rule)?;
    }
    let decide = || state.repository.decide_access_change(id, &caller.eppn, confirm).map_err(internal);
    let outcome = if confirm {
        let payload = json!({ "action": "request_rule_confirmed", "change_id": id, "change": change, "proposed_by": proposed_by, "proposer": crate::service::access_drafts::proposer(state, id) });
        admin_write(state, caller, DecisionType::AccessGrantChanged, OSCAL_CONTROL, reason, &payload, decide)?
    } else {
        decide()?
    };
    use crate::repository_grants::ChangeDecision;
    match outcome {
        ChangeDecision::Done { .. } => Ok(json!({ "change_id": id, "status": if confirm { "Confirmed" } else { "Rejected" }, "grant_id": Value::Null })),
        ChangeDecision::Missing => Err(ServiceError::NotFound("Change not found".to_string())),
        ChangeDecision::AlreadyDecided => Err(ServiceError::conflict("That change has already been decided")),
        ChangeDecision::OwnChange => Err(ServiceError::forbidden("The person who proposed a change cannot confirm it")),
        ChangeDecision::Expired => Err(ServiceError::conflict("That draft expired. A draft made by an AI tool cannot be confirmed after seven days. Ask for it again")),
    }
}
