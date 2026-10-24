//! Registered outside apps and their ceilings (external apps phase 1).
//!
//! A client is proposed with a ceiling: per app, per table, the fields it may read and write and whether it may create
//! or delete. An owner or admin decides. A ceiling never names a sensitive field. The ceiling is shown as sentences by
//! `sentences`, which the consent screen and the console both use. Every proposal, decision and disablement is written
//! to the ledger before the change.

use crate::repository_clients::ClientRow;
use crate::service::identity::hash_token;
use crate::service::ServiceError;
use crate::state::{AuthUser, RecordDecisionInput, SharedState};
use scaffoldry_core::DecisionType;
use scaffoldry_engine::{effective_ferpa_sensitive, AppManifest, HostRouter};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

const MAX_APPS: usize = 20;
const MAX_REDIRECTS: usize = 10;
const MAX_URI: usize = 2048;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableCeiling {
    pub table_id: String,
    #[serde(default)]
    pub read: Vec<String>,
    #[serde(default)]
    pub write: Vec<String>,
    #[serde(default)]
    pub create: bool,
    #[serde(default)]
    pub delete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppCeiling {
    pub app_slug: String,
    pub tables: Vec<TableCeiling>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ceiling {
    pub apps: Vec<AppCeiling>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    pub name: String,
    pub operator: String,
    pub kind: String,
    pub redirect_uris: Vec<String>,
    /// An agent client has none: it acts within the person's own rights.
    #[serde(default)]
    pub ceiling: Ceiling,
}

/// A new client secret. The `scs_` prefix tells a person and a scanner what it is.
fn new_client_secret() -> String {
    format!("scs_{:x}{:x}", uuid::Uuid::new_v4().as_simple(), uuid::Uuid::new_v4().as_simple())
}

fn internal(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

fn live(slug: &str, state: &SharedState) -> Option<AppManifest> {
    state.engine.read().ok().and_then(|e| e.resolve_by_slug(slug).cloned())
}

/// The role a person has in a workspace. A Platform Admin counts as an admin everywhere.
fn role_in(caller: &AuthUser, workspace: &str, state: &SharedState) -> Option<String> {
    if caller.affiliation == "central_admin" {
        return Some("admin".to_string());
    }
    crate::service::members::member_role(caller, workspace, state).map(|r| r.as_str().to_string())
}

fn is_platform_admin(caller: &AuthUser) -> bool {
    caller.affiliation == "central_admin"
}

/// The workspaces a ceiling's apps belong to, once each.
fn workspaces_of(ceiling: &Ceiling, state: &SharedState) -> BTreeSet<String> {
    ceiling.apps.iter().filter_map(|a| live(&a.app_slug, state)).filter_map(|m| m.workspace_id).collect()
}

/// A ceiling that writes, or that reaches more than one workspace, needs a second person.
fn needs_second_person(ceiling: &Ceiling, state: &SharedState) -> bool {
    let writes = ceiling.apps.iter().flat_map(|a| &a.tables).any(|t| !t.write.is_empty() || t.create || t.delete);
    writes || workspaces_of(ceiling, state).len() > 1
}

/// Checks a redirect URI. Exact `https` addresses only. `http` is allowed for a localhost address when the setting says so.
fn check_redirect(uri: &str, allow_localhost: bool) -> Result<(), String> {
    let bad = |why: &str| Err(format!("The redirect address '{uri}' {why}"));
    if uri.is_empty() || uri.len() > MAX_URI || uri.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return bad("is not a valid address.");
    }
    if uri.contains('*') {
        return bad("has a wildcard. A redirect address must match exactly.");
    }
    if uri.contains('?') || uri.contains('#') {
        return bad("has a query string or a fragment. A redirect address must not.");
    }
    let (secure, rest) = if let Some(rest) = uri.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = uri.strip_prefix("http://") {
        (false, rest)
    } else {
        return bad("must start with https://.");
    };
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return bad("has no host, or has a user name in it.");
    }
    let host = authority.rsplit_once(':').filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit())).map_or(authority, |(h, _)| h);
    if !secure {
        let local = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
        if !local {
            return bad("must use https://. Only a localhost address may use http://.");
        }
        if !allow_localhost {
            return bad("is a localhost address, which this server does not allow.");
        }
    }
    Ok(())
}

