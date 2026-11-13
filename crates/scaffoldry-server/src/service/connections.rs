//! Connections phase 1: the rules behind the connection routes. A Platform Admin creates and changes connections, reads
//! their runs, and runs one now. Each change carries a reason and a `ConnectionChanged` ledger entry. Creating a
//! connection also creates the dataset it fills: an app of kind `dataset` in the system workspace.

use crate::connections::{self, RunError, COLUMN_TYPES};
use crate::service::admin::{admin_write, require_platform_admin};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_engine::HostRouter;
use serde_json::{json, Value};
use uuid::Uuid;

/// The workspace that holds every dataset. Only Platform Admins own it.
pub const SYSTEM_WORKSPACE: &str = "system-datasets";

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

fn bad(m: impl Into<String>) -> ServiceError {
    ServiceError::bad_request(m)
}

fn reason_of(body: &Value) -> String {
    body.get("reason").and_then(Value::as_str).unwrap_or("").to_string()
}

fn id_of(id: &str) -> Result<Uuid, ServiceError> {
    Uuid::parse_str(id).map_err(|_| ServiceError::NotFound("Connection not found".to_string()))
}

fn slug_ok(s: &str, max: usize, dashes: bool) -> bool {
    !s.is_empty() && s.len() <= max && s.starts_with(|c: char| c.is_ascii_lowercase()) && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || (dashes && c == '-'))
}

const SENSITIVITIES: [&str; 5] = ["Public", "Directory", "Internal", "Restricted", "Confidential"];

fn check_schedule(body: &Value) -> Result<(Option<i32>, Option<String>), ServiceError> {
    let every = match body.get("every_minutes") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_i64().filter(|n| (1..=10_080).contains(n)).ok_or_else(|| bad("every_minutes is a number from 1 to 10080"))? as i32),
    };
    let at = match body.get("at_time") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let t = v.as_str().unwrap_or("");
            chrono::NaiveTime::parse_from_str(t, "%H:%M").ok().filter(|_| t.len() == 5).ok_or_else(|| bad("at_time is a time of day such as 02:30"))?;
            Some(t.to_string())
        }
    };
    if every.is_some() && at.is_some() {
        return Err(bad("A connection runs every so many minutes or once a day at a time, not both"));
    }
    Ok((every, at))
}

fn check_config(kind: &str, config: &Value) -> Result<(), ServiceError> {
    match kind {
        "file_drop" => {
            let path = config.get("path").and_then(Value::as_str).unwrap_or("");
            connections::check_name(path).map_err(bad)?;
            match config.get("format").and_then(Value::as_str) {
                Some("csv" | "jsonl") => Ok(()),
                _ => Err(bad("format is csv or jsonl")),
            }
        }
        "postgres" => crate::connection_pg::check_config(config).map_err(bad),
        other => Err(bad(format!("The connection kind '{other}' is not available yet. Use file_drop or postgres"))),
    }
}

/// The connection as the routes show it, with its newest run. A secret is named and never shown.
fn show(state: &SharedState, id: Uuid) -> Result<Value, ServiceError> {
    // A change approved since the last run, by a job that finished its snapshot, reaches the connection here too.
    crate::connection_drift::settle_by_id(state, id);
    let row = state
        .repository
        .with_client(move |c| {
            Ok(c.query_opt(
                "SELECT json_build_object('id', id, 'name', name, 'kind', kind, 'config', config, 'secret', secret, 'dataset_app', dataset_app, 'dataset_table', dataset_table, \
                        'key_field', key_field, 'column_map', column_map, 'sensitivity', sensitivity, 'every_minutes', every_minutes, 'at_time', at_time, 'enabled', enabled, \
                        'created_by', created_by, 'created_at', created_at, 'pending_proposal', pending_proposal) FROM connections WHERE id = $1",
                &[&id],
            )?
            .map(|r| r.get::<_, Value>(0)))
        })
        .map_err(storage)?
        .ok_or_else(|| ServiceError::NotFound("Connection not found".to_string()))?;
    let mut v = row;
    v["last_run"] = connections::runs(&state.repository, id).map_err(storage)?.into_iter().next().unwrap_or(Value::Null);
    // What the console says: a connection a changed source stopped needs a person to review the proposal.
    let (mut status, mut note) = (if v["enabled"] == false { "Disabled" } else { "Active" }, Value::Null);
    // A drift that made no proposal (a preview is running, or the key column is gone) is also waiting on a person.
    let drift_error = v["last_run"]["error"].as_str().filter(|e| e.starts_with("schema_drift") && !e.contains("Proposal ")).map(str::to_string);
    if let Some(pid) = v["pending_proposal"].as_str().and_then(|p| Uuid::parse_str(p).ok()) {
        let rejected = state.repository.get_proposal(pid).ok().flatten().is_some_and(|r| r.status.eq_ignore_ascii_case("rejected"));
        status = "Needs review";
        note = json!(if rejected { format!("The proposal {pid} for the change in the source was rejected. The connection stays stopped.") } else { format!("The proposal {pid} for the change in the source is waiting for approval.") });
    } else if let Some(e) = drift_error {
        status = "Needs review";
        note = json!(e);
    }
    v["state"] = json!(status);
    v["review_note"] = note;
    Ok(v)
}

