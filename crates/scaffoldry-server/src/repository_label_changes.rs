//! Data layer phase 6: the reads and the decisions of label changes.
//!
//! The proposals, and every statement on the rows they name, are in `scaffoldry_data::changes`. This file runs them in a
//! transaction with the history rows, the stored flags and the ledger entry, so a decision either does all of it or none.

use crate::repository::{append_ledger_in_tx, reflag_table, write_history, AppendDecisionParams, PostgresRepository, RepositoryError, Touched, WriteCtx};
use scaffoldry_data::changes::{self, Approval, Change, NewChange};
use serde_json::{json, Value};
use uuid::Uuid;

/// What a decision does to the data, once it is complete.
#[derive(Debug, Clone)]
pub enum Plan {
    /// Sets the dataset's floor.
    Floor { dataset_id: String, app_slug: String, table_id: String, floor: Vec<String>, names: Vec<String>, keep: Vec<String> },
    /// Adds and removes tags on named rows.
    Tags { app_slug: String, table_id: String, ids: Vec<String>, add: Vec<String>, remove: Vec<String>, names: Vec<String>, keep: Vec<String> },
}

/// A proposal to store, with what it needs to be stored.
#[derive(Debug, Clone)]
pub struct Proposal {
    pub dataset_id: String,
    pub kind: String,
    pub spec: Value,
    pub reason: String,
    /// `person` or `review`.
    pub source: String,
    pub finding: Option<Uuid>,
    pub by: String,
}

impl Proposal {
    fn as_new(&self) -> NewChange<'_> {
        NewChange { dataset_id: &self.dataset_id, kind: &self.kind, spec: &self.spec, reason: &self.reason, source: &self.source, finding_id: self.finding, proposed_by: &self.by }
    }
}

/// A decision to record.
pub struct DecideReq {
    pub id: Uuid,
    pub by: String,
    pub note: String,
    pub open_work: Option<String>,
    /// The approvals the change needs: one to raise protection, two to remove it.
    pub needs: usize,
    /// The steward chose "apply for new work only" while work is open. The change is approved and held.
    pub hold: bool,
    /// Applies a change that was held. No approval is added.
    pub release: bool,
    pub plan: Plan,
    /// The ledger entry, built from what the decision did.
    pub ledger: Box<dyn FnOnce(&Done) -> AppendDecisionParams + Send>,
}

/// What a decision did, for the ledger entry and the answer.
#[derive(Debug, Clone)]
pub struct Done {
    pub proposed_by: String,
    pub deciders: Vec<String>,
    pub finding: Option<Uuid>,
    pub open_work: Option<String>,
    /// The floor the change replaced, for a floor change.
    pub floor_was: Option<Vec<String>>,
    /// The rows whose tags changed, for a tags change.
    pub tagged: Vec<String>,
}

pub enum DecideOutcome {
    Missing,
    /// The status it has, when it is not the one a decision needs.
    NotPending(String),
    Proposer,
    Duplicate,
    Waiting { approvals: usize },
    Held { approvals: usize },
    Applied { done: Box<Done> },
}

/// One person with a role: where, which position, when, and the scoped affiliation.
#[derive(Debug, Clone)]
pub struct Holder {
    pub eppn: String,
    pub unit: Uuid,
    pub position_key: Option<String>,
    pub effective: Option<i64>,
    pub end: Option<i64>,
    pub affiliation: String,
}

/// An open task on a row.
#[derive(Debug, Clone)]
pub struct OpenTask {
    pub instance: String,
    pub step: String,
    pub person: String,
    pub record: String,
}

/// A process instance still waiting.
#[derive(Debug, Clone)]
pub struct OpenProcess {
    pub id: String,
    pub rule_id: String,
    pub record: String,
    pub started_by: Option<String>,
}

/// What reads a dataset's app besides the app itself.
#[derive(Debug, Clone, Default)]
pub struct Dependents {
    pub automations: Vec<(String, String)>,
    pub shares: Vec<(String, String, String)>,
    pub exports: Vec<String>,
}

fn dates(h: &Holder, now: i64) -> bool {
    h.effective.is_none_or(|e| e <= now) && h.end.is_none_or(|e| e >= now)
}

impl Holder {
    /// True when the holding is in force today.
    pub fn in_force(&self) -> bool {
        dates(self, chrono::Utc::now().timestamp())
    }
}

impl PostgresRepository {
    /// Stores a proposal. It applies nothing.
    pub fn propose_label_change(&self, p: Proposal) -> Result<Uuid, RepositoryError> {
        self.with_client(move |c| Ok(changes::insert(c, &p.as_new())?))
    }

    pub fn label_change(&self, id: Uuid) -> Result<Option<Change>, RepositoryError> {
        self.with_client(move |c| Ok(changes::get(c, id)?))
    }

    pub fn list_label_changes(&self, status: Option<String>, dataset: Option<String>, limit: i64) -> Result<Vec<Change>, RepositoryError> {
        self.with_client(move |c| Ok(changes::list(c, status.as_deref(), dataset.as_deref(), limit)?))
    }