/// Checks a ceiling against the live apps. A field must exist and must not be sensitive.
fn check_ceiling(ceiling: &Ceiling, state: &SharedState) -> Result<(), ServiceError> {
    if ceiling.apps.is_empty() || ceiling.apps.len() > MAX_APPS {
        return Err(ServiceError::bad_request(format!("A ceiling must name between 1 and {MAX_APPS} apps")));
    }
    let categories = state.sensitivity_categories();
    let labels = state.labels.read().unwrap_or_else(|p| p.into_inner()).clone();
    let mut seen = BTreeSet::new();
    for app in &ceiling.apps {
        if !seen.insert(&app.app_slug) {
            return Err(ServiceError::bad_request(format!("The app '{}' appears twice in the ceiling", app.app_slug)));
        }
        let manifest = live(&app.app_slug, state).ok_or_else(|| ServiceError::bad_request(format!("The app '{}' does not exist", app.app_slug)))?;
        if manifest.workspace_id.is_none() {
            return Err(ServiceError::bad_request(format!("The app '{}' belongs to no workspace", app.app_slug)));
        }
        if app.tables.is_empty() {
            return Err(ServiceError::bad_request(format!("The ceiling names no table of '{}'", app.app_slug)));
        }
        for t in &app.tables {
            let table = manifest
                .tables
                .iter()
                .find(|x| x.id == t.table_id)
                .ok_or_else(|| ServiceError::bad_request(format!("The app '{}' has no table '{}'", app.app_slug, t.table_id)))?;
            for name in t.read.iter().chain(t.write.iter()) {
                let field = table
                    .fields
                    .iter()
                    .find(|f| &f.name == name)
                    .ok_or_else(|| ServiceError::bad_request(format!("The table '{}' has no field '{name}'", t.table_id)))?;
                let label = labels.get(&(app.app_slug.clone(), table.id.clone(), name.clone()));
                if effective_ferpa_sensitive(field, label, &categories).is_protected() {
                    return Err(ServiceError::ClientSensitiveField(format!(
                        "The field '{name}' of '{}' is sensitive. An outside app cannot be given a sensitive field.",
                        table.name
                    )));
                }
            }
        }
    }
    Ok(())
}

fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [a, b] => format!("{a} and {b}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

/// The ceiling as sentences, such as "Read the title and status of Proposals in Grants Office." One function, for the
/// consent screen and the console. An app or table that has gone is named by its id.
pub fn sentences(ceiling: &Ceiling, state: &SharedState) -> Vec<String> {
    let mut out = Vec::new();
    for app in &ceiling.apps {
        let manifest = live(&app.app_slug, state);
        let app_title = manifest.as_ref().map(|m| m.title.clone()).unwrap_or_else(|| app.app_slug.clone());
        for t in &app.tables {
            let table = manifest.as_ref().and_then(|m| m.tables.iter().find(|x| x.id == t.table_id));
            let table_name = table.map(|x| x.name.clone()).unwrap_or_else(|| t.table_id.clone());
            let label_of = |name: &String| table.and_then(|x| x.fields.iter().find(|f| &f.name == name)).map(|f| f.label.clone()).unwrap_or_else(|| name.clone());
            if !t.read.is_empty() {
                out.push(format!("Read the {} of {table_name} in {app_title}.", join_and(&t.read.iter().map(label_of).collect::<Vec<_>>())));
            }
            if !t.write.is_empty() {
                out.push(format!("Change the {} of {table_name} in {app_title}.", join_and(&t.write.iter().map(label_of).collect::<Vec<_>>())));
            }
            if t.create {
                out.push(format!("Create {table_name} in {app_title}."));
            }
            if t.delete {
                out.push(format!("Delete {table_name} in {app_title}."));
            }
        }
    }
    out
}

/// A client row, for functions that show its ceiling.
pub struct ClientView<'a>(pub &'a ClientRow);

/// The ceiling of a stored client as sentences. The consent screen and the console both call this.
pub fn ceiling_sentences(client: &ClientView<'_>, state: &SharedState) -> Vec<String> {
    sentences_of(client.0, state)
}

