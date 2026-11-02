//! Policy library phase 5: fetching web sources and noticing change.
//!
//! A `url` source is fetched through `net::egress` and the outbound allowlist, at most once a day: 20 seconds, 2 MB,
//! up to 3 redirects that are each checked against the list. A snapshot is stored only when the words change. When one
//! is, every published rule of the policy that cites a passage of this source that the new text no longer holds is
//! marked `source_changed`, with a ledger entry, and goes to its reviewers. Its status does not change.
//!
//! A page that is not `text/html` or `text/plain` stores a snapshot with no text, `needs_text`, and marks nothing.
//! A failure counts one, and three in a row is `unreachable`. A host the list does not allow is `not_allowed`, and
//! nothing is connected to.

use crate::net::egress::{Allow, Client, EgressError, FetchOptions, SystemResolver};
use crate::repository::{AppendDecisionParams, PostgresRepository, RepositoryError};
use crate::state::ServerState;
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_core::library::{normalize_text, passage_in};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Who the ledger names for what this module does on its own.
pub const PRINCIPAL: &str = "scaffoldry-policy-library";
pub const USER_AGENT: &str = "Scaffoldry-policy-library";
pub const TIMEOUT: Duration = Duration::from_secs(20);
pub const MAX_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_REDIRECTS: u8 = 3;
/// Failures in a row that make a source unreachable.
pub const UNREACHABLE_AFTER: i32 = 3;
/// The remedy for a host that is not on the list.
pub const NOT_ALLOWED_REMEDY: &str = "This host is not on the outbound allowlist. Ask a Platform Admin to allow the host.";

/// What one check did.
#[derive(Debug, Clone)]
pub struct Checked {
    /// `ok`, `needs_text`, `failed`, `unreachable` or `not_allowed`.
    pub status: String,
    /// Whether a new snapshot was stored.
    pub stored: bool,
    /// The rules marked `source_changed` by this check.
    pub marked: Vec<Uuid>,
}

