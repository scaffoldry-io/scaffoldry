//! Snapshots (import-export phase 5).
//!
//! A snapshot is an app's definition and every record. It holds everything the app's data holds, so it is as sensitive as
//! the app: the app's owner or an admin takes, lists and downloads it, and no agent tool returns it. A download of one
//! with sensitive fields is a disclosure. Restore makes a new app from it and never overwrites a live one.
//! A change that loses data needs a snapshot from the last seven days first.
//! NIST SP 800-53 CP-9, CP-10 and AU-2.

use crate::jobs;
use crate::repository_disclosure::NewDisclosure;
use crate::repository_snapshots::SnapshotRow;
use crate::service::proposals::{self, ProposeInput, Reviewed};
use crate::service::{bundle, ServiceError};
use crate::state::{AuthUser, SharedState};
use crate::files;
use scaffoldry_core::ledger::DecisionType;
use serde_json::{json, Value};
use uuid::Uuid;

/// A snapshot at most this many days old satisfies a change that loses data.
pub const RECENT_DAYS: i32 = 7;

fn internal(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

fn bad(m: impl Into<String>) -> ServiceError {
    ServiceError::bad_request(m)
}

fn jobs_error(e: jobs::JobsError) -> ServiceError {
    match e {
        jobs::JobsError::Quota(m) => ServiceError::QuotaExceeded(m),
        other => internal(other),
    }
}

/// The workspace an app is in. An app that has gone belongs to nobody, so only a Platform Admin passes.
fn workspace_of(slug: &str, state: &SharedState) -> Result<String, ServiceError> {
    Ok(state.repository.get_app_manifest(slug).map_err(internal)?.and_then(|m| m.workspace_id).unwrap_or_default())
}

fn require_keeper(caller: &AuthUser, slug: &str, state: &SharedState) -> Result<(), ServiceError> {
    let ws = workspace_of(slug, state)?;
    if matches!(proposals::role_in(caller, &ws, state).as_deref(), Some("owner" | "admin")) {
        Ok(())
    } else {
        Err(ServiceError::forbidden("Forbidden: a snapshot is for the owner or an admin of the app's workspace"))
    }
}

fn load(id: &str, state: &SharedState) -> Result<SnapshotRow, ServiceError> {
    let id = Uuid::parse_str(id).map_err(|_| ServiceError::not_found("Snapshot not found"))?;
    state.repository.get_snapshot(id).map_err(internal)?.ok_or_else(|| ServiceError::not_found("Snapshot not found"))
}

fn view(s: &SnapshotRow) -> Value {
    json!({
        "id": s.id, "snapshot_id": s.id, "app_slug": s.app_slug, "name": s.name, "kind": s.kind, "state": s.state,
        "record_count": s.record_count, "size": s.size, "error": s.error, "created_by": s.created_by, "created_at": s.created_at,
    })
}

/// Records a snapshot and queues the job that writes it. `then_approve` is the approval that waits for it.
pub(crate) fn start(state: &SharedState, slug: &str, kind: &str, name: &str, by: &str, then_approve: Option<Value>) -> Result<(Uuid, Uuid), ServiceError> {
    let row = SnapshotRow {
        id: Uuid::new_v4(),
        app_slug: slug.to_string(),
        name: name.to_string(),
        kind: kind.to_string(),
        state: "pending".to_string(),
        file_sha256: None,
        record_count: None,
        size: None,
        error: None,
        definition: bundle::definition(slug, state)?,
        created_by: by.to_string(),
        created_at: String::new(),
    };
    state.repository.insert_snapshot(&row).map_err(internal)?;
    let mut payload = json!({ "snapshot_id": row.id });
    if let Some(t) = then_approve {
        payload["then_approve"] = t;
    }
    match jobs::enqueue(&state.repository, "snapshot_take", payload, by, None) {
        Ok(job) => Ok((row.id, job)),
        Err(e) => {
            let _ = state.repository.fail_snapshot(row.id, "The job could not be queued");
            Err(jobs_error(e))
        }
    }
}

/// A manual snapshot.
pub fn take(caller: &AuthUser, slug: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_keeper(caller, slug, state)?;
    let name = body.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad("A snapshot needs a 'name'"))?;
    if name.chars().count() > 120 {
        return Err(bad("A snapshot's name is at most 120 characters"));
    }
    let (id, job) = start(state, slug, "manual", name, &caller.eppn, None)?;
    Ok(json!({ "snapshot_id": id, "job_id": job, "kind": "manual", "state": "pending", "app_slug": slug }))
}

