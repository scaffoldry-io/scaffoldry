//! NIST OSCAL 1.1.2 Compliance Catalog and Governance Export

use crate::state::SharedState;
use axum::{response::IntoResponse, routing::get, Json, Router};
use serde_json::json;

pub fn router() -> Router<SharedState> {
    Router::new().route("/governance/oscal", get(get_oscal_catalog))
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
                    "implementation": "Immutable append-only audit ledger and proposal branches"
                }
            ]
        }
    });

    Json(oscal)
}