    /// The units and tags of the live rows of a table, grouped. `ids` limits it to named rows.
    pub fn label_groups(&self, app: &str, table: &str, ids: Option<Vec<String>>) -> Result<Vec<changes::Group>, RepositoryError> {
        let (app, table) = (app.to_string(), table.to_string());
        self.with_client(move |c| Ok(changes::groups(c, &app, &table, ids.as_deref())?))
    }

    pub fn label_rows(&self, app: &str, table: &str, ids: Vec<String>) -> Result<Vec<changes::RowLabel>, RepositoryError> {
        let (app, table) = (app.to_string(), table.to_string());
        self.with_client(move |c| Ok(changes::rows_of(c, &app, &table, &ids)?))
    }

    pub fn label_live_count(&self, app: &str, table: &str) -> Result<i64, RepositoryError> {
        let (app, table) = (app.to_string(), table.to_string());
        self.with_client(move |c| Ok(changes::live_count(c, &app, &table)?))
    }

    pub fn label_examples(&self, app: &str, table: &str, n: i64) -> Result<Vec<String>, RepositoryError> {
        let (app, table) = (app.to_string(), table.to_string());
        self.with_client(move |c| Ok(changes::examples(c, &app, &table, n)?))
    }

    /// Every holding of every person, one pass. The evaluation groups people by what they hold.
    pub fn all_holders(&self) -> Result<Vec<Holder>, RepositoryError> {
        self.with_client(|c| {
            let rows = c.query(
                "SELECT lower(p.eppn), r.organization_id, r.position_key, FLOOR(EXTRACT(EPOCH FROM r.effective_date::timestamptz))::bigint, \
                        FLOOR(EXTRACT(EPOCH FROM (r.end_date + 1)::timestamptz))::bigint - 1, r.scoped_affiliation \
                 FROM roles r JOIN persons p ON r.person_id = p.id",
                &[],
            )?;
            Ok(rows.iter().map(|r| Holder { eppn: r.get(0), unit: r.get(1), position_key: r.get(2), effective: r.get(3), end: r.get(4), affiliation: r.get(5) }).collect())
        })
    }

    /// The open task grants on a table.
    pub fn open_tasks(&self, app: &str, table: &str) -> Result<Vec<OpenTask>, RepositoryError> {
        let (app, table) = (app.to_string(), table.to_string());
        self.with_client(move |c| {
            Ok(c.query("SELECT instance_id, step_id, person, record_id FROM task_grants WHERE app_slug = $1 AND table_id = $2 AND state = 'open' ORDER BY instance_id, step_id, person", &[&app, &table])?
                .iter()
                .map(|r| OpenTask { instance: r.get(0), step: r.get(1), person: r.get(2), record: r.get(3) })
                .collect())
        })
    }

    /// The process instances of an app that are still waiting.
    pub fn open_processes(&self, app: &str) -> Result<Vec<OpenProcess>, RepositoryError> {
        let app = app.to_string();
        self.with_client(move |c| {
            Ok(c.query(
                "SELECT id, rule_id, instance_json->>'record_id', instance_json->>'started_by' FROM process_instances WHERE app_slug = $1 AND status = 'Waiting' ORDER BY id",
                &[&app],
            )?
            .iter()
            .filter_map(|r| Some(OpenProcess { id: r.get(0), rule_id: r.get(1), record: r.get::<_, Option<String>>(2)?, started_by: r.get(3) }))
            .collect())
        })
    }

    /// The automations, the live share links and the running exports of an app.
    pub fn label_dependents(&self, app: &str) -> Result<Dependents, RepositoryError> {
        let app = app.to_string();
        self.with_client(move |c| {
            let automations = c.query("SELECT id, name FROM workflow_automations WHERE app_slug = $1 AND enabled ORDER BY id", &[&app])?.iter().map(|r| (r.get(0), r.get(1))).collect();
            let shares = c
                .query("SELECT id::text, view_id::text, scope FROM view_shares WHERE app_slug = $1 AND revoked_at IS NULL AND expires_at > NOW() ORDER BY created_at, id", &[&app])?
                .iter()
                .map(|r| (r.get(0), r.get(1), r.get(2)))
                .collect();
            let exports = c.query("SELECT id::text FROM jobs WHERE kind = 'export_run' AND state IN ('queued', 'running') AND payload->>'app_slug' = $1 ORDER BY created_at, id", &[&app])?.iter().map(|r| r.get(0)).collect();
            Ok(Dependents { automations, shares, exports })
        })
    }

