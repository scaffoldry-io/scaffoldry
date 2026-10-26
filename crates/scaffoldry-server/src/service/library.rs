//! Policy library phase 1: policies, sources, snapshots and cited rules. A route and a tool call the same functions.
//!
//! Text from a source is data. It is stored and returned, and never placed in an error, a description or a log line.

use crate::repository_library::{NewCitation, NewPolicy};
use crate::service::admin::{admin_write, require_platform_admin_or_compliance};
use crate::service::approvers::{check_spec, resolves_to};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::approver::ApproverSpec;
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_core::library::{normalize_text, passage_in};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use uuid::Uuid;

const MAX_SNAPSHOT_BYTES: usize = 2 * 1024 * 1024;
const MAX_SOURCES: i64 = 20;
const MAX_RULE_KEYS: i64 = 500;
pub(crate) const OSCAL_CONTROL: &str = "PL-01";

pub(crate) fn internal(e: impl ToString) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

/// The fields of a body that did not pass, each with a sentence.
#[derive(Default)]
pub(crate) struct Problems(pub(crate) BTreeMap<String, String>);

impl Problems {
    pub(crate) fn add(&mut self, field: &str, why: impl Into<String>) {
        self.0
            .entry(field.to_string())
            .or_insert_with(|| why.into());
    }

    pub(crate) fn text(&mut self, body: &Value, field: &str, max: usize, required: bool) -> Option<String> {
        match body.get(field) {
            None | Some(Value::Null) if !required => None,
            None | Some(Value::Null) => {
                self.add(field, format!("{field} is required"));
                None
            }
            Some(Value::String(s)) => {
                let s = s.trim();
                if required && s.is_empty() {
                    self.add(field, format!("{field} may not be empty"));
                    None
                } else if s.chars().count() > max {
                    self.add(field, format!("{field} may be at most {max} characters"));
                    None
                } else {
                    Some(s.to_string())
                }
            }
            Some(_) => {
                self.add(field, format!("{field} must be text"));
                None
            }
        }
    }

    fn key(&mut self, body: &Value, field: &str) -> Option<String> {
        let key = self.text(body, field, 64, true)?;
        let ok = key.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && key
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
        if !ok {
            self.add(
                field,
                format!(
                    "{field} is lowercase letters, digits, '-' and '_', starting with a letter"
                ),
            );
            return None;
        }
        Some(key)
    }

    fn uuid(&mut self, body: &Value, field: &str) -> Option<Uuid> {
        let text = self.text(body, field, 64, true)?;
        match Uuid::parse_str(&text) {
            Ok(u) => Some(u),
            Err(_) => {
                self.add(field, format!("{field} must be an id"));
                None
            }
        }
    }

    fn days(&mut self, body: &Value) -> Option<i32> {
        match body.get("review_every_days") {
            None | Some(Value::Null) => None,
            Some(v) => match v.as_i64().filter(|d| (1..=3650).contains(d)) {
                Some(d) => Some(d as i32),
                None => {
                    self.add(
                        "review_every_days",
                        "review_every_days is a whole number of days from 1 to 3650",
                    );
                    None
                }
            },
        }
    }

    fn owner(&mut self, state: &SharedState, body: &Value) -> Option<(ApproverSpec, Value)> {
        let Some(raw) = body.get("owner") else {
            self.add("owner", "owner is required");
            return None;
        };
        let spec: ApproverSpec = match serde_json::from_value(raw.clone()) {
            Ok(s) => s,
            Err(_) => {
                self.add(
                    "owner",
                    "owner is a Person, a Position or a Role, as in an approver spec",
                );
                return None;
            }
        };
        if let Err(why) = check_spec(state, &spec) {
            self.add("owner", why);
            return None;
        }
        Some((spec, raw.clone()))
    }

    pub(crate) fn done(self) -> Result<(), ServiceError> {
        if self.0.is_empty() {
            return Ok(());
        }
        let message = self
            .0
            .iter()
            .map(|(f, w)| format!("{f}: {w}"))
            .collect::<Vec<_>>()
            .join("; ");
        Err(ServiceError::Invalid {
            message,
            fields: self.0,
        })
    }
}

