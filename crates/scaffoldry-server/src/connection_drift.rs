//! Connections phase 2: when the source changes shape.
//!
//! A column the source added, a column it dropped, or a column that no longer holds what it was mapped to stops the
//! run. Nothing is added or dropped on its own. The run makes one proposal that changes the dataset's definition, and
//! says in words what differs: `The source added "Campus". The source no longer has "Dept code".`
//!
//! The connection remembers the proposal and the column map it would give. Approving the proposal puts that map in
//! place, and the next run goes ahead. Rejecting it leaves the connection stopped, and no run makes the same proposal
//! again unless a person asks. A field the source dropped is retired by the proposal, so its values are kept. A column
//! that no longer holds numbers becomes text, and the proposal carries a preview of the conversion, with counts.

use crate::connections::{columns_of, is_sensitive, typed, Column, Conn, RunOpts, Source};
use crate::service::connections::SYSTEM_WORKSPACE;
use crate::state::{AuthUser, SharedState};
use scaffoldry_engine::recalc::conversion_supported;
use scaffoldry_engine::FieldType;
use serde_json::{json, Value};
use std::collections::HashSet;
use uuid::Uuid;

/// What changed in the source, against the connection's column map.
#[derive(Debug, Clone, Default)]
pub struct Drift {
    /// A column the source added, with the type its values suggest.
    pub added: Vec<(String, String)>,
    /// A mapped column the source no longer has.
    pub removed: Vec<String>,
    /// A mapped column whose values no longer fit its type: `(source, field, type it was)`.
    pub retyped: Vec<(String, String, String)>,
}

impl Drift {
    /// The difference, one sentence for each change.
    pub fn sentences(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (source, _) in &self.added {
            out.push(format!("The source added \"{source}\"."));
        }
        for source in &self.removed {
            out.push(format!("The source no longer has \"{source}\"."));
        }
        for (source, _, was) in &self.retyped {
            out.push(format!("The values of \"{source}\" no longer all fit {}, so it becomes text.", was.to_lowercase()));
        }
        out
    }
}

/// How the source differs from the column map, or `None` when it does not. Only a file with a header has columns to
/// compare, so only a csv can drift.
pub(crate) fn find(conn: &Conn, src: &Source) -> Option<Drift> {
    let headers = src.headers.as_ref()?;
    let present: HashSet<&str> = headers.iter().map(String::as_str).collect();
    let mapped: HashSet<&str> = conn.columns.iter().map(|c| c.source.as_str()).collect();
    let mut drift = Drift::default();
    for h in headers.iter().filter(|h| !h.is_empty() && !mapped.contains(h.as_str())) {
        let values: Vec<&str> = src.rows.iter().filter_map(|r| r.get(h)).map(String::as_str).filter(|v| !v.trim().is_empty()).collect();
        let numeric = !values.is_empty() && values.iter().all(|v| typed("Number", v).is_ok());
        drift.added.push((h.clone(), if numeric { "Number" } else { "Text" }.to_string()));
    }
    for c in &conn.columns {
        if !present.contains(c.source.as_str()) {
            drift.removed.push(c.source.clone());
        } else if c.kind != "Text" && src.rows.iter().filter_map(|r| r.get(&c.source)).any(|v| typed(&c.kind, v).is_err()) {
            drift.retyped.push((c.source.clone(), c.field.clone(), c.kind.clone()));
        }
    }
    (!drift.added.is_empty() || !drift.removed.is_empty() || !drift.retyped.is_empty()).then_some(drift)
}

/// The proposal's status, lowercase, when it exists.
fn status_of(state: &SharedState, id: Uuid) -> Option<String> {
    state.repository.get_proposal(id).ok().flatten().map(|r| r.status.to_lowercase())
}

/// The sync as a person the proposal can name. It holds no right of its own to approve anything: a different person
/// decides, which a change with a flagged check needs anyway.
fn actor(conn: &Conn) -> AuthUser {
    AuthUser {
        eppn: format!("sync:{}", conn.name),
        name: conn.name.clone(),
        role_title: "Data sync".to_string(),
        affiliation: "central_admin".to_string(),
        department: "platform".to_string(),
        view_as_by: None,
        via_client: None,
    }
}

/// A field name for a column the source added: lowercase letters, digits and underscores, not taken.
fn field_for(source: &str, taken: &HashSet<String>) -> String {
    let mut name: String = source.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    while name.contains("__") {
        name = name.replace("__", "_");
    }
    let mut name = name.trim_matches('_').to_string();
    if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
        name = format!("f_{name}");
    }
    name.truncate(56);
    let mut candidate = name.clone();
    let mut n = 2;
    while taken.contains(&candidate) {
        candidate = format!("{name}_{n}");
        n += 1;
    }
    candidate
}