fn storage(e: RepositoryError) -> String {
    e.to_string()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn ledger(repo: &PostgresRepository, rationale: &str, payload: Value) -> Result<(), String> {
    repo.append_ledger_decision(AppendDecisionParams {
        principal: PRINCIPAL.to_string(),
        organization_code: "INST".to_string(),
        app_slug: None,
        decision_type: DecisionType::PolicyLibraryChanged,
        oscal_control_id: crate::service::library::OSCAL_CONTROL.to_string(),
        rationale: rationale.to_string(),
        payload,
    })
    .map(|_| ())
    .map_err(storage)
}

/// The client a check uses on this server: its allowlist and the system resolver.
pub fn client_for(repo: &PostgresRepository) -> Result<Client, String> {
    let settings = repo.get_platform_settings().map_err(storage)?;
    Ok(Client::new(Allow::from_settings(&settings), Arc::new(SystemResolver)))
}

fn options(repo: &PostgresRepository) -> FetchOptions {
    let mut headers = vec![("User-Agent".to_string(), USER_AGENT.to_string()), ("Accept".to_string(), "text/html, text/plain".to_string())];
    // The institution's contact address, so a site's operator can say if the checks are a bother.
    if let Some(contact) = repo.get_platform_settings().ok().and_then(|s| s.get("platform.contact_email").and_then(Value::as_str).map(str::to_string)) {
        headers.push(("From".to_string(), contact));
    }
    FetchOptions { total_timeout: TIMEOUT, max_bytes: MAX_BYTES, max_redirects: MAX_REDIRECTS, headers }
}

/// Fetches one `url` source now, stores a snapshot when its words changed, and marks the rules whose passages left it.
pub fn check_source(state: &ServerState, source: Uuid, client: &Client) -> Result<Checked, String> {
    check_source_in(&state.repository, source, client)
}

/// `check_source` for work that holds only the repository, such as a job.
pub fn check_source_in(repo: &PostgresRepository, source: Uuid, client: &Client) -> Result<Checked, String> {
    let row = repo
        .with_client(move |c| Ok(c.query_opt("SELECT s.policy_id, s.kind, s.href, p.key FROM library_sources s JOIN library_policies p ON p.id = s.policy_id WHERE s.id = $1", &[&source])?))
        .map_err(storage)?
        .ok_or("No such source")?;
    let (policy, kind, href, policy_key): (Uuid, String, Option<String>, String) = (row.get(0), row.get(1), row.get(2), row.get(3));
    if kind != "url" {
        return Err("Only a web source is fetched".to_string());
    }
    let href = href.ok_or("The source has no address")?;
    let outcome = client.fetch(&href, &options(repo));
    let (status, stored, marked) = match outcome {
        Err(EgressError::NotAllowed(_)) | Err(EgressError::Blocked(_)) => {
            record(repo, source, "not_allowed", false)?;
            ("not_allowed".to_string(), false, Vec::new())
        }
        Err(_) => (failed(repo, source, policy, &policy_key)?, false, Vec::new()),
        Ok(resp) if !(200..300).contains(&resp.status) => (failed(repo, source, policy, &policy_key)?, false, Vec::new()),
        Ok(resp) => {
            let media = resp.content_type.as_deref().unwrap_or("").split(';').next().unwrap_or("").trim().to_ascii_lowercase();
            let readable = media == "text/html" || media == "text/plain";
            let text = if readable { normalize_text(&media, &resp.body) } else { String::new() };
            let status = if readable { "ok" } else { "needs_text" };
            let text_sha = hex(&Sha256::digest(text.as_bytes()));
            let latest = repo.library_latest_snapshot(source).map_err(storage)?;
            let (stored, marked) = if latest.as_ref().is_some_and(|(_, sha)| *sha == text_sha) {
                (false, Vec::new())
            } else {
                let content_sha = hex(&Sha256::digest(&resp.body));
                let media_type = if media.is_empty() { "application/octet-stream".to_string() } else { media.clone() };
                ledger(
                    repo,
                    "The source's words changed",
                    json!({ "action": "snapshot_taken", "policy_key": policy_key, "source_id": source, "text_sha256": text_sha, "text_origin": "fetched", "status": status }),
                )?;
                repo.library_add_snapshot(source, &media_type, &content_sha, &text, &text_sha, "fetched", PRINCIPAL).map_err(storage)?;
                // A page with no text tells nothing about the passages, so nothing is marked from it.
                let marked = if readable { mark_changed(repo, policy, &policy_key, source, &text)? } else { Vec::new() };
                (true, marked)
            };
            record(repo, source, status, true)?;
            (status.to_string(), stored, marked)
        }
    };
    Ok(Checked { status, stored, marked })
}

/// Writes the outcome of a check: when, what, the next check a day later, and the failure count reset on success.
fn record(repo: &PostgresRepository, source: Uuid, status: &str, reset: bool) -> Result<(), String> {
    let status = status.to_string();
    repo.with_client(move |c| {
        c.execute(
            "UPDATE library_sources SET last_checked_at = NOW(), last_status = $2, next_check_at = NOW() + INTERVAL '1 day', \
             consecutive_failures = CASE WHEN $3 THEN 0 ELSE consecutive_failures END WHERE id = $1",
            &[&source, &status, &reset],
        )?;
        Ok(())
    })
    .map_err(storage)
}

/// Counts a failure. At three in a row the source is `unreachable`, and the ledger tells the policy's owner.
fn failed(repo: &PostgresRepository, source: Uuid, policy: Uuid, policy_key: &str) -> Result<String, String> {
    let failures: i32 = repo
        .with_client(move |c| {
            Ok(c.query_one(
                "UPDATE library_sources SET last_checked_at = NOW(), next_check_at = NOW() + INTERVAL '1 day', consecutive_failures = consecutive_failures + 1, \
                 last_status = CASE WHEN consecutive_failures + 1 >= $2 THEN 'unreachable' ELSE 'failed' END WHERE id = $1 RETURNING consecutive_failures",
                &[&source, &UNREACHABLE_AFTER],
            )?
            .get(0))
        })
        .map_err(storage)?;
    if failures == UNREACHABLE_AFTER {
        ledger(
            repo,
            "A source could not be fetched three times in a row",
            json!({ "action": "source_unreachable", "policy_id": policy, "policy_key": policy_key, "source_id": source, "consecutive_failures": failures }),
        )?;
    }
    Ok(if failures >= UNREACHABLE_AFTER { "unreachable" } else { "failed" }.to_string())
}

/// Marks each published rule of the policy that cites a passage of this source the new text no longer holds. A rule
/// already marked is left as it is. Returns the rules marked now.
fn mark_changed(repo: &PostgresRepository, policy: Uuid, policy_key: &str, source: Uuid, text: &str) -> Result<Vec<Uuid>, String> {
    let rows = repo
        .with_client(move |c| {
            Ok(c.query(
                "SELECT r.id, r.key, c.passage FROM library_rules r JOIN library_citations c ON c.rule_id = r.id JOIN library_snapshots n ON n.id = c.snapshot_id \
                 WHERE r.policy_id = $1 AND r.status = 'Published' AND NOT r.source_changed AND n.source_id = $2 ORDER BY r.key, r.id",
                &[&policy, &source],
            )?)
        })
        .map_err(storage)?;
    let mut gone: Vec<(Uuid, String)> = Vec::new();
    for r in &rows {
        let (id, key, passage): (Uuid, String, String) = (r.get(0), r.get(1), r.get(2));
        if !passage_in(text, &passage) && !gone.iter().any(|(g, _)| *g == id) {
            gone.push((id, key));
        }
    }
    let mut marked = Vec::new();
    for (id, key) in gone {
        ledger(repo, "A cited passage is no longer in its source", json!({ "action": "source_changed", "policy_key": policy_key, "rule_id": id, "rule_key": key, "source_id": source }))?;
        let changed = repo
            .with_client(move |c| Ok(c.execute("UPDATE library_rules SET source_changed = TRUE, source_changed_at = NOW() WHERE id = $1 AND NOT source_changed", &[&id])? > 0))
            .map_err(storage)?;
        if changed {
            marked.push(id);
        }
    }
    Ok(marked)
}

/// Claims the `url` sources whose next check has come, pushes each a day ahead, and checks them. Returns how many.
pub fn check_due(state: &ServerState, client: &Client) -> Result<usize, String> {
    check_due_in(&state.repository, client)
}

pub fn check_due_in(repo: &PostgresRepository, client: &Client) -> Result<usize, String> {
    let due: Vec<Uuid> = repo
        .with_client(|c| {
            Ok(c.query(
                "UPDATE library_sources SET next_check_at = NOW() + INTERVAL '1 day' WHERE id IN ( \
                   SELECT s.id FROM library_sources s JOIN library_policies p ON p.id = s.policy_id \
                   WHERE s.kind = 'url' AND p.status = 'Active' AND (s.next_check_at IS NULL OR s.next_check_at <= NOW()) \
                   ORDER BY s.next_check_at NULLS FIRST LIMIT 100 FOR UPDATE OF s SKIP LOCKED) RETURNING id",
                &[],
            )?
            .iter()
            .map(|r| r.get(0))
            .collect())
        })
        .map_err(storage)?;
    for id in &due {
        // One source that fails does not stop the others.
        if let Err(e) = check_source_in(repo, *id, client) {
            eprintln!("Warning: the policy library could not check source {id}: {e}");
        }
    }
    Ok(due.len())
}

/// The `library_fetch` job, run hourly.
pub fn library_fetch_job(ctx: &crate::jobs::JobCtx, _payload: Value) -> Result<Value, crate::jobs::JobError> {
    let repo = ctx.repository();
    let client = client_for(repo).map_err(crate::jobs::JobError::Retry)?;
    let checked = check_due_in(repo, &client).map_err(crate::jobs::JobError::Retry)?;
    Ok(json!({ "checked": checked }))
}