fn reason_of(p: &mut Problems, body: &Value) -> String {
    p.text(body, "reason", 500, true).unwrap_or_default()
}

pub(crate) fn is_admin(state: &SharedState, caller: &AuthUser) -> bool {
    require_platform_admin_or_compliance(caller, state).is_ok()
}

/// A Platform Admin, a caller with the compliance affiliation, or a person the owner spec resolves to today.
pub(crate) fn may_write(state: &SharedState, caller: &AuthUser, policy: &Value) -> bool {
    if is_admin(state, caller) {
        return true;
    }
    let (Some(unit), Ok(spec)) = (
        policy["organization_id"]
            .as_str()
            .and_then(|u| Uuid::parse_str(u).ok()),
        serde_json::from_value::<ApproverSpec>(policy["owner"].clone()),
    ) else {
        return false;
    };
    resolves_to(state, &spec, unit, &caller.eppn)
}

/// The owner of a policy in words a person can act on.
fn owner_text(owner: &Value) -> String {
    if let Some(e) = owner["Person"]["eppn"].as_str() {
        return e.to_string();
    }
    if let Some(k) = owner["Position"]["key"].as_str() {
        return format!("the {k} position");
    }
    if let Some(n) = owner["Role"]["name"].as_str() {
        return format!("the {n} role");
    }
    "not set".to_string()
}

pub(crate) fn forbidden(policy: Option<&Value>) -> ServiceError {
    ServiceError::forbidden(match policy {
        Some(p) => format!(
            "Only a Platform Admin, compliance, or the policy's owner may do this. The owner is {}",
            owner_text(&p["owner"])
        ),
        None => "Only a Platform Admin or compliance may do this".to_string(),
    })
}

/// The policy, when the caller may write it. Someone who may not is told so whether or not the id exists.
pub(crate) fn writable(state: &SharedState, caller: &AuthUser, id: Uuid) -> Result<Value, ServiceError> {
    match state.repository.library_policy(id).map_err(internal)? {
        Some(p) if may_write(state, caller, &p) => Ok(p),
        Some(p) => Err(forbidden(Some(&p))),
        None if is_admin(state, caller) => {
            Err(ServiceError::NotFound("Policy not found".to_string()))
        }
        None => Err(forbidden(None)),
    }
}

pub(crate) fn parse_id(text: &str, what: &str) -> Result<Uuid, ServiceError> {
    Uuid::parse_str(text).map_err(|_| ServiceError::NotFound(format!("{what} not found")))
}

pub fn create_policy(
    state: &SharedState,
    caller: &AuthUser,
    body: &Value,
) -> Result<Value, ServiceError> {
    if !is_admin(state, caller) {
        return Err(forbidden(None));
    }
    let mut p = Problems::default();
    let key = p.key(body, "key");
    let title = p.text(body, "title", 255, true);
    let summary = p.text(body, "summary", 5000, false).unwrap_or_default();
    let organization_id = p.uuid(body, "organization_id");
    let owner = p.owner(state, body);
    let days = p.days(body).unwrap_or(365);
    let reason = reason_of(&mut p, body);
    if let Some(org) = organization_id {
        if !state
            .organizations
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&org)
        {
            p.add(
                "organization_id",
                "organization_id is not a unit of the institution",
            );
        }
    }
    p.done()?;
    let (key, title, organization_id, (_, owner)) = (
        key.unwrap_or_default(),
        title.unwrap_or_default(),
        organization_id.unwrap_or_default(),
        owner.unwrap_or_default_owner(),
    );
    if state
        .repository
        .library_policy_key_exists(&key)
        .map_err(internal)?
    {
        return Err(ServiceError::conflict(format!(
            "A policy with the key '{key}' exists"
        )));
    }
    let payload = json!({ "action": "policy_created", "policy_key": key, "title": title, "organization_id": organization_id, "owner": owner });
    let policy = admin_write(
        state,
        caller,
        DecisionType::PolicyLibraryChanged,
        OSCAL_CONTROL,
        &reason,
        &payload,
        || {
            state
                .repository
                .library_create_policy(
                    NewPolicy {
                        key: key.clone(),
                        title,
                        summary,
                        organization_id,
                        owner,
                        review_every_days: days,
                    },
                    &caller.eppn,
                )
                .map_err(internal)
        },
    )?;
    Ok(json!({ "policy": policy }))
}

