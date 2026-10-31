//! A trial copy of a proposed app (proposal-preview phase 3).
//!
//! A reviewer sometimes needs to try the form, not only look at it. A trial is the proposed app under a slug of its own,
//! in a synthetic workspace the reviewer owns, filled with synthetic records made from the proposed tables. It never holds
//! a live record, and a write in it changes nothing outside it. A person has a few open at a time. A trial ends when its
//! proposal is decided, or when it has gone unopened for the number of days in the settings.
//!
//! The numbers are the settings `preview.trials_per_person` (3), `preview.trial_records` (500) and `preview.trial_days` (7).
//! NIST SP 800-53 CM-3, CM-4 and SA-11.
//!
//! Destruction is a transaction of its own here. The brief sends it through `purge::destroy`, which `lifecycle.md` phases 3
//! and 4 build. A trial holds only synthetic records, so no legal hold can apply to it. When `purge::destroy` exists, this
//! should call it.

use crate::repository_trials::TrialRow;
use crate::service::synthesize::DeterministicPrng;
use crate::service::ServiceError;
use crate::state::{AuthUser, CollaboratorRecord, DatasetRecord, OrganizationNode, RecordDecisionInput, SharedState, WorkspaceRecord};
use scaffoldry_core::DecisionType;
use scaffoldry_engine::{FieldSpec, FieldType};
use serde_json::{json, Value};
use uuid::Uuid;

pub const TRIAL_MARK: &str = "~trial~";
const SYNTHETIC_ACTOR: &str = "synthetic@synthetic.invalid";
/// Records made for one table. A trial with many tables gets fewer each, so the whole stays under the limit.
const PER_TABLE: usize = 50;
/// The synthetic unit every trial workspace sits in, under the institution's academic root.
const TRIALS_UNIT: Uuid = Uuid::from_u128(0x7472_6961_6c73_0000_0000_0000_0000_0001);
const ROOT_UNIT: Uuid = Uuid::from_u128(1);

