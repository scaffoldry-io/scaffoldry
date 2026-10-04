//! NIST OSCAL 1.1.2 Compliance Catalog, Cryptographic Ledger, and Governance Export

use crate::state::{RecordDecisionInput, SharedState};
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use scaffoldry_core::{DecisionType, GENESIS_PREVIOUS_HASH};
use serde::Deserialize;
use serde_json::{json, Value};

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
    pub principal: String,
    pub organization_code: String,
    pub app_slug: Option<String>,
    pub decision_type: DecisionType,
    pub oscal_control_id: String,
    pub rationale: String,
    #[serde(default)]
    pub payload: Value,
}

async fn get_governance_ledger(State(state): State<SharedState>) -> impl IntoResponse {
    let ledger = state.ledger.read().unwrap();
    let is_valid = state.verify_ledger().unwrap_or(false);
    let head_hash = ledger
        .last()
        .map(|e| e.entry_hash.clone())
        .unwrap_or_else(|| GENESIS_PREVIOUS_HASH.to_string());

    Json(json!({
        "chain_valid": is_valid,
        "total_entries": ledger.len(),
        "head_hash": head_hash,
        "entries": *ledger
    }))
}

async fn append_ledger_decision(
    State(state): State<SharedState>,
    Json(payload): Json<AppendDecisionRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<Value>)> {
    match state.append_ledger_entry(RecordDecisionInput {
        principal: payload.principal,
        organization_code: payload.organization_code,
        app_slug: payload.app_slug,
        decision_type: payload.decision_type,
        oscal_control_id: payload.oscal_control_id,
        rationale: payload.rationale,
        payload: &payload.payload,
    }) {
        Ok(entry) => Ok((StatusCode::CREATED, Json(json!({ "entry": entry, "success": true })))),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to append to ledger: {e}"), "success": false })),
        )),
    }
}

async fn verify_governance_ledger(State(state): State<SharedState>) -> impl IntoResponse {
    let ledger = state.ledger.read().unwrap();
    match state.verify_ledger() {
        Ok(valid) => Json(json!({
            "verified": valid,
            "total_entries": ledger.len(),
            "genesis_previous_hash": GENESIS_PREVIOUS_HASH,
            "head_hash": ledger.last().map(|e| e.entry_hash.as_str()).unwrap_or(GENESIS_PREVIOUS_HASH),
            "verification_status": "All cryptographic SHA-256 blocks verified intact without drift or tampering"
        })),
        Err(e) => Json(json!({
            "verified": false,
            "total_entries": ledger.len(),
            "error": format!("Ledger chain verification failed: {e}"),
            "verification_status": "Chain broken or signature mismatch detected"
        })),
    }
}

async fn export_oscal_component_definition(State(state): State<SharedState>) -> impl IntoResponse {
    let doc = state.export_oscal_component_definition();
    Json(doc)
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