/// What an agent client may do is chosen by each person at sign-in, so its sentences say how it works.
fn sentences_of(row: &ClientRow, state: &SharedState) -> Vec<String> {
    if row.kind == "agent" {
        return vec![
            "Acts as you, within what you may already do. It never has more than you.".to_string(),
            "You choose what it may do each time it signs in: read, propose changes, or write records.".to_string(),
            "It may read sensitive fields you may read. Those reads are recorded.".to_string(),
        ];
    }
    sentences(&ceiling_of(row), state)
}

/// A stored client's ceiling. One that does not parse is empty, which allows nothing.
pub fn ceiling_of(row: &ClientRow) -> Ceiling {
    serde_json::from_value(row.ceiling.clone()).unwrap_or(Ceiling { apps: Vec::new() })
}

/// The token scope a ceiling comes to. A client token is a narrowed token with this scope (integrations phase 2), and
/// the fields come from the same ceiling through `field_limits`. A table that reads, writes, creates or deletes nothing
/// is left out, so an empty ceiling is a scope that allows nothing.
pub fn scope_of(ceiling: &Ceiling) -> scaffoldry_core::TokenScope {
    use scaffoldry_core::TokenOp;
    let mut scope = scaffoldry_core::TokenScope { apps: Vec::new(), tables: Default::default(), tools: None, max_rows_per_call: None, level: None };
    for app in &ceiling.apps {
        for table in &app.tables {
            let ops: Vec<TokenOp> = [
                (!table.read.is_empty(), TokenOp::Read),
                (!table.write.is_empty(), TokenOp::Update),
                (table.create, TokenOp::Create),
                (table.delete, TokenOp::Delete),
            ]
            .into_iter()
            .filter_map(|(on, op)| on.then_some(op))
            .collect();
            if ops.is_empty() {
                continue;
            }
            if !scope.apps.contains(&app.app_slug) {
                scope.apps.push(app.app_slug.clone());
            }
            scope.tables.entry(table.table_id.clone()).or_default().extend(ops);
        }
    }
    if scope.apps.is_empty() {
        // Nothing to name. A scope with no tools allows no tool, and no app allows no table.
        scope.tools = Some(Vec::new());
    }
    scope
}

/// The fields of `table` in `app` the ceiling does not let the client read, and those it does not let it change.
pub fn field_limits(ceiling: &Ceiling, app: &str, table: &str, fields: &[String]) -> (Vec<String>, Vec<String>) {
    let allowed = ceiling.apps.iter().filter(|a| a.app_slug == app).flat_map(|a| &a.tables).find(|t| t.table_id == table);
    let outside = |pick: fn(&TableCeiling) -> &Vec<String>| -> Vec<String> {
        fields.iter().filter(|f| !allowed.is_some_and(|t| pick(t).contains(f))).cloned().collect()
    };
    (outside(|t| &t.read), outside(|t| &t.write))
}

/// A client as the API shows it. It never holds a secret or a hash of one.
pub fn view(row: &ClientRow, state: &SharedState) -> Value {
    json!({
        "client_id": row.client_id,
        "name": row.name,
        "operator": row.operator,
        "kind": row.kind,
        "redirect_uris": row.redirect_uris,
        "ceiling": row.ceiling,
        "sentences": sentences_of(row, state),
        "status": row.status,
        "proposed_by": row.proposed_by,
        "proposed_at": row.proposed_at,
        "decided_by": row.decided_by,
        "decided_at": row.decided_at,
        "disabled_by": row.disabled_by,
        "disabled_at": row.disabled_at,
    })
}

fn ledger(caller: &AuthUser, row: &ClientRow, kind: DecisionType, control: &str, rationale: String, extra: Value, state: &SharedState) -> Result<(), ServiceError> {
    let ceiling = ceiling_of(row);
    let first = ceiling.apps.first().and_then(|a| live(&a.app_slug, state));
    let payload = json!({
        "client_id": row.client_id, "name": row.name, "operator": row.operator, "kind": row.kind,
        "ceiling": row.ceiling, "by": caller.eppn, "detail": extra
    });
    state
        .append_ledger_entry(RecordDecisionInput {
            principal: caller.eppn.clone(),
            organization_code: first.as_ref().map(|m| m.organization_code.clone()).unwrap_or_default(),
            app_slug: ceiling.apps.first().map(|a| a.app_slug.clone()),
            decision_type: kind,
            oscal_control_id: control.to_string(),
            rationale,
            payload: &payload,
        })
        .map(|_| ())
        .map_err(|e| ServiceError::internal(format!("Could not record the ledger entry: {e}")))
}