    /// Closes a proposal as declined.
    pub fn decline_label_change(&self, id: Uuid, by: &str, note: &str) -> Result<Option<String>, RepositoryError> {
        let (by, note) = (by.to_string(), note.to_string());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(change) = changes::get_for_update(&mut tx, id)? else { return Ok(Some("Missing".to_string())) };
            if change.status != "Pending" && change.status != "Approved" {
                return Ok(Some(change.status));
            }
            changes::set_status(&mut tx, id, "Declined", &by, &note, None)?;
            tx.commit()?;
            Ok(None)
        })
    }

    /// Changes what a pending proposal asks, and takes back every approval. `Err(status)` when it is not pending.
    pub fn amend_label_change(&self, id: Uuid, spec: Value, by: &str, note: &str) -> Result<Result<(), String>, RepositoryError> {
        let (by, note) = (by.to_string(), note.to_string());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(change) = changes::get_for_update(&mut tx, id)? else { return Ok(Err("Missing".to_string())) };
            if change.status != "Pending" {
                return Ok(Err(change.status));
            }
            changes::amend(&mut tx, id, &spec, &by, &note)?;
            tx.commit()?;
            Ok(Ok(()))
        })
    }

    /// Confirms a review finding and stores the proposal it makes, in one transaction. `Ok(None)` when the finding is not open.
    pub fn confirm_review_finding(&self, finding: Uuid, by: &str, reason: &str, proposal: Proposal, ledger: impl FnOnce(Uuid) -> AppendDecisionParams + Send + 'static) -> Result<Option<Uuid>, RepositoryError> {
        let (by, reason) = (by.to_string(), reason.to_string());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(status) = tx.query_opt("SELECT status FROM content_findings WHERE id = $1 FOR UPDATE", &[&finding])?.map(|r| r.get::<_, String>(0)) else { return Ok(None) };
            if status != "open" {
                return Ok(None);
            }
            tx.execute("UPDATE content_findings SET status = 'confirmed', decided_by = $2, decided_at = NOW(), reason = $3 WHERE id = $1", &[&finding, &by, &reason])?;
            let id = changes::insert(&mut tx, &proposal.as_new())?;
            append_ledger_in_tx(&mut tx, ledger(id))?;
            tx.commit()?;
            Ok(Some(id))
        })
    }

    /// Records an approval, and applies the change when it is complete (or releases one that was held). The row is locked, so two people
    /// deciding at once decide it once. The tags or the floor, the history rows, the stored flags, the ledger entry and the status commit together.
    pub fn decide_label_change(&self, req: DecideReq) -> Result<DecideOutcome, RepositoryError> {
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(change) = changes::get_for_update(&mut tx, req.id)? else { return Ok(DecideOutcome::Missing) };
            let wanted = if req.release { "Approved" } else { "Pending" };
            if change.status != wanted {
                return Ok(DecideOutcome::NotPending(change.status));
            }
            let mut approvals = change.approvals.clone();
            if !req.release {
                if req.needs >= 2 && change.proposed_by.eq_ignore_ascii_case(&req.by) {
                    return Ok(DecideOutcome::Proposer);
                }
                if approvals.iter().any(|a| a.by.eq_ignore_ascii_case(&req.by)) {
                    return Ok(DecideOutcome::Duplicate);
                }
                let at: String = tx.query_one("SELECT NOW()::text", &[])?.get(0);
                approvals.push(Approval { by: req.by.clone(), at, note: req.note.clone() });
                changes::set_approvals(&mut tx, req.id, &approvals)?;
                if approvals.len() < req.needs {
                    tx.commit()?;
                    return Ok(DecideOutcome::Waiting { approvals: approvals.len() });
                }
                if req.hold {
                    changes::set_status(&mut tx, req.id, "Approved", &req.by, &req.note, req.open_work.as_deref())?;
                    tx.commit()?;
                    return Ok(DecideOutcome::Held { approvals: approvals.len() });
                }
            }
            let mut done = Done {
                proposed_by: change.proposed_by.clone(),
                deciders: approvals.iter().map(|a| a.by.clone()).collect(),
                finding: change.finding_id,
                open_work: req.open_work.clone().or(change.open_work.clone()),
                floor_was: None,
                tagged: Vec::new(),
            };
            match &req.plan {
                Plan::Floor { dataset_id, app_slug, table_id, floor, names, keep } => {
                    done.floor_was = scaffoldry_data::labels::set_floor(&mut tx, dataset_id, floor)?;
                    reflag_table(&mut tx, app_slug, table_id, names, keep)?;
                }
                Plan::Tags { app_slug, table_id, ids, add, remove, names, keep } => {
                    let tagged = changes::set_tags(&mut tx, app_slug, table_id, ids, add, remove)?;
                    let mut ctx = WriteCtx::new(req.by.clone(), crate::repository::ActorKind::User);
                    ctx.token_label = Some(format!("label change {}", req.id));
                    for t in &tagged {
                        let mut diff = serde_json::Map::new();
                        diff.insert("_tags".to_string(), json!({ "old": t.before, "new": t.after }));
                        write_history(&mut tx, Touched { app_slug, table_id, record_id: &t.id, version: t.version, action: "update" }, &diff, &ctx)?;
                    }
                    reflag_table(&mut tx, app_slug, table_id, names, keep)?;
                    done.tagged = tagged.into_iter().map(|t| t.id).collect();
                }
            }
            append_ledger_in_tx(&mut tx, (req.ledger)(&done))?;
            changes::set_status(&mut tx, req.id, "Applied", &req.by, &req.note, done.open_work.as_deref())?;
            tx.commit()?;
            Ok(DecideOutcome::Applied { done: Box::new(done) })
        })
    }
}