/// The column map the change would give the connection.
fn planned_map(conn: &Conn, drift: &Drift) -> (Vec<Column>, Value) {
    let mut cols: Vec<Column> = conn
        .columns
        .iter()
        .filter(|c| !drift.removed.contains(&c.source))
        .map(|c| if drift.retyped.iter().any(|(s, _, _)| *s == c.source) { Column { kind: "Text".to_string(), ..c.clone() } } else { c.clone() })
        .collect();
    let mut taken: HashSet<String> = conn.columns.iter().map(|c| c.field.clone()).collect();
    for (source, kind) in &drift.added {
        let field = field_for(source, &taken);
        taken.insert(field.clone());
        cols.push(Column { source: source.clone(), field, kind: kind.clone() });
    }
    let map = Value::Array(cols.iter().map(|c| json!({ "source": c.source, "field": c.field, "type": c.kind })).collect());
    (cols, map)
}

/// The dataset's definition with the change in it. A removed field leaves `fields`, and the proposal retires it.
fn planned_manifest(state: &SharedState, conn: &Conn, drift: &Drift, cols: &[Column]) -> Result<Value, String> {
    let live = state.get_app_manifest(&conn.dataset_app).ok_or("The dataset's definition could not be read")?;
    let mut m = serde_json::to_value(&live).map_err(|e| e.to_string())?;
    let table = m["tables"].as_array_mut().and_then(|t| t.iter_mut().find(|t| t["id"] == conn.dataset_table.as_str())).ok_or("The dataset's table is not in its definition")?;
    let gone: Vec<&str> = conn.columns.iter().filter(|c| drift.removed.contains(&c.source)).map(|c| c.field.as_str()).collect();
    let fields = table["fields"].as_array_mut().ok_or("The dataset's table has no fields")?;
    fields.retain(|f| !f["name"].as_str().is_some_and(|n| gone.contains(&n)));
    for (_, field, _) in &drift.retyped {
        if let Some(f) = fields.iter_mut().find(|f| f["name"] == field.as_str()) {
            f["field_type"] = json!("Text");
        }
    }
    for (source, kind) in &drift.added {
        let Some(c) = cols.iter().find(|c| c.source == *source) else { continue };
        fields.push(json!({ "name": c.field, "label": source, "field_type": kind, "required": false, "ferpa_sensitive": is_sensitive(&conn.sensitivity) }));
    }
    Ok(m)
}

fn field_type(kind: &str) -> FieldType {
    match kind {
        "Number" => FieldType::Number,
        "Date" => FieldType::Date,
        "Boolean" => FieldType::Boolean,
        _ => FieldType::Text,
    }
}

/// Asks for a preview of each conversion the proposal needs, once. Returns true when every preview is done.
fn previews_ready(state: &SharedState, conn: &Conn, drift: &Drift) -> Result<bool, String> {
    let mut ready = true;
    for (_, field, was) in &drift.retyped {
        let have = state.repository.latest_convert_preview(&conn.dataset_app, &conn.dataset_table, field, FieldType::Text).map_err(|e| e.to_string())?;
        if have.is_some() {
            continue;
        }
        ready = false;
        let payload = json!({ "app_slug": conn.dataset_app, "table_id": conn.dataset_table, "field": field, "from": field_type(was), "to": FieldType::Text });
        let (app, f) = (conn.dataset_app.clone(), field.clone());
        let waiting: bool = state
            .repository
            .with_client(move |c| {
                Ok(c.query_opt("SELECT 1 FROM jobs WHERE kind = 'convert_preview' AND state IN ('queued', 'running') AND payload->>'app_slug' = $1 AND payload->>'field' = $2", &[&app, &f])?.is_some())
            })
            .map_err(|e| e.to_string())?;
        if !waiting {
            crate::jobs::enqueue(&state.repository, "convert_preview", payload, &format!("sync:{}", conn.name), None).map_err(|e| e.to_string())?;
        }
    }
    Ok(ready)
}

fn remember(state: &SharedState, conn: &Conn, proposal: Option<Uuid>, map: Option<Value>) -> Result<(), String> {
    let id = conn.id;
    state
        .repository
        .with_client(move |c| {
            c.execute("UPDATE connections SET pending_proposal = $2, pending_column_map = $3 WHERE id = $1", &[&id, &proposal, &map])?;
            Ok(())
        })
        .map_err(|e| e.to_string())
}

/// Deals with a source that changed shape. Returns the error the run keeps. It starts with `schema_drift:`.
pub(crate) fn handle(state: &SharedState, conn: &Conn, drift: &Drift, opts: RunOpts) -> String {
    let said = drift.sentences().join(" ");
    match handle_inner(state, conn, drift, opts, &said) {
        Ok(message) | Err(message) => format!("schema_drift: {said} {message}"),
    }
}