/// Whether the person may see a client: the one who proposed it, an owner or admin of a workspace it names, or a Platform Admin.
fn may_see(caller: &AuthUser, row: &ClientRow, state: &SharedState) -> bool {
    if is_platform_admin(caller) || row.proposed_by.eq_ignore_ascii_case(&caller.eppn) {
        return true;
    }
    workspaces_of(&ceiling_of(row), state).iter().any(|w| matches!(role_in(caller, w, state).as_deref(), Some("owner" | "admin")))
}

pub fn propose(caller: &AuthUser, input: Registration, state: &SharedState) -> Result<Value, ServiceError> {
    let (name, operator) = (input.name.trim(), input.operator.trim());
    if name.is_empty() || name.len() > 255 {
        return Err(ServiceError::bad_request("Argument 'name' must be 1 to 255 characters"));
    }
    if operator.is_empty() || operator.len() > 255 {
        return Err(ServiceError::bad_request("Argument 'operator' must name who runs the app, in 1 to 255 characters"));
    }
    if !matches!(input.kind.as_str(), "public" | "confidential" | "agent") {
        return Err(ServiceError::bad_request("Argument 'kind' must be public, confidential or agent"));
    }
    // An AI tool the institution allows. Only a Platform Admin registers one, and it has no ceiling of its own.
    if input.kind == "agent" {
        if !is_platform_admin(caller) {
            return Err(ServiceError::forbidden("Forbidden: only a Platform Admin registers an AI tool as an agent client"));
        }
        if !input.ceiling.apps.is_empty() {
            return Err(ServiceError::bad_request("An agent client has no ceiling: it acts within the person's own rights"));
        }
    }
    if input.redirect_uris.is_empty() || input.redirect_uris.len() > MAX_REDIRECTS {
        return Err(ServiceError::bad_request(format!("A client needs between 1 and {MAX_REDIRECTS} redirect addresses")));
    }
    let allow_localhost = state.settings.read().ok().and_then(|s| s.get("oauth.allow_localhost").and_then(Value::as_bool)).unwrap_or(false);
    for uri in &input.redirect_uris {
        check_redirect(uri, allow_localhost).map_err(ServiceError::bad_request)?;
    }
    if input.kind != "agent" {
        check_ceiling(&input.ceiling, state)?;
    }
    // The person must be able to propose in every workspace the ceiling names.
    for ws in workspaces_of(&input.ceiling, state) {
        if !matches!(role_in(caller, &ws, state).as_deref(), Some("owner" | "admin" | "editor")) {
            return Err(ServiceError::forbidden("Forbidden: registering a client needs the owner, admin, or editor role in every workspace it names"));
        }
    }

    let row = ClientRow {
        client_id: format!("scc_{}", uuid::Uuid::new_v4().simple()),
        name: name.to_string(),
        operator: operator.to_string(),
        kind: input.kind,
        redirect_uris: input.redirect_uris,
        ceiling: serde_json::to_value(&input.ceiling).map_err(internal)?,
        status: "Pending".to_string(),
        proposed_by: caller.eppn.clone(),
        proposed_at: String::new(),
        decided_by: None,
        decided_at: None,
        disabled_by: None,
        disabled_at: None,
    };
    ledger(caller, &row, DecisionType::ClientProposed, "CM-03", format!("{} proposed the outside app '{}' run by {}", caller.name, row.name, row.operator), json!({}), state)?;
    state.repository.insert_client(&row).map_err(internal)?;
    let stored = state.repository.get_client(&row.client_id).map_err(internal)?.ok_or_else(|| ServiceError::internal("The client was not stored"))?;
    Ok(view(&stored, state))
}

pub fn list(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    let rows = state.repository.list_clients().map_err(internal)?;
    Ok(Value::Array(rows.iter().filter(|r| may_see(caller, r, state)).map(|r| view(r, state)).collect()))
}

fn load(caller: &AuthUser, id: &str, state: &SharedState) -> Result<ClientRow, ServiceError> {
    let row = state.repository.get_client(id).map_err(internal)?.filter(|r| may_see(caller, r, state));
    row.ok_or_else(|| ServiceError::not_found("Client not found"))
}