pub fn list(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let ids: Vec<Uuid> = state.repository.with_client(|c| Ok(c.query("SELECT id FROM connections ORDER BY name", &[])?.iter().map(|r| r.get(0)).collect())).map_err(storage)?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        out.push(show(state, id)?);
    }
    Ok(json!({ "connections": out }))
}

/// The manifest of the dataset a connection fills: one table, a field for each mapped column, each carrying the
/// connection's sensitivity.
fn dataset_manifest(name: &str, app: &str, table: &str, key: &str, columns: &[connections::Column], sensitivity: &str) -> Result<scaffoldry_engine::AppManifest, ServiceError> {
    let sensitive = connections::is_sensitive(sensitivity);
    let fields: Vec<Value> = columns
        .iter()
        .map(|c| {
            let kind = match c.kind.as_str() {
                "Number" => "Number",
                "Date" => "Date",
                "Boolean" => "Boolean",
                _ => "Text",
            };
            json!({ "name": c.field, "label": c.source, "field_type": kind, "required": c.field == key, "ferpa_sensitive": sensitive })
        })
        .collect();
    serde_json::from_value(json!({
        "slug": app, "title": name, "description": format!("A read-only copy of {name}. Only its sync changes it."), "organization_code": "INST", "department": "platform",
        "workspace_id": SYSTEM_WORKSPACE, "views": [], "kind": "Dataset",
        "tables": [{ "id": table, "name": name, "slug": table, "primary_field": key, "fields": fields }],
    }))
    .map_err(|e| ServiceError::Internal(format!("The dataset's manifest could not be built: {e}")))
}

/// A connection, checked and ready to store.
pub struct Spec {
    name: String,
    kind: String,
    config: Value,
    /// The stored secret a source's password or token is read from. A postgres source needs one.
    secret: Option<String>,
    app: String,
    table: String,
    key: String,
    map: Value,
    columns: Vec<connections::Column>,
    sensitivity: String,
    every: Option<i32>,
    at: Option<String>,
}