trait OwnerOrDefault {
    fn unwrap_or_default_owner(self) -> (ApproverSpec, Value);
}

impl OwnerOrDefault for Option<(ApproverSpec, Value)> {
    fn unwrap_or_default_owner(self) -> (ApproverSpec, Value) {
        self.unwrap_or((
            ApproverSpec::Role {
                name: String::new(),
            },
            Value::Null,
        ))
    }
}

pub fn list_policies(state: &SharedState, caller: &AuthUser) -> Result<Value, ServiceError> {
    let all = state.repository.library_policies().map_err(internal)?;
    let admin = is_admin(state, caller);
    let mine: Vec<Value> = all
        .into_iter()
        .filter(|p| admin || may_write(state, caller, p))
        .collect();
    if mine.is_empty() && !admin {
        return Err(forbidden(None));
    }
    Ok(json!({ "policies": mine }))
}

pub fn get_policy(state: &SharedState, caller: &AuthUser, id: &str) -> Result<Value, ServiceError> {
    let id = parse_id(id, "Policy")?;
    let policy = writable(state, caller, id)?;
    Ok(json!({
        "policy": policy,
        "sources": state.repository.library_sources_of(id).map_err(internal)?,
        "rule_counts": state.repository.library_rule_counts(id).map_err(internal)?,
        "open_questions": state.repository.library_open_questions(id).map_err(internal)?,
    }))
}

pub fn patch_policy(
    state: &SharedState,
    caller: &AuthUser,
    id: &str,
    body: &Value,
) -> Result<Value, ServiceError> {
    let id = parse_id(id, "Policy")?;
    let policy = writable(state, caller, id)?;
    let mut p = Problems::default();
    let mut patch = serde_json::Map::new();
    if body.get("title").is_some() {
        if let Some(t) = p.text(body, "title", 255, true) {
            patch.insert("title".into(), json!(t));
        }
    }
    if body.get("summary").is_some() {
        if let Some(t) = p.text(body, "summary", 5000, false) {
            patch.insert("summary".into(), json!(t));
        }
    }
    if body.get("owner").is_some() {
        if let Some((_, raw)) = p.owner(state, body) {
            patch.insert("owner".into(), raw);
        }
    }
    if body.get("review_every_days").is_some() {
        if let Some(d) = p.days(body) {
            patch.insert("review_every_days".into(), json!(d));
        }
    }
    if let Some(s) = body.get("status") {
        match s.as_str().filter(|s| ["Active", "Retired"].contains(s)) {
            Some(s) => {
                patch.insert("status".into(), json!(s));
            }
            None => p.add("status", "status is Active or Retired"),
        }
    }
    let reason = reason_of(&mut p, body);
    if patch.is_empty() && p.0.is_empty() {
        p.add("title", "name at least one field to change");
    }
    p.done()?;
    let payload = json!({ "action": "policy_changed", "policy_key": policy["key"], "changed": patch.keys().collect::<Vec<_>>() });
    let patch = Value::Object(patch);
    let updated = admin_write(
        state,
        caller,
        DecisionType::PolicyLibraryChanged,
        OSCAL_CONTROL,
        &reason,
        &payload,
        || {
            state
                .repository
                .library_update_policy(id, &patch)
                .map_err(internal)
        },
    )?;
    Ok(json!({ "policy": updated }))
}