fn internal(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

fn setting(state: &SharedState, key: &str, default: i64) -> i64 {
    state.settings.read().ok().and_then(|s| s.get(key).and_then(Value::as_i64)).filter(|n| *n > 0).unwrap_or(default)
}

/// Open trials per person, records per trial, and days unopened before a trial ends.
pub fn limits(state: &SharedState) -> (i64, i64, i64) {
    (setting(state, "preview.trials_per_person", 3), setting(state, "preview.trial_records", 500), setting(state, "preview.trial_days", 7))
}

/// A value for a field, or none when the field is computed, points at another record, or is set by the system.
fn value_for(field: &FieldSpec, n: usize, prng: &mut DeterministicPrng) -> Option<Value> {
    let r = prng.next_u64();
    let month = r % 12 + 1;
    let day = (r / 12) % 28 + 1;
    Some(match &field.field_type {
        FieldType::Text => json!(format!("{} {}", field.label, n + 1)),
        FieldType::LongText => json!(format!("Sample text for {} {}. Nothing here is real.", field.label, n + 1)),
        FieldType::Number => json!((r % 1000) as i64),
        FieldType::Currency => json!(((r % 100_000) as f64) / 100.0),
        FieldType::Percent => json!((r % 101) as i64),
        FieldType::Rating => json!((r % 5 + 1) as i64),
        FieldType::Date => json!(format!("2026-{month:02}-{day:02}")),
        FieldType::DateTime => json!(format!("2026-{month:02}-{day:02}T{:02}:00:00Z", r % 24)),
        FieldType::Boolean | FieldType::Checkbox => json!(r.is_multiple_of(2)),
        FieldType::Select => json!(field.select_options.get((r as usize) % field.select_options.len().max(1))?.id),
        FieldType::MultiSelect => json!([field.select_options.get((r as usize) % field.select_options.len().max(1))?.id]),
        FieldType::Email => json!(format!("person{}@synthetic.invalid", n + 1)),
        FieldType::Phone => json!(format!("555-01{:02}", n % 100)),
        FieldType::Url => json!(format!("https://example.invalid/{}", n + 1)),
        FieldType::Duration { .. } => json!((r % 7200) as i64),
        _ => return None,
    })
}

/// A trial's tables, filled. Returns how many records were written.
fn fill(slug: &str, manifest: &scaffoldry_engine::AppManifest, cap: usize, state: &SharedState) -> Result<usize, ServiceError> {
    let tables = manifest.tables.len().max(1);
    let per_table = PER_TABLE.min(cap / tables).max(usize::from(cap > 0));
    let mut prng = DeterministicPrng::new(u64::from_le_bytes(Uuid::new_v4().as_bytes()[..8].try_into().unwrap_or([7; 8])));
    let mut written = 0usize;
    for table in &manifest.tables {
        let probe = json!({ "_table_id": table.id });
        let ctx = crate::service::history::ctx_for(SYNTHETIC_ACTOR, crate::repository::ActorKind::System, slug, &probe, state);
        let rows: Vec<DatasetRecord> = (0..per_table.min(cap.saturating_sub(written)))
            .map(|n| {
                let mut data = serde_json::Map::new();
                data.insert("_table_id".to_string(), json!(table.id));
                for field in &table.fields {
                    if let Some(v) = value_for(field, n, &mut prng) {
                        data.insert(field.name.clone(), v);
                    }
                }
                DatasetRecord {
                    id: Uuid::new_v4().to_string(),
                    app_slug: slug.to_string(),
                    data: Value::Object(data),
                    ceds_mapping: json!({}),
                    is_ferpa_sensitive: false,
                    created_at: String::new(),
                    created_by: Some(SYNTHETIC_ACTOR.to_string()),
                    version: 1,
                }
            })
            .collect();
        written += rows.len();
        state
            .repository
            .with_client(move |c| {
                let mut tx = c.transaction()?;
                for rec in &rows {
                    crate::repository::insert_record_in_tx(&mut tx, rec, &ctx)?;
                }
                tx.commit()?;
                Ok(())
            })
            .map_err(internal)?;
    }
    Ok(written)
}

/// The synthetic unit trial workspaces sit in. It is made once.
fn ensure_unit(state: &SharedState) -> Result<(), ServiceError> {
    let known = state.organizations.read().unwrap_or_else(|p| p.into_inner()).contains_key(&TRIALS_UNIT);
    if known {
        return Ok(());
    }
    let unit = OrganizationNode {
        id: TRIALS_UNIT,
        parent_id: Some(ROOT_UNIT),
        name: "Trial copies".to_string(),
        code: "SYN-TRIALS-ROOT".to_string(),
        org_type: "College".to_string(),
        hierarchy_key: "academic".to_string(),
        synthetic: true,
    };
    state.repository.upsert_organization(&unit).map_err(internal)
}

/// Builds the trial's unit, workspace, owner, app and records. The caller removes what was built if this fails.
fn build(caller: &AuthUser, row: &crate::repository_proposals::ProposalRow, trial: &TrialRow, cap: usize, state: &SharedState) -> Result<usize, ServiceError> {
    ensure_unit(state)?;
    let mut manifest = crate::service::preview::proposed_manifest(row, state)?;
    manifest.slug = trial.app_slug.clone();
    manifest.title = format!("Trial: {}", manifest.title);
    manifest.workspace_id = Some(trial.workspace_id.clone());
    manifest.custom_domain = None;
    manifest.custom_domain_verified = false;

    let workspace: WorkspaceRecord = serde_json::from_value(json!({
        "id": trial.workspace_id, "name": format!("Trial of {}", row.app_slug), "code": trial.workspace_id.to_uppercase(),
        "organization": "Trial copies", "department": manifest.department, "description": "A trial copy of a proposed app. It holds synthetic records only.",
        "lead": caller.eppn, "visibility": "restricted", "data_classification": "Internal", "organization_id": TRIALS_UNIT, "synthetic": true,
        "created_at": chrono::Utc::now().to_rfc3339()
    }))
    .map_err(internal)?;
    state.repository.upsert_workspace(&workspace).map_err(internal)?;
    state
        .repository
        .upsert_collaborator(&CollaboratorRecord {
            id: Uuid::new_v4().to_string(),
            workspace_id: trial.workspace_id.clone(),
            eppn: caller.eppn.clone(),
            name: caller.name.clone(),
            role: "owner".to_string(),
            scoped_affiliation: caller.affiliation.clone(),
            department: caller.department.clone(),
            added_at: chrono::Utc::now().to_rfc3339(),
            app_slug: String::new(),
        })
        .map_err(internal)?;
    state.persist_app_manifest(manifest.clone()).map_err(internal)?;
    let written = fill(&trial.app_slug, &manifest, cap, state)?;
    state.refresh().map_err(internal)?;
    Ok(written)
}

fn view(t: &TrialRow, days: i64) -> Value {
    let opened = chrono::DateTime::parse_from_rfc3339(&t.last_opened_at).map(|d| d.to_utc()).unwrap_or_else(|_| chrono::Utc::now());
    json!({
        "trial_id": t.id, "proposal_id": t.proposal_id, "app_slug": t.app_slug, "workspace_id": t.workspace_id, "records": t.records,
        "created_at": t.created_at, "last_opened_at": t.last_opened_at, "expires_at": (opened + chrono::Duration::days(days)).to_rfc3339(),
    })
}

/// `POST /proposals/{id}/trial`.
pub fn open(caller: &AuthUser, proposal_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    if caller.view_as_by.is_some() {
        return Err(ServiceError::ViewAsReadOnly("You are viewing as someone else. Exit first.".to_string()));
    }
    let id = Uuid::parse_str(proposal_id).map_err(|_| ServiceError::bad_request("Argument 'proposal_id' must be a proposal id"))?;
    let row = state.repository.get_proposal(id).map_err(internal)?.ok_or_else(|| ServiceError::not_found("Proposal not found"))?;
    crate::service::preview::authorize(caller, &row, state)?;
    if row.status != "Pending" {
        return Err(ServiceError::Conflict(format!("This proposal is already {}", row.status)));
    }
    let (per_person, cap, days) = limits(state);
    let trial_id = Uuid::new_v4();
    let short = trial_id.simple().to_string();
    let live: String = row.app_slug.chars().take(40).collect();
    let mut trial = TrialRow {
        id: trial_id,
        proposal_id: id,
        owner: caller.eppn.clone(),
        app_slug: format!("{live}{TRIAL_MARK}{}", &short[..8]),
        workspace_id: format!("trial-{}", &short[..16]),
        records: 0,
        created_at: String::new(),
        last_opened_at: String::new(),
    };
    if !state.repository.reserve_trial(&trial, per_person).map_err(internal)? {
        return Err(ServiceError::QuotaExceeded(format!("You already have {per_person} trial copies open. Let one end, or open one of them, before you make another.")));
    }
    let payload = json!({ "proposal_id": id, "trial_id": trial_id, "app_slug": trial.app_slug, "owner": caller.eppn });
    let ledger = state.append_ledger_entry(RecordDecisionInput {
        principal: caller.eppn.clone(),
        organization_code: String::new(),
        app_slug: Some(row.app_slug.clone()),
        decision_type: DecisionType::ProposalTrialOpened,
        oscal_control_id: "CM-03".to_string(),
        rationale: format!("{} opened a trial copy of proposal {} for {}, with synthetic records", caller.name, id, row.app_slug),
        payload: &payload,
    });
    if let Err(e) = ledger {
        let _ = state.repository.destroy_trial(&trial);
        return Err(ServiceError::internal(format!("Could not record the ledger entry: {e}")));
    }
    match build(caller, &row, &trial, cap as usize, state) {
        Ok(written) => {
            trial.records = written as i32;
            state.repository.set_trial_records(trial_id, trial.records).map_err(internal)?;
        }
        Err(e) => {
            let _ = state.repository.destroy_trial(&trial);
            let _ = state.refresh();
            return Err(e);
        }
    }
    let stored = state.repository.open_trials_of(&caller.eppn).map_err(internal)?.into_iter().find(|t| t.id == trial_id).unwrap_or(trial);
    Ok(view(&stored, days))
}

/// `POST /me/trials/{id}/open`: the owner opens a trial again, and its days start over.
pub fn reopen(caller: &AuthUser, trial_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let id = Uuid::parse_str(trial_id).map_err(|_| ServiceError::not_found("Trial not found"))?;
    if !state.repository.touch_trial(id, &caller.eppn).map_err(internal)? {
        return Err(ServiceError::not_found("Trial not found"));
    }
    let (_, _, days) = limits(state);
    let trial = state.repository.open_trials_of(&caller.eppn).map_err(internal)?.into_iter().find(|t| t.id == id).ok_or_else(|| ServiceError::not_found("Trial not found"))?;
    Ok(view(&trial, days))
}

/// `GET /me/trials`: the caller's open trials and the limit.
pub fn mine(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    let (per_person, _, days) = limits(state);
    let trials: Vec<Value> = state.repository.open_trials_of(&caller.eppn).map_err(internal)?.iter().map(|t| view(t, days)).collect();
    Ok(json!({ "limit": per_person, "trials": trials }))
}

fn destroy_one(repo: &crate::repository::PostgresRepository, t: &TrialRow, why: &str) -> Result<(), String> {
    repo.append_ledger_decision(crate::repository::AppendDecisionParams {
        principal: "system@scaffoldry.internal".to_string(),
        organization_code: String::new(),
        app_slug: None,
        decision_type: DecisionType::ProposalTrialDestroyed,
        oscal_control_id: "SI-12".to_string(),
        rationale: format!("A trial copy of proposal {} was destroyed: {why}", t.proposal_id),
        payload: json!({ "proposal_id": t.proposal_id, "trial_id": t.id, "app_slug": t.app_slug, "records": t.records, "why": why }),
    })
    .map_err(|e| format!("Could not record the ledger entry: {e}"))?;
    repo.destroy_trial(t).map_err(|e| e.to_string())
}

/// Destroys every open trial of a proposal that has just been decided. A failure here never undoes the decision: the
/// scheduled job takes the trial when it goes stale.
pub fn destroy_for_proposal(state: &SharedState, proposal_id: Uuid) {
    let Ok(trials) = state.repository.trials_of_proposal(proposal_id) else { return };
    if trials.is_empty() {
        return;
    }
    for t in &trials {
        if let Err(e) = destroy_one(&state.repository, t, "its proposal was decided") {
            eprintln!("trial {} was not destroyed: {e}", t.id);
        }
    }
    let _ = state.refresh();
}

/// Destroys the trials unopened for `days` days. Returns how many. The job runs this with only a repository: the other
/// servers read the change from the counters the destruction bumps.
pub fn expire_with(repo: &crate::repository::PostgresRepository, days: i64) -> Result<usize, String> {
    let stale = repo.stale_trials(days).map_err(|e| e.to_string())?;
    let mut gone = 0;
    for t in &stale {
        destroy_one(repo, t, &format!("it was not opened for {days} days"))?;
        gone += 1;
    }
    Ok(gone)
}

/// `expire_with`, for the number of days in this server's settings, and this server reloads afterward.
pub fn expire(state: &SharedState) -> Result<usize, ServiceError> {
    let (_, _, days) = limits(state);
    let gone = expire_with(&state.repository, days).map_err(ServiceError::Internal)?;
    if gone > 0 {
        state.refresh().map_err(internal)?;
    }
    Ok(gone)
}