/// Checks the body of a new connection. Nothing is stored.
pub fn parse_spec(body: &Value) -> Result<Spec, ServiceError> {
    let name = body.get("name").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if name.is_empty() || name.len() > 120 {
        return Err(bad("A connection needs a name of 1 to 120 characters"));
    }
    let kind = body.get("kind").and_then(Value::as_str).unwrap_or("").to_string();
    let config = body.get("config").cloned().unwrap_or(Value::Null);
    check_config(&kind, &config)?;
    let (app, table, key) = (
        body.get("dataset_app").and_then(Value::as_str).unwrap_or("").to_string(),
        body.get("dataset_table").and_then(Value::as_str).unwrap_or("").to_string(),
        body.get("key_field").and_then(Value::as_str).unwrap_or("").to_string(),
    );
    if !slug_ok(&app, 64, true) || !slug_ok(&table, 64, false) {
        return Err(bad("dataset_app is lowercase letters, digits and dashes, and dataset_table is lowercase letters, digits and underscores"));
    }
    let map = body.get("column_map").cloned().unwrap_or(Value::Null);
    let columns = connections::columns_of(&map);
    if columns.is_empty() || columns.len() != map.as_array().map_or(0, Vec::len) {
        return Err(bad("column_map is a list of { source, field, type }"));
    }
    let mut sources = std::collections::HashSet::new();
    let mut fields = std::collections::HashSet::new();
    for c in &columns {
        if !slug_ok(&c.field, 64, false) || c.field.starts_with('_') {
            return Err(bad(format!("The field '{}' is lowercase letters, digits and underscores", c.field)));
        }
        if !COLUMN_TYPES.contains(&c.kind.as_str()) {
            return Err(bad(format!("The type '{}' is not one of {}", c.kind, COLUMN_TYPES.join(", "))));
        }
        if !sources.insert(c.source.clone()) || !fields.insert(c.field.clone()) {
            return Err(bad("Each column and each field may be mapped once"));
        }
    }
    if !fields.contains(&key) {
        return Err(bad("key_field must be one of the mapped fields"));
    }
    let sensitivity = body.get("sensitivity").and_then(Value::as_str).unwrap_or("").to_string();
    if !SENSITIVITIES.contains(&sensitivity.as_str()) {
        return Err(bad(format!("sensitivity is one of {}", SENSITIVITIES.join(", "))));
    }
    let (every, at) = check_schedule(body)?;
    let secret = match body.get("secret") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().filter(|n| crate::secrets::valid_name(n)).ok_or_else(|| bad("secret names a stored secret"))?.to_string()),
    };
    match (kind.as_str(), &secret) {
        ("postgres", None) => return Err(bad("A postgres connection names the secret that holds its password")),
        ("file_drop", Some(_)) => return Err(bad("A file drop reads no secret")),
        _ => {}
    }
    Ok(Spec { name, kind, config, secret, app, table, key, map, columns, sensitivity, every, at })
}

/// Refuses a connection whose name or dataset is taken.
pub fn check_free(spec: &Spec, state: &SharedState) -> Result<(), ServiceError> {
    if crate::service::proposals::live_app_exists(&spec.app, state) {
        return Err(ServiceError::conflict(format!("An app named '{}' already exists", spec.app)));
    }
    if let Some(name) = &spec.secret {
        let known = crate::secrets::list(&state.repository).map_err(storage)?.iter().any(|s| s.name == *name);
        if !known {
            return Err(bad(format!("There is no stored secret named '{name}'. Add it under Secrets first")));
        }
    }
    let name = spec.name.clone();
    let taken: bool = state.repository.with_client(move |c| Ok(c.query_opt("SELECT 1 FROM connections WHERE name = $1", &[&name])?.is_some())).map_err(storage)?;
    if taken {
        return Err(ServiceError::conflict(format!("A connection named '{}' already exists", spec.name)));
    }
    Ok(())
}

/// Stores the connection and the dataset it fills.
pub fn store(spec: Spec, id: Uuid, created_by: &str, state: &SharedState) -> Result<(), ServiceError> {
    let manifest = dataset_manifest(&spec.name, &spec.app, &spec.table, &spec.key, &spec.columns, &spec.sensitivity)?;
    state.persist_app_manifest(manifest).map_err(|e| ServiceError::StorageUnavailable(e.to_string()))?;
    let by = created_by.to_string();
    state
        .repository
        .with_client(move |c| {
            c.execute(
                "INSERT INTO connections (id, name, kind, config, secret, dataset_app, dataset_table, key_field, column_map, sensitivity, every_minutes, at_time, created_by) \
                 VALUES ($1, $2, $3, $4, $13, $5, $6, $7, $8, $9, $10, $11, $12)",
                &[&id, &spec.name, &spec.kind, &spec.config, &spec.app, &spec.table, &spec.key, &spec.map, &spec.sensitivity, &spec.every, &spec.at, &by, &spec.secret],
            )?;
            Ok(())
        })
        .map_err(storage)
}