pub fn add_source(
    state: &SharedState,
    caller: &AuthUser,
    policy_id: &str,
    body: &Value,
) -> Result<Value, ServiceError> {
    let policy_id = parse_id(policy_id, "Policy")?;
    let policy = writable(state, caller, policy_id)?;
    let mut p = Problems::default();
    let title = p.text(body, "title", 255, true);
    let kind = match body.get("kind").and_then(Value::as_str) {
        Some(k @ ("url" | "text")) => Some(k.to_string()),
        _ => {
            p.add("kind", "kind is url or text");
            None
        }
    };
    let mut href = None;
    if kind.as_deref() == Some("url") {
        let h = p.text(body, "href", 2000, true);
        if let Some(h) = h {
            if !h.starts_with("https://")
                || h.len() <= "https://".len()
                || h.contains(char::is_whitespace)
            {
                p.add("href", "href is an https address");
            } else {
                href = Some(h);
            }
        }
    }
    let reason = reason_of(&mut p, body);
    p.done()?;
    if state
        .repository
        .library_source_count(policy_id)
        .map_err(internal)?
        >= MAX_SOURCES
    {
        return Err(ServiceError::BadRequest(format!(
            "A policy has at most {MAX_SOURCES} sources"
        )));
    }
    let (title, kind) = (title.unwrap_or_default(), kind.unwrap_or_default());
    let payload = json!({ "action": "source_added", "policy_key": policy["key"], "title": title, "kind": kind, "href": href });
    let source = admin_write(
        state,
        caller,
        DecisionType::PolicyLibraryChanged,
        OSCAL_CONTROL,
        &reason,
        &payload,
        || {
            state
                .repository
                .library_add_source(policy_id, &title, &kind, href.as_deref(), &caller.eppn)
                .map_err(internal)
        },
    )?;
    Ok(json!({ "source": source }))
}

/// Stores the text a person pasted for a `text` source. The text is compared as the words it holds, so a change of spacing is
/// not a new snapshot. Returns whether a snapshot was stored and the snapshot that holds this text.
pub fn add_snapshot(
    state: &SharedState,
    caller: &AuthUser,
    source_id: &str,
    body: &Value,
) -> Result<(bool, Value), ServiceError> {
    let source_id = parse_id(source_id, "Source")?;
    let Some((policy_id, kind)) = state
        .repository
        .library_source(source_id)
        .map_err(internal)?
    else {
        return Err(if is_admin(state, caller) {
            ServiceError::NotFound("Source not found".to_string())
        } else {
            forbidden(None)
        });
    };
    let policy = writable(state, caller, policy_id)?;
    if kind != "text" {
        return Err(ServiceError::BadRequest(
            "A web source is fetched by the platform. Its text cannot be supplied".to_string(),
        ));
    }
    let mut p = Problems::default();
    let raw = match body.get("text") {
        Some(Value::String(s)) => Some(s.clone()),
        None | Some(Value::Null) => {
            p.add("text", "text is required");
            None
        }
        Some(_) => {
            p.add("text", "text must be text");
            None
        }
    };
    let reason = reason_of(&mut p, body);
    p.done()?;
    let raw = raw.unwrap_or_default();
    if raw.len() > MAX_SNAPSHOT_BYTES {
        return Err(ServiceError::TooLarge(format!(
            "A snapshot's text is at most {} MB",
            MAX_SNAPSHOT_BYTES / 1024 / 1024
        )));
    }
    let text = normalize_text("text/plain", raw.as_bytes());
    if text.is_empty() {
        let mut p = Problems::default();
        p.add("text", "text holds no words");
        p.done()?;
    }
    let text_sha = hex(&Sha256::digest(text.as_bytes()));
    if let Some((id, latest_sha)) = state
        .repository
        .library_latest_snapshot(source_id)
        .map_err(internal)?
    {
        if latest_sha == text_sha {
            return Ok((
                false,
                json!({ "id": id, "source_id": source_id, "text_sha256": text_sha }),
            ));
        }
    }
    let content_sha = hex(&Sha256::digest(raw.as_bytes()));
    let payload = json!({ "action": "snapshot_taken", "policy_key": policy["key"], "source_id": source_id, "text_sha256": text_sha, "text_origin": "supplied" });
    let snapshot = admin_write(
        state,
        caller,
        DecisionType::PolicyLibraryChanged,
        OSCAL_CONTROL,
        &reason,
        &payload,
        || {
            state
                .repository
                .library_add_snapshot(
                    source_id,
                    "text/plain",
                    &content_sha,
                    &text,
                    &text_sha,
                    "supplied",
                    &caller.eppn,
                )
                .map_err(internal)
        },
    )?;
    Ok((true, snapshot))
}