pub fn get(caller: &AuthUser, id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    Ok(view(&load(caller, id, state)?, state))
}

/// Approves or rejects a pending client. On the approval of a confidential client the secret is made and returned
/// once. Only its hash is stored.
pub fn decide(caller: &AuthUser, id: &str, decision: &str, rationale: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let row = load(caller, id, state)?;
    let approve = match decision {
        "approve" => true,
        "reject" => false,
        _ => return Err(ServiceError::bad_request("Argument 'decision' must be approve or reject")),
    };
    let ceiling = ceiling_of(&row);
    let workspaces = workspaces_of(&ceiling, state);
    let roles: Vec<Option<String>> = workspaces.iter().map(|w| role_in(caller, w, state)).collect();
    let agent = row.kind == "agent";
    if agent {
        if !is_platform_admin(caller) {
            return Err(ServiceError::forbidden("Forbidden: only a Platform Admin decides an agent client"));
        }
    } else if roles.is_empty() || !roles.iter().all(|r| matches!(r.as_deref(), Some("owner" | "admin"))) {
        return Err(ServiceError::forbidden("Forbidden: deciding a client needs the owner or admin role in every workspace it names"));
    }
    let is_proposer = row.proposed_by.eq_ignore_ascii_case(&caller.eppn);
    if is_proposer && !agent {
        if needs_second_person(&ceiling, state) {
            return Err(ServiceError::forbidden("Forbidden: a client that writes, or reaches more than one workspace, needs a second person to decide it"));
        }
        if !is_platform_admin(caller) && !roles.iter().all(|r| r.as_deref() == Some("owner")) {
            return Err(ServiceError::forbidden("Forbidden: a person may decide their own client only when they own the workspace"));
        }
    }
    if row.status != "Pending" {
        return Err(ServiceError::Conflict(format!("This client is already {}", row.status)));
    }
    if rationale.trim().is_empty() {
        return Err(ServiceError::bad_request("Argument 'rationale' must say why"));
    }

    let secret = (approve && row.kind == "confidential").then(new_client_secret);
    let hash = secret.as_deref().map(hash_token);
    let verb = if approve { "approved" } else { "rejected" };
    ledger(caller, &row, DecisionType::ClientDecided, "CM-03", format!("{} {verb} the outside app '{}': {}", caller.name, row.name, rationale.trim()), json!({ "decision": decision, "rationale": rationale.trim() }), state)?;
    if !state.repository.decide_client(&row.client_id, approve, &caller.eppn, hash.as_deref()).map_err(internal)? {
        return Err(ServiceError::Conflict("This client was decided by someone else a moment ago".to_string()));
    }
    let stored = state.repository.get_client(&row.client_id).map_err(internal)?.ok_or_else(|| ServiceError::internal("The client is gone"))?;
    let mut out = view(&stored, state);
    if let Some(secret) = secret {
        out["client_secret"] = json!(secret);
    }
    Ok(out)
}

/// Cuts a client off for everyone. A Platform Admin, or an owner of a workspace it names.
pub fn disable(caller: &AuthUser, id: &str, reason: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let row = load(caller, id, state)?;
    let owns = workspaces_of(&ceiling_of(&row), state).iter().any(|w| role_in(caller, w, state).as_deref() == Some("owner"));
    if !is_platform_admin(caller) && !owns {
        return Err(ServiceError::forbidden("Forbidden: disabling a client needs a Platform Admin or an owner of a workspace it names"));
    }
    if row.status != "Active" {
        return Err(ServiceError::Conflict(format!("This client is {}, not active", row.status)));
    }
    ledger(caller, &row, DecisionType::ClientDisabled, "AC-02", format!("{} disabled the outside app '{}': {}", caller.name, row.name, reason.trim()), json!({ "reason": reason.trim() }), state)?;
    if !state.repository.disable_client(&row.client_id, &caller.eppn).map_err(internal)? {
        return Err(ServiceError::Conflict("This client was disabled a moment ago".to_string()));
    }
    let stored = state.repository.get_client(&row.client_id).map_err(internal)?.ok_or_else(|| ServiceError::internal("The client is gone"))?;
    Ok(view(&stored, state))
}
