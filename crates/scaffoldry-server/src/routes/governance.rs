use crate::guard::session_user;
use crate::state::{RecordDecisionInput, SharedState};
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use scaffoldry_core::{DecisionType, GENESIS_PREVIOUS_HASH};
use serde::Deserialize;
use serde_json::{json, Value};
use crate::service::ServiceError;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/governance/oscal", get(get_oscal_catalog))
        .route("/governance/oscal/export", get(export_oscal_component_definition))
        .route("/governance/ledger", get(get_governance_ledger))
        .route("/governance/ledger/append", post(append_ledger_decision))
        .route("/governance/ledger/verify", post(verify_governance_ledger))
}

#[derive(Debug, Deserialize)]
pub struct AppendDecisionRequest {
    #[serde(default)]
    pub principal: Option<String>,
    pub organization_code: String,
    pub app_slug: Option<String>,
    pub decision_type: DecisionType,
    pub oscal_control_id: String,
    pub rationale: String,
    #[serde(default)]
    pub payload: Value,
}

const DEFAULT_PAGE: i64 = 100;
const MAX_PAGE: i64 = 1000;

#[derive(Debug, Default, Deserialize)]
struct LedgerQuery {
    limit: Option<i64>,
    /// The sequence the page starts under: entries older than it.
    before: Option<u64>,
    /// Also run the full chain check. It reads the whole chain, so it is not the default.
    verify: Option<bool>,
}

async fn get_governance_ledger(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(query): Query<LedgerQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    if user.affiliation != "central_admin" && user.affiliation != "compliance" {
        return Err(ServiceError::forbidden("Forbidden: Requires Platform Admin or compliance affiliation").into_pair());
    }

    // One page, newest first. The whole chain is not read to answer it, and not checked unless the caller asks.
    let limit = query.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let entries = state
        .repository
        .ledger_page(query.before, limit)
        .map_err(|e| ServiceError::StorageUnavailable(e.to_string()).into_pair())?;
    let (total, head) = state
        .repository
        .ledger_summary()
        .map_err(|e| ServiceError::StorageUnavailable(e.to_string()).into_pair())?;
    let chain_valid = if query.verify.unwrap_or(false) { Some(state.verify_ledger().unwrap_or(false)) } else { None };
    // A full page may have more behind it. Its cursor is the sequence of its oldest entry.
    let next_cursor = if entries.len() as i64 == limit { entries.last().map(|e| e.sequence).filter(|s| *s > 0) } else { None };

    Ok(Json(json!({
        "chain_valid": chain_valid,
        "total_entries": total,
        "head_hash": head.unwrap_or_else(|| GENESIS_PREVIOUS_HASH.to_string()),
        "next_cursor": next_cursor,
        "entries": entries
    })))
}

async fn append_ledger_decision(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<AppendDecisionRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    crate::service::access::require(
        &state,
        &user,
        "record_decision",
        &scaffoldry_policy::entities::Resource::System(crate::service::access::ledger_resource()),
        "Forbidden: Cedar policy denied ledger append",
    )
    .map_err(|e| e.into_pair())?;

    match state.append_ledger_entry(RecordDecisionInput {
        principal: user.eppn,
        organization_code: payload.organization_code,
        app_slug: payload.app_slug,
        decision_type: payload.decision_type,
        oscal_control_id: payload.oscal_control_id,
        rationale: payload.rationale,
        payload: &payload.payload,
    }) {
        Ok(entry) => Ok((StatusCode::CREATED, Json(json!({ "entry": entry, "success": true })))),
        Err(e) => Err(ServiceError::internal(format!("Failed to append to ledger: {e}")).into_pair()),
    }
}

async fn verify_governance_ledger(State(state): State<SharedState>) -> impl IntoResponse {
    let total = state.repository.ledger_summary().map(|(n, _)| n).unwrap_or(0);
    let head = state.repository.ledger_summary().ok().and_then(|(_, h)| h);
    match state.repository.verify_ledger_chain() {
        Ok(report) => Json(json!({
            "verified": true,
            "total_entries": report.entries,
            "genesis_previous_hash": GENESIS_PREVIOUS_HASH,
            "head_hash": head.as_deref().unwrap_or(GENESIS_PREVIOUS_HASH),
            "verification_status": "All cryptographic SHA-256 blocks verified intact without drift or tampering"
        })),
        Err(e) => Json(json!({
            "verified": false,
            "total_entries": total,
            "error": format!("Ledger chain verification failed: {e}"),
            "verification_status": "Chain broken or signature mismatch detected"
        })),
    }
}

async fn export_oscal_component_definition(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    if user.affiliation != "central_admin" && user.affiliation != "compliance" {
        return Err(ServiceError::forbidden("Forbidden: Requires Platform Admin or compliance affiliation").into_pair());
    }

    let doc = state.export_oscal_component_definition();
    Ok(Json(doc))
}

async fn get_oscal_catalog() -> impl IntoResponse {
    let oscal = json!({
        "schema_version": "1.1.2",
        "catalog": {
            "uuid": "8b5123d5-3be7-4a0b-9df2-4752b5ee4759",
            "metadata": {
                "title": "Scaffoldry Institutional Security and Compliance Lattice",
                "version": "1.0",
                "last_modified": "2026-10-04T12:00:00Z",
                "oscal_version": "1.1.2"
            },
            "controls": [
                {
                    "id": "ac-03",
                    "title": "Access Enforcement",
                    "framework": "NIST SP 800-53 Rev 5",
                    "status": "Automated",
                    "implementation": "Cedar Policy Engine ABAC with sub-millisecond evaluation at API boundary"
                },
                {
                    "id": "ia-02",
                    "title": "Identification and Authentication",
                    "framework": "NIST SP 800-53 Rev 5",
                    "status": "Automated",
                    "implementation": "SCIM 2.0 (RFC 7643 / RFC 7644) and eduPerson scoped affiliation tokens"
                },
                {
                    "id": "mp-04",
                    "title": "Media Transport / Privacy Export",
                    "framework": "FERPA / 34 CFR Part 99",
                    "status": "Automated",
                    "implementation": "Strict Cedar forbid rule for records flagged ferpa_sensitive without compliance affiliation"
                },
                {
                    "id": "au-02",
                    "title": "Event Logging & Audit Ledger",
                    "framework": "NIST SP 800-53 Rev 5",
                    "status": "Automated",
                    "implementation": "Immutable append-only cryptographic SHA-256 ledger"
                }
            ]
        }
    });

    Json(oscal)
}