/// Sentences in a statement: stops (. ! ?) that are followed by a space and more text, plus the last one.
pub(crate) fn sentences(text: &str) -> usize {
    let chars: Vec<char> = text.trim().chars().collect();
    let stops = chars
        .windows(3)
        .filter(|w| {
            matches!(w[0], '.' | '!' | '?') && w[1].is_whitespace() && !w[2].is_whitespace()
        })
        .count();
    stops + 1
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn get_snapshot(
    state: &SharedState,
    caller: &AuthUser,
    id: &str,
) -> Result<Value, ServiceError> {
    let id = parse_id(id, "Snapshot")?;
    let Some((policy_id, snapshot)) = state.repository.library_snapshot(id).map_err(internal)?
    else {
        return Err(if is_admin(state, caller) {
            ServiceError::NotFound("Snapshot not found".to_string())
        } else {
            forbidden(None)
        });
    };
    writable(state, caller, policy_id)?;
    Ok(json!({ "snapshot": snapshot }))
}

/// One citation of a rule: the snapshot must be this policy's, and the passage must be in its text.
fn check_citations(
    state: &SharedState,
    policy_id: Uuid,
    body: &Value,
    p: &mut Problems,
) -> Result<Vec<NewCitation>, ServiceError> {
    let list = match body.get("citations") {
        Some(Value::Array(l)) if !l.is_empty() => l,
        Some(Value::Array(_)) | None => {
            p.add("citations", "a rule needs at least one citation");
            return Ok(Vec::new());
        }
        Some(_) => {
            p.add("citations", "citations is a list");
            return Ok(Vec::new());
        }
    };
    let mut out = Vec::new();
    for (i, c) in list.iter().enumerate() {
        let field = |f: &str| format!("citations[{i}].{f}");
        let snapshot_id = p.uuid(c, "snapshot_id").map(|u| (u, field("snapshot_id")));
        let passage = p.text(c, "passage", 2000, true);
        let locator = p.text(c, "locator", 255, false).unwrap_or_default();
        // The helper above names the bare field. Rename what it recorded to the indexed field.
        for f in ["snapshot_id", "passage", "locator"] {
            if let Some(why) = p.0.remove(f) {
                p.0.insert(field(f), why);
            }
        }
        let (Some((snapshot_id, snapshot_field)), Some(passage)) = (snapshot_id, passage) else {
            continue;
        };
        let Some((owner_policy, snapshot)) = state
            .repository
            .library_snapshot(snapshot_id)
            .map_err(internal)?
        else {
            p.add(&snapshot_field, "no such snapshot");
            continue;
        };
        if owner_policy != policy_id {
            p.add(&snapshot_field, "that snapshot belongs to another policy");
            continue;
        }
        if !passage_in(snapshot["text"].as_str().unwrap_or_default(), &passage) {
            let shown: String = passage.chars().take(120).collect();
            return Err(ServiceError::CitationNotFound(format!("The passage \"{shown}\" is not in the snapshot. A passage is 20 to 2000 characters, copied word for word.")));
        }
        out.push(NewCitation {
            snapshot_id,
            passage,
            locator,
        });
    }
    Ok(out)
}

/// A person drafts a rule. It is stored as the next version of its key, as a draft.
pub fn draft_rule(
    state: &SharedState,
    caller: &AuthUser,
    policy_id: &str,
    body: &Value,
) -> Result<Value, ServiceError> {
    draft_rule_via(state, caller, policy_id, body, "person")
}

/// `via` is `person` for a route and `agent` for a tool. Both run every check and write one ledger entry.
pub fn draft_rule_via(
    state: &SharedState,
    caller: &AuthUser,
    policy_id: &str,
    body: &Value,
    via: &str,
) -> Result<Value, ServiceError> {
    let policy_id = parse_id(policy_id, "Policy")?;
    let policy = writable(state, caller, policy_id)?;
    let mut p = Problems::default();
    let key = p.key(body, "key");
    // These words are routes under /library/rules/{id}/..., so a rule of that key could not be read at its address.
    if key.as_deref().is_some_and(|k| ["review", "submit", "decide", "retire"].contains(&k)) {
        p.add("key", "key may not be review, submit, decide or retire");
    }
    let statement = p.text(body, "statement", 500, true);
    if statement.as_deref().is_some_and(|s| sentences(s) > 2) {
        p.add("statement", "a statement is one sentence or two");
    }
    let applies_to = p.text(body, "applies_to", 1000, false).unwrap_or_default();
    let reason = reason_of(&mut p, body);
    let citations = check_citations(state, policy_id, body, &mut p)?;
    p.done()?;
    let (key, statement) = (key.unwrap_or_default(), statement.unwrap_or_default());
    if state
        .repository
        .library_rule_key_count(policy_id)
        .map_err(internal)?
        >= MAX_RULE_KEYS
    {
        return Err(ServiceError::BadRequest(format!(
            "A policy has at most {MAX_RULE_KEYS} rule keys"
        )));
    }
    let payload = json!({ "action": "rule_drafted", "policy_key": policy["key"], "rule_key": key, "citations": citations.len(), "submitted_via": via });
    let rule = admin_write(
        state,
        caller,
        DecisionType::PolicyLibraryChanged,
        OSCAL_CONTROL,
        &reason,
        &payload,
        || {
            state
                .repository
                .library_draft_rule(
                    policy_id,
                    &key,
                    &statement,
                    &applies_to,
                    &caller.eppn,
                    via,
                    citations,
                )
                .map_err(internal)
        },
    )?;
    Ok(json!({ "rule": rule }))
}

// --- what the MCP tools call --------------------------------------------------------------------------------------

const MAX_READ_CHARS: usize = 20_000;
const AGENT_REASON: &str = "Proposed by the person's AI tool";

/// Every policy, as the summary a person or an agent picks from.
pub fn policy_summaries(state: &SharedState) -> Result<Value, ServiceError> {
    let mut out = Vec::new();
    for p in state.repository.library_policies().map_err(internal)? {
        let id = p["id"].as_str().and_then(|u| Uuid::parse_str(u).ok());
        let counts = match id {
            Some(id) => state.repository.library_rule_counts(id).map_err(internal)?,
            None => json!({}),
        };
        out.push(json!({ "id": p["id"], "key": p["key"], "title": p["title"], "owner": p["owner"], "review_every_days": p["review_every_days"], "status": p["status"], "rule_counts": counts }));
    }
    Ok(json!({ "policies": out }))
}

/// One page of a snapshot's text, for a caller who may write its policy. The text is returned only as `untrusted_text`.
pub fn read_snapshot_page(
    state: &SharedState,
    caller: &AuthUser,
    id: &str,
    offset: usize,
    limit: usize,
) -> Result<Value, ServiceError> {
    let id = parse_id(id, "Snapshot")?;
    let Some((policy_id, snapshot)) = state.repository.library_snapshot(id).map_err(internal)? else {
        return Err(if is_admin(state, caller) {
            ServiceError::NotFound("Snapshot not found".to_string())
        } else {
            forbidden(None)
        });
    };
    writable(state, caller, policy_id)?;
    let limit = limit.clamp(1, MAX_READ_CHARS);
    let text = snapshot["text"].as_str().unwrap_or_default();
    let total = text.chars().count();
    let page: String = text.chars().skip(offset).take(limit).collect();
    let end = offset.saturating_add(page.chars().count());
    Ok(json!({
        "snapshot_id": snapshot["id"],
        "source_title": snapshot["source_title"],
        "taken_at": snapshot["taken_at"],
        "untrusted_text": page,
        "next_offset": if end < total { json!(end) } else { Value::Null },
    }))
}

/// Published rules that match, each with its policy and whether anything enforces it.
pub fn find_rules(state: &SharedState, query: &str, applies_to: &str) -> Result<Value, ServiceError> {
    let mut rules = state.repository.library_published_rules(query, applies_to).map_err(internal)?;
    for r in &mut rules {
        let page = format!("/policies/{}", r["policy_key"].as_str().unwrap_or_default());
        let pid = r["policy_id"].as_str().and_then(|u| Uuid::parse_str(u).ok()).unwrap_or_default();
        r["enforcement"] = json!(crate::service::library_public::enforcement_for(state, pid, r["key"].as_str().unwrap_or_default()));
        r["page"] = json!(page);
    }
    Ok(json!({ "rules": rules }))
}

fn outcome(key: &str, result: Result<Value, ServiceError>) -> Value {
    match result {
        Ok(_) => json!({ "key": key, "stored": true }),
        Err(e) => json!({ "key": key, "stored": false, "code": e.code(), "reason": e.message() }),
    }
}

/// Stores each proposed rule as a draft and each question, in the caller's name. One bad item does not stop the others.
pub fn propose_rules(
    state: &SharedState,
    caller: &AuthUser,
    policy_id: &str,
    rules: &[Value],
    questions: &[Value],
) -> Result<Value, ServiceError> {
    let id = parse_id(policy_id, "Policy")?;
    let policy = writable(state, caller, id)?;
    let mut rule_out = Vec::new();
    for r in rules {
        let mut body = r.clone();
        if let Some(o) = body.as_object_mut() {
            o.insert("reason".to_string(), json!(AGENT_REASON));
        }
        let key = r["key"].as_str().unwrap_or_default().to_string();
        rule_out.push(outcome(&key, draft_rule_via(state, caller, policy_id, &body, "agent")));
    }
    let mut question_out = Vec::new();
    for (i, q) in questions.iter().enumerate() {
        let label = format!("question {}", i + 1);
        question_out.push(outcome(&label, ask_question(state, caller, id, &policy, q)));
    }
    Ok(json!({ "rules": rule_out, "questions": question_out }))
}

fn ask_question(
    state: &SharedState,
    caller: &AuthUser,
    policy_id: Uuid,
    policy: &Value,
    q: &Value,
) -> Result<Value, ServiceError> {
    let mut p = Problems::default();
    let question = p.text(q, "question", 2000, true);
    let context = p.text(q, "context_passage", 2000, false).unwrap_or_default();
    let snapshot = match q.get("snapshot_id") {
        None | Some(Value::Null) => None,
        Some(_) => p.uuid(q, "snapshot_id"),
    };
    if let Some(s) = snapshot {
        match state.repository.library_snapshot(s).map_err(internal)? {
            Some((owner, _)) if owner == policy_id => {}
            _ => p.add("snapshot_id", "no such snapshot in this policy"),
        }
    }
    p.done()?;
    let question = question.unwrap_or_default();
    let payload = json!({ "action": "question_asked", "policy_key": policy["key"], "asked_via": "agent" });
    admin_write(
        state,
        caller,
        DecisionType::PolicyLibraryChanged,
        OSCAL_CONTROL,
        AGENT_REASON,
        &payload,
        || {
            state
                .repository
                .library_add_question(policy_id, &question, &context, snapshot, &caller.eppn, "agent")
                .map_err(internal)
        },
    )
}

/// Open questions for the policies the caller may write.
pub fn open_questions(state: &SharedState, caller: &AuthUser) -> Result<Value, ServiceError> {
    let admin = is_admin(state, caller);
    let mut out = Vec::new();
    for q in state.repository.library_open_questions_all().map_err(internal)? {
        let allowed = admin
            || q["policy_id"]
                .as_str()
                .and_then(|u| Uuid::parse_str(u).ok())
                .and_then(|id| state.repository.library_policy(id).ok().flatten())
                .is_some_and(|p| may_write(state, caller, &p));
        if allowed {
            out.push(q);
        }
    }
    Ok(json!({ "questions": out }))
}