pub fn list(caller: &AuthUser, slug: &str, state: &SharedState) -> Result<Value, ServiceError> {
    require_keeper(caller, slug, state)?;
    let rows = state.repository.list_snapshots(slug).map_err(internal)?;
    Ok(json!({ "snapshots": rows.iter().map(view).collect::<Vec<_>>() }))
}

/// The bytes of a ready snapshot, and a name for the file. A snapshot with sensitive fields writes a disclosure.
pub fn download(caller: &AuthUser, id: &str, state: &SharedState) -> Result<(String, Vec<u8>), ServiceError> {
    let s = load(id, state)?;
    require_keeper(caller, &s.app_slug, state)?;
    let sha = s.file_sha256.clone().filter(|_| s.state == "ready").ok_or_else(|| ServiceError::conflict("This snapshot is not ready"))?;
    let dir = files::dir().ok_or_else(|| ServiceError::Internal("File storage is not configured".to_string()))?;
    let bytes = std::fs::read(files::blob_path(&dir, &sha)).map_err(|e| ServiceError::Internal(format!("The snapshot file could not be read: {e}")))?;

    let manifest = state.repository.get_app_manifest(&s.app_slug).map_err(internal)?;
    let sensitive: Vec<String> = manifest
        .as_ref()
        .map(|m| {
            let mut names: Vec<String> = proposals::sensitive_fields(m, state).into_iter().map(|(_, f)| f).collect();
            names.sort();
            names.dedup();
            names
        })
        .unwrap_or_default();
    if !sensitive.is_empty() {
        let repo = &state.repository;
        repo.insert_disclosure(NewDisclosure {
            kind: "snapshot".to_string(),
            actor: caller.eppn.clone(),
            app_slug: s.app_slug.clone(),
            table_id: None,
            record_ids: vec![],
            fields: sensitive.clone(),
            recipient: None,
            note: format!("Downloaded the snapshot \"{}\" of {} records", s.name, s.record_count.unwrap_or(0)),
        })
        .map_err(internal)?;
        let receipt = crate::service::receipts::disclosure_receipt(&caller.eppn, "snapshot", &s.app_slug, None);
        repo.insert_access_receipt(&receipt).map_err(internal)?;
        repo.append_ledger_decision(crate::repository::AppendDecisionParams {
            principal: caller.eppn.clone(),
            organization_code: manifest.as_ref().map(|m| m.organization_code.clone()).filter(|c| !c.is_empty()).unwrap_or_else(|| "SCI".to_string()),
            app_slug: Some(s.app_slug.clone()),
            decision_type: DecisionType::DataExported,
            oscal_control_id: "AU-2".to_string(),
            rationale: "Download of a snapshot".to_string(),
            payload: json!({ "snapshot_id": s.id, "app_slug": s.app_slug, "record_count": s.record_count, "sensitive_fields": sensitive }),
        })
        .map_err(internal)?;
    }
    Ok((format!("{}-{}.jsonl.gz", s.app_slug, &s.id.to_string()[..8]), bytes))
}

