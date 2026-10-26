//! Policy library phase 3: review, answers, and publishing.
//!
//! A rule moves Draft, InReview, Published or Rejected. The reviewers of a rule are the people the policy's owner
//! resolves to today, less the person who submitted it. A Platform Admin is a reviewer only when the owner names them.

use crate::service::admin::admin_write;
use crate::service::approvers::resolved_approvers;
use crate::service::library::{forbidden, hex, internal, is_admin, may_write, parse_id, sentences, writable, Problems, OSCAL_CONTROL};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::approver::ApproverSpec;
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_core::library::context_around;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use uuid::Uuid;

const CONTEXT_CHARS: usize = 300;
const DECISION_CONTROL: &str = "AC-05";

fn statement_sha256(statement: &str) -> String {
    hex(&Sha256::digest(statement.as_bytes()))
}

fn load(state: &SharedState, id: &str) -> Result<(Uuid, Value, Value), ServiceError> {
    let id = parse_id(id, "Rule")?;
    let rule = state.repository.library_rule(id).map_err(internal)?.ok_or_else(|| ServiceError::NotFound("Rule not found".to_string()))?;
    let policy_id = rule["policy_id"].as_str().and_then(|u| Uuid::parse_str(u).ok()).ok_or_else(|| internal("rule without policy"))?;
    let policy = state.repository.library_policy(policy_id).map_err(internal)?.ok_or_else(|| ServiceError::NotFound("Policy not found".to_string()))?;
    Ok((id, rule, policy))
}

/// The people who may decide `rule` today.
fn reviewers(state: &SharedState, policy: &Value, rule: &Value) -> Vec<String> {
    let (Some(unit), Ok(spec)) = (policy["organization_id"].as_str().and_then(|u| Uuid::parse_str(u).ok()), serde_json::from_value::<ApproverSpec>(policy["owner"].clone())) else { return Vec::new() };
    let submitter = rule["submitted_by"].as_str().unwrap_or_default();
    resolved_approvers(state, &spec, unit, &[], Some(submitter)).into_iter().filter(|e| !e.eq_ignore_ascii_case(submitter)).collect()
}

fn require_reviewer(state: &SharedState, caller: &AuthUser, policy: &Value, rule: &Value) -> Result<(), ServiceError> {
    if reviewers(state, policy, rule).iter().any(|e| e.eq_ignore_ascii_case(&caller.eppn)) {
        return Ok(());
    }
    Err(ServiceError::forbidden("Only a reviewer may do this. A reviewer is a person the policy's owner resolves to, other than the person who submitted the rule"))
}

fn summary(rule: &Value, no_approver: bool) -> Value {
    json!({
        "rule_id": rule["id"], "policy_id": rule["policy_id"], "policy_title": rule["policy_title"], "key": rule["key"], "version": rule["version"],
        "statement": rule["statement"], "submitted_by": rule["submitted_by"], "submitted_via": rule["submitted_via"], "submitted_at": rule["submitted_at"],
        "no_approver": no_approver,
    })
}

fn status_of(rule: &Value) -> &str {
    rule["status"].as_str().unwrap_or_default()
}

/// Draft to InReview, by the person who submitted it or a person who may write the policy.
pub fn submit(state: &SharedState, caller: &AuthUser, id: &str) -> Result<Value, ServiceError> {
    let (rule_id, rule, policy) = load(state, id)?;
    if !caller.eppn.eq_ignore_ascii_case(rule["submitted_by"].as_str().unwrap_or_default()) && !may_write(state, caller, &policy) {
        return Err(forbidden(Some(&policy)));
    }
    if status_of(&rule) != "Draft" {
        return Err(ServiceError::Conflict("Only a draft is submitted for review".to_string()));
    }
    let payload = json!({ "action": "rule_submitted", "policy_key": policy["key"], "rule_key": rule["key"], "version": rule["version"] });
    admin_write(state, caller, DecisionType::PolicyLibraryChanged, OSCAL_CONTROL, "Submitted for review", &payload, || {
        if !state.repository.library_move_rule(rule_id, &["Draft"], "InReview", None, "").map_err(internal)? {
            return Err(ServiceError::Conflict("Only a draft is submitted for review".to_string()));
        }
        Ok(json!({ "rule": state.repository.library_rule(rule_id).map_err(internal)? }))
    })
}