pub fn create(caller: &AuthUser, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let spec = parse_spec(body)?;
    check_free(&spec, state)?;
    let id = Uuid::new_v4();
    let payload = json!({ "connection": id, "name": spec.name, "kind": spec.kind, "dataset_app": spec.app, "action": "created" });
    admin_write(state, caller, DecisionType::ConnectionChanged, "CM-03", &reason_of(body), &payload, || {
        store(spec, id, &caller.eppn, state)?;
        Ok(json!({ "connection": show(state, id)? }))
    })
}

/// Changes a connection's name, source file, schedule, or whether it runs. Its columns and dataset are fixed here: a
/// source that changes shape is handled by a proposal, not by an edit.
pub fn update(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let id = id_of(id)?;
    let current = show(state, id)?;
    let name = match body.get("name") {
        None => current["name"].as_str().unwrap_or("").to_string(),
        Some(v) => v.as_str().map(str::trim).filter(|n| !n.is_empty() && n.len() <= 120).ok_or_else(|| bad("A connection needs a name of 1 to 120 characters"))?.to_string(),
    };
    let config = match body.get("config") {
        None => current["config"].clone(),
        Some(c) => {
            check_config(current["kind"].as_str().unwrap_or(""), c)?;
            c.clone()
        }
    };
    let enabled = match body.get("enabled") {
        None => current["enabled"].as_bool().unwrap_or(true),
        Some(v) => v.as_bool().ok_or_else(|| bad("enabled is true or false"))?,
    };
    let (every, at) = if body.get("every_minutes").is_some() || body.get("at_time").is_some() {
        check_schedule(body)?
    } else {
        (current["every_minutes"].as_i64().map(|n| n as i32), current["at_time"].as_str().map(str::to_string))
    };
    let payload = json!({ "connection": id, "name": name, "action": "changed", "enabled": enabled });
    admin_write(state, caller, DecisionType::ConnectionChanged, "CM-03", &reason_of(body), &payload, || {
        state
            .repository
            .with_client(move |c| {
                c.execute("UPDATE connections SET name = $2, config = $3, enabled = $4, every_minutes = $5, at_time = $6 WHERE id = $1", &[&id, &name, &config, &enabled, &every, &at])?;
                Ok(())
            })
            .map_err(|e| match e {
                crate::repository::RepositoryError::Db(ref d) if d.code().is_some_and(|c| c.code() == "23505") => ServiceError::conflict("Another connection has that name"),
                e => storage(e),
            })?;
        Ok(json!({ "connection": show(state, id)? }))
    })
}

pub fn runs(caller: &AuthUser, id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let id = id_of(id)?;
    show(state, id)?;
    Ok(json!({ "runs": connections::runs(&state.repository, id).map_err(storage)? }))
}

/// Runs a connection now. With `approve_shrink`, a file much smaller than the last one is applied, and the approval is
/// a ledger entry.
pub fn run_now(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let id = id_of(id)?;
    show(state, id)?;
    let approve = body.get("approve_shrink").and_then(Value::as_bool).unwrap_or(false);
    let repropose = body.get("repropose").and_then(Value::as_bool).unwrap_or(false);
    let go = || {
        connections::run(state, id, connections::RunOpts { approve_shrink: approve, only_if_due: false, repropose }).map_err(|e| match e {
            RunError::NotFound => ServiceError::NotFound("Connection not found".to_string()),
            RunError::Busy => ServiceError::conflict("A run of this connection is already going"),
            RunError::NotDue => ServiceError::conflict("The connection is not due"),
            RunError::Storage(m) => ServiceError::StorageUnavailable(m),
        })
    };
    let run = if approve {
        let payload = json!({ "connection": id, "action": "shrink_approved" });
        admin_write(state, caller, DecisionType::ConnectionChanged, "CM-03", &reason_of(body), &payload, go)?
    } else {
        go()?
    };
    Ok(json!({ "run": run }))
}