/// Restores a snapshot into a new app. The definition goes through a proposal like any other. The records are loaded by a
/// job once the app is live. `{ "slug": ... }` starts it. When the proposal needs a second person, `{ "proposal_id": ... }`
/// finishes it after that person has approved.
pub fn restore(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let s = load(id, state)?;
    require_keeper(caller, &s.app_slug, state)?;
    if s.state != "ready" {
        return Err(ServiceError::conflict("This snapshot is not ready"));
    }
    let ws = workspace_of(&s.app_slug, state)?;
    let queue = |slug: &str| -> Result<Uuid, ServiceError> {
        jobs::enqueue(&state.repository, "snapshot_restore", json!({ "snapshot_id": s.id, "slug": slug, "caller_eppn": caller.eppn }), &caller.eppn, None).map_err(jobs_error)
    };

    if let Some(pid) = body.get("proposal_id").and_then(Value::as_str) {
        let p = state.repository.get_proposal(Uuid::parse_str(pid).map_err(|_| ServiceError::not_found("Proposal not found"))?).map_err(internal)?.ok_or_else(|| ServiceError::not_found("Proposal not found"))?;
        if p.workspace_id != ws || p.proposed_by.to_lowercase() != caller.eppn.to_lowercase() {
            return Err(ServiceError::not_found("Proposal not found"));
        }
        if p.status != "Approved" {
            return Err(ServiceError::conflict(format!("The proposal is {}. Records are loaded once it is approved", p.status)));
        }
        let job = queue(&p.app_slug)?;
        return Ok(json!({ "proposal_id": pid, "approved": true, "job_id": job, "app_slug": p.app_slug }));
    }

    let p = bundle::prepare(&s.definition, &ws, body, None, state)?;
    let summary = bundle::summary_of(body, &p.manifest, &p.slug, "Restore a snapshot as");
    let reviewed = Reviewed { manifest: s.definition["manifest"].clone(), rules: p.rules.clone() };
    let input = ProposeInput { workspace_id: ws.clone(), app_slug: None, manifest: Some(p.manifest.clone()), rules: p.rules.clone(), summary, page_sources: p.page_sources.clone(), notes: p.notes.clone() };
    let proposal = proposals::propose_reviewed(caller, input, reviewed, state)?;
    let approved = proposals::approve_if_direct(caller, &proposal, state)?;
    let mut out = json!({ "proposal_id": proposal["proposal_id"], "app_slug": p.slug, "approved": approved, "checks": proposal["checks"] });
    if approved {
        out["job_id"] = json!(queue(&p.slug)?);
    } else {
        out["next"] = json!("The new app needs a second person to approve it. Once it is approved, send this request again with the proposal_id to load the records.");
    }
    Ok(out)
}

/// The gate in front of an approval that loses data. Returns true when the approval must wait for a snapshot, which this
/// has started. The proposal is `Approving` until the job finishes the approval or gives it back.
pub(crate) fn hold_for_snapshot(
    caller: &AuthUser,
    row: &crate::repository_proposals::ProposalRow,
    rationale: &str,
    digest: &str,
    other_person: bool,
    state: &SharedState,
) -> Result<bool, ServiceError> {
    let loses = row.checks["checks"].as_array().is_some_and(|l| l.iter().any(|c| c["name"] == "data_loss" && c["outcome"] == "fail"));
    if !loses {
        return Ok(false);
    }
    if state.repository.recent_snapshot(&row.app_slug, RECENT_DAYS).map_err(internal)?.is_some() {
        return Ok(false);
    }
    if !state.repository.move_proposal(row.id, "Pending", "Approving").map_err(internal)? {
        return Err(ServiceError::conflict("This proposal is already being approved"));
    }
    let then = json!({ "proposal_id": row.id, "decider": caller.eppn, "rationale": rationale, "digest": digest, "other_person": other_person });
    if let Err(e) = start(state, &row.app_slug, "pre_change", &format!("Before: {}", row.summary.chars().take(100).collect::<String>()), &caller.eppn, Some(then)) {
        let _ = state.repository.move_proposal(row.id, "Approving", "Pending");
        return Err(e);
    }
    Ok(true)
}