/// What the caller can act on: rules they may decide, rules nobody may decide (for a writer to see), and open questions.
pub fn my_reviews(state: &SharedState, caller: &AuthUser) -> Result<Value, ServiceError> {
    let mut policies: HashMap<Uuid, Option<Value>> = HashMap::new();
    let (mut reviews, mut unassigned) = (Vec::new(), Vec::new());
    for rule in state.repository.library_rules_in_review().map_err(internal)? {
        let pid = rule["policy_id"].as_str().and_then(|u| Uuid::parse_str(u).ok()).unwrap_or_default();
        let policy = policies.entry(pid).or_insert_with(|| state.repository.library_policy(pid).ok().flatten()).clone();
        let Some(policy) = policy else { continue };
        let who = reviewers(state, &policy, &rule);
        if who.is_empty() {
            if is_admin(state, caller) || may_write(state, caller, &policy) {
                unassigned.push(summary(&rule, true));
            }
        } else if who.iter().any(|e| e.eq_ignore_ascii_case(&caller.eppn)) {
            reviews.push(summary(&rule, false));
        }
    }
    let admin = is_admin(state, caller);
    let mut questions = Vec::new();
    for q in state.repository.library_open_questions_all().map_err(internal)? {
        let pid = q["policy_id"].as_str().and_then(|u| Uuid::parse_str(u).ok()).unwrap_or_default();
        let policy = policies.entry(pid).or_insert_with(|| state.repository.library_policy(pid).ok().flatten()).clone();
        if admin || policy.is_some_and(|p| may_write(state, caller, &p)) {
            questions.push(q);
        }
    }
    Ok(json!({ "reviews": reviews, "unassigned": unassigned, "questions": questions }))
}

/// Everything a reviewer reads before deciding.
pub fn review_view(state: &SharedState, caller: &AuthUser, id: &str) -> Result<Value, ServiceError> {
    let (_, rule, policy) = load(state, id)?;
    require_reviewer(state, caller, &policy, &rule)?;
    let mut citations = Vec::new();
    for c in rule["citations"].as_array().cloned().unwrap_or_default() {
        let snapshot = c["snapshot_id"].as_str().and_then(|u| Uuid::parse_str(u).ok()).and_then(|u| state.repository.library_snapshot(u).ok().flatten());
        let (before, after, origin, title) = match &snapshot {
            Some((_, s)) => {
                let (b, a) = context_around(s["text"].as_str().unwrap_or_default(), c["passage"].as_str().unwrap_or_default(), CONTEXT_CHARS).unwrap_or_default();
                (b, a, s["text_origin"].clone(), s["source_title"].clone())
            }
            None => (String::new(), String::new(), Value::Null, Value::Null),
        };
        citations.push(json!({ "snapshot_id": c["snapshot_id"], "passage": c["passage"], "locator": c["locator"], "before": before, "after": after, "text_origin": origin, "source_title": title }));
    }
    let pid = rule["policy_id"].as_str().and_then(|u| Uuid::parse_str(u).ok()).unwrap_or_default();
    let version = rule["version"].as_i64().unwrap_or(1) as i32;
    let previous = state.repository.library_previous_version(pid, rule["key"].as_str().unwrap_or_default(), version).map_err(internal)?;
    let by = rule["submitted_by"].as_str().unwrap_or_default().to_string();
    let name = state.repository.users_by_names(std::slice::from_ref(&by)).ok().and_then(|u| u.first().map(crate::service::people::display_name)).unwrap_or_else(|| by.clone());
    Ok(json!({
        "rule": rule,
        "policy": { "id": policy["id"], "key": policy["key"], "title": policy["title"] },
        "citations": citations,
        "questions": state.repository.library_questions_of(pid).map_err(internal)?,
        "previous": previous,
        "submitted_by": by,
        "submitted_by_name": name,
        "submitted_via": rule["submitted_via"],
    }))
}