/// Makes the system workspace, if it is not there. Nobody is a member: Platform Admins manage it.
pub fn ensure_system_workspace(repo: &crate::repository::PostgresRepository) -> Result<(), crate::repository::RepositoryError> {
    let exists = repo.list_workspaces()?.iter().any(|w| w.id == SYSTEM_WORKSPACE);
    if exists {
        return Ok(());
    }
    repo.upsert_workspace(&crate::state::WorkspaceRecord {
        id: SYSTEM_WORKSPACE.to_string(),
        name: "System datasets".to_string(),
        code: "SYS-DATA".to_string(),
        organization: "Institution".to_string(),
        department: "platform".to_string(),
        description: "Read-only copies of data from other systems. Only their syncs change them.".to_string(),
        icon: "🗄️".to_string(),
        lead: String::new(),
        visibility: "restricted".to_string(),
        data_classification: "Internal".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        organization_id: None,
        coordinates: Default::default(),
        synthetic: false,
    })
}

/// A dataset's records are changed only by its sync. Every service that writes a record calls this first.
pub fn ensure_writable(app_slug: &str, state: &SharedState) -> Result<(), ServiceError> {
    let dataset = state.engine.read().ok().and_then(|e| e.resolve_by_slug(app_slug).map(|m| m.kind == scaffoldry_engine::AppKind::Dataset)).unwrap_or(false);
    if dataset {
        return Err(ServiceError::ReadOnlySource("This dataset is read-only. It is a copy of another system's data, and only its sync changes it".to_string()));
    }
    Ok(())
}

/// The demo datasets, as fixture files. `seed-demo` calls `seed_demo_connections` after the catalog is seeded: it copies
/// each file into the drop directory and connects it, so the demo datasets are real connections. With no drop directory
/// configured, none is made.
const DEMO_FILES: [(&str, &str); 4] = [
    ("faculty", include_str!("../../fixtures/datasets/faculty.csv")),
    ("programs", include_str!("../../fixtures/datasets/programs.csv")),
    ("courses", include_str!("../../fixtures/datasets/courses.csv")),
    ("grants", include_str!("../../fixtures/datasets/grants.csv")),
];

pub fn seed_demo_connections(state: &SharedState) {
    let Some(root) = state.drop_dir.clone() else { return };
    let Ok(published) = state.repository.list_published_datasets() else { return };
    for (id, text) in DEMO_FILES {
        let Some(ds) = published.iter().find(|d| d.id == id) else { continue };
        let rel = format!("demo/{id}.csv");
        let path = root.join(&rel);
        if !path.exists() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if std::fs::write(&path, text).is_err() {
                continue;
            }
        }
        let column_map: Vec<Value> = ds
            .fields
            .iter()
            .map(|f| json!({ "source": f.name, "field": f.name, "type": match f.field_type.as_str() { "Number" => "Number", "Boolean" => "Boolean", "Date" => "Date", _ => "Text" } }))
            .collect();
        let key = ds.fields.iter().find(|f| f.required).map(|f| f.name.clone()).unwrap_or_default();
        let sensitivity = match ds.sensitivity_level.split(['/', ' ']).next().unwrap_or("Internal") {
            "Public" => "Public",
            "Directory" => "Directory",
            _ => "Restricted",
        };
        let body = json!({
            "name": ds.name, "kind": "file_drop", "config": { "path": rel, "format": "csv" }, "dataset_app": format!("dataset-{id}"), "dataset_table": id,
            "key_field": key, "column_map": column_map, "sensitivity": sensitivity, "every_minutes": 1440,
        });
        let Ok(spec) = parse_spec(&body) else { continue };
        if check_free(&spec, state).is_err() {
            continue;
        }
        let conn = Uuid::new_v4();
        let payload = json!({ "connection": conn, "name": spec.name, "kind": spec.kind, "dataset_app": spec.app, "action": "created" });
        let _ = state.append_ledger_entry(crate::state::RecordDecisionInput {
            principal: "system".to_string(),
            organization_code: "DIV-ADMIN-CONSOLE".to_string(),
            app_slug: None,
            decision_type: DecisionType::ConnectionChanged,
            oscal_control_id: "CM-03".to_string(),
            rationale: "A demo dataset, connected to its fixture file".to_string(),
            payload: &payload,
        });
        if store(spec, conn, "system", state).is_ok() {
            let _ = connections::run(state, conn, connections::RunOpts::default());
        }
    }
}