fn handle_inner(state: &SharedState, conn: &Conn, drift: &Drift, opts: RunOpts, said: &str) -> Result<String, String> {
    if let Some(key) = conn.columns.iter().find(|c| c.field == conn.key_field && drift.removed.contains(&c.source)) {
        return Err(format!("The key column \"{}\" is gone, so no change is proposed. Change the connection by hand.", key.source));
    }
    for (source, _, was) in &drift.retyped {
        if !conversion_supported(field_type(was), FieldType::Text) {
            return Err(format!("\"{source}\" cannot be converted to text automatically, so no change is proposed. Change the connection by hand."));
        }
    }
    let (cols, map) = planned_map(conn, drift);
    if let Some(pending) = conn.pending_proposal {
        let same = conn.pending_map.as_ref() == Some(&map);
        match (same, status_of(state, pending).as_deref()) {
            (true, Some("rejected")) if !opts.repropose => {
                return Err(format!("Proposal {pending} was rejected, so the connection stays stopped. Put the source back as it was, or run it again with repropose to propose the change again."));
            }
            (true, Some("rejected")) => {}
            (true, _) => return Ok(format!("Proposal {pending} is waiting for approval.")),
            (false, Some("pending")) => {
                // The source changed again. The old proposal is out of date.
                let _ = state.repository.reject_proposal(pending, &actor(conn).eppn);
            }
            (false, _) => {}
        }
    }
    if !previews_ready(state, conn, drift)? {
        return Ok("A preview of the change of type is running. Run it again when the preview is done, and the change will be proposed.".to_string());
    }
    let manifest = planned_manifest(state, conn, drift, &cols)?;
    let summary = format!("Connection \"{}\": {said}", conn.name);
    let input = crate::service::proposals::ProposeInput {
        workspace_id: SYSTEM_WORKSPACE.to_string(),
        app_slug: Some(conn.dataset_app.clone()),
        manifest: Some(manifest),
        rules: Vec::new(),
        summary,
        page_sources: Vec::new(),
        notes: vec![format!("This change comes from the connection \"{}\". Approving it changes the dataset's fields and the connection's column map together.", conn.name)],
    };
    let made = crate::service::proposals::propose(&actor(conn), input, state).map_err(|e| format!("The change could not be proposed: {}", e.message()))?;
    let id = made["proposal_id"].as_str().or_else(|| made["id"].as_str()).and_then(|s| Uuid::parse_str(s).ok()).ok_or("The proposal was made, but its id could not be read")?;
    remember(state, conn, Some(id), Some(map))?;
    Ok(format!("Proposal {id} is waiting for approval."))
}

/// Puts an approved change in place: the connection takes the column map the proposal gave it. Called before each run
/// and when a proposal is decided, so a crash between the two does not leave the connection behind its dataset.
pub(crate) fn settle(state: &SharedState, conn: &mut Conn) -> Result<(), crate::repository::RepositoryError> {
    let (Some(pid), Some(map)) = (conn.pending_proposal, conn.pending_map.clone()) else { return Ok(()) };
    if status_of(state, pid).as_deref() != Some("approved") {
        return Ok(());
    }
    let id = conn.id;
    let new_map = map.clone();
    state.repository.with_client(move |c| {
        c.execute("UPDATE connections SET column_map = $2, pending_proposal = NULL, pending_column_map = NULL WHERE id = $1 AND pending_proposal = $3", &[&id, &new_map, &pid])?;
        Ok(())
    })?;
    conn.columns = columns_of(&map);
    conn.pending_proposal = None;
    conn.pending_map = None;
    Ok(())
}

/// The source went back to what the dataset holds. A change that still waits is withdrawn, and the connection is clear.
pub(crate) fn withdraw(state: &SharedState, conn: &Conn) {
    let Some(pid) = conn.pending_proposal else { return };
    if status_of(state, pid).as_deref() == Some("pending") {
        let _ = state.repository.reject_proposal(pid, &actor(conn).eppn);
    }
    let _ = remember(state, conn, None, None);
}

/// `settle` for the connection with this id.
pub(crate) fn settle_by_id(state: &SharedState, id: Uuid) {
    if let Ok(Some(mut conn)) = crate::connections::load(&state.repository, id) {
        let _ = settle(state, &mut conn);
    }
}

/// Called after a proposal for `app` is decided: an approved one reaches its connection at once.
pub fn after_decision(state: &SharedState, app: &str) {
    let app = app.to_string();
    let id: Option<Uuid> = state.repository.with_client(move |c| Ok(c.query_opt("SELECT id FROM connections WHERE dataset_app = $1 AND pending_proposal IS NOT NULL", &[&app])?.map(|r| r.get(0)))).ok().flatten();
    if let Some(id) = id {
        if let Ok(Some(mut conn)) = crate::connections::load(&state.repository, id) {
            let _ = settle(state, &mut conn);
        }
    }
}