/// A reviewer publishes or rejects a rule they did not submit. A reviewer who rewords it has made the next version,
/// which a different reviewer decides.
pub fn decide(state: &SharedState, caller: &AuthUser, id: &str, body: &Value) -> Result<Value, ServiceError> {
    let (rule_id, rule, policy) = load(state, id)?;
    require_reviewer(state, caller, &policy, &rule)?;
    let mut p = Problems::default();
    let decision = match body.get("decision").and_then(Value::as_str) {
        Some(d @ ("publish" | "reject")) => Some(d),
        _ => {
            p.add("decision", "decision is publish or reject");
            None
        }
    };
    let note = p.text(body, "note", 1000, true).unwrap_or_default();
    let statement = p.text(body, "statement", 500, false).filter(|s| !s.is_empty());
    if statement.as_deref().is_some_and(|s| sentences(s) > 2) {
        p.add("statement", "a statement is one sentence or two");
    }
    let applies_to = p.text(body, "applies_to", 1000, false);
    p.done()?;
    if status_of(&rule) != "InReview" {
        return Err(ServiceError::Conflict("This rule is not waiting for a decision".to_string()));
    }
    let version = rule["version"].clone();
    let rule_statement = rule["statement"].as_str().unwrap_or_default().to_string();
    let reworded = decision == Some("publish")
        && (statement.as_deref().is_some_and(|s| s != rule_statement) || applies_to.as_deref().is_some_and(|a| a != rule["applies_to"].as_str().unwrap_or_default()));
    let new_statement = statement.clone().unwrap_or_else(|| rule_statement.clone());
    let new_applies = applies_to.clone().unwrap_or_else(|| rule["applies_to"].as_str().unwrap_or_default().to_string());
    let label = if reworded { "reworded" } else { decision.unwrap_or_default() };
    let hashed = if reworded { statement_sha256(&new_statement) } else { statement_sha256(&rule_statement) };
    let payload = json!({ "action": "rule_decided", "decision": label, "policy_key": policy["key"], "rule_key": rule["key"], "rule_id": rule_id, "version": version, "statement_sha256": hashed });
    admin_write(state, caller, DecisionType::PolicyRuleDecided, DECISION_CONTROL, &note, &payload, || {
        let conflict = || ServiceError::Conflict("This rule is not waiting for a decision".to_string());
        if reworded {
            let new = state.repository.library_reword_rule(rule_id, &new_statement, &new_applies, &caller.eppn, &note).map_err(internal)?.ok_or_else(conflict)?;
            return Ok(json!({ "reworded": true, "rule": state.repository.library_rule(new).map_err(internal)? }));
        }
        let done = match decision {
            Some("publish") => state.repository.library_publish_rule(rule_id, &caller.eppn, &note).map_err(internal)?,
            _ => state.repository.library_move_rule(rule_id, &["InReview"], "Rejected", Some(&caller.eppn), &note).map_err(internal)?,
        };
        if !done {
            return Err(conflict());
        }
        Ok(json!({ "reworded": false, "rule": state.repository.library_rule(rule_id).map_err(internal)? }))
    })
}

/// A policy writer answers a question. The answer is kept with who gave it and when.
pub fn answer_question(state: &SharedState, caller: &AuthUser, id: &str, body: &Value) -> Result<Value, ServiceError> {
    let qid = parse_id(id, "Question")?;
    let (policy_id, question) = state.repository.library_question(qid).map_err(internal)?.ok_or_else(|| ServiceError::NotFound("Question not found".to_string()))?;
    let policy = writable(state, caller, policy_id)?;
    let mut p = Problems::default();
    let answer = p.text(body, "answer", 4000, true).unwrap_or_default();
    p.done()?;
    if !question["answer"].is_null() {
        return Err(ServiceError::Conflict("This question is already answered".to_string()));
    }
    let payload = json!({ "action": "question_answered", "policy_key": policy["key"], "question_id": qid });
    admin_write(state, caller, DecisionType::PolicyLibraryChanged, OSCAL_CONTROL, "Answered a question", &payload, || {
        if !state.repository.library_answer_question(qid, &answer, &caller.eppn).map_err(internal)? {
            return Err(ServiceError::Conflict("This question is already answered".to_string()));
        }
        let (_, q) = state.repository.library_question(qid).map_err(internal)?.ok_or_else(|| internal("question gone"))?;
        Ok(json!({ "question": q }))
    })
}

/// A reviewer takes a published rule out of use.
pub fn retire(state: &SharedState, caller: &AuthUser, id: &str, body: &Value) -> Result<Value, ServiceError> {
    let (rule_id, rule, policy) = load(state, id)?;
    require_reviewer(state, caller, &policy, &rule)?;
    let mut p = Problems::default();
    let reason = p.text(body, "reason", 1000, true).unwrap_or_default();
    p.done()?;
    if status_of(&rule) != "Published" {
        return Err(ServiceError::Conflict("Only a published rule is retired".to_string()));
    }
    let payload = json!({ "action": "rule_decided", "decision": "retired", "policy_key": policy["key"], "rule_key": rule["key"], "rule_id": rule_id, "version": rule["version"], "statement_sha256": statement_sha256(rule["statement"].as_str().unwrap_or_default()) });
    admin_write(state, caller, DecisionType::PolicyRuleDecided, DECISION_CONTROL, &reason, &payload, || {
        if !state.repository.library_move_rule(rule_id, &["Published"], "Retired", Some(&caller.eppn), &reason).map_err(internal)? {
            return Err(ServiceError::Conflict("Only a published rule is retired".to_string()));
        }
        Ok(json!({ "rule": state.repository.library_rule(rule_id).map_err(internal)? }))
    })
}
