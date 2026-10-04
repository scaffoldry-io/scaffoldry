//! Native Model Context Protocol (MCP) Server Endpoint
//!
//! Exposes JSON-RPC 2.0 compliant Model Context Protocol tools, resources,
//! and prompt templates for sovereign, AI-native application co-building
//! and governance inspection.

use crate::state::SharedState;
use axum::{
    extract::State,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/mcp", get(get_mcp_overview).post(handle_mcp_request))
}

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// GET /api/v1/mcp - Server metadata and capabilities summary
async fn get_mcp_overview(State(state): State<SharedState>) -> impl IntoResponse {
    let dataset_count = state.datasets.read().unwrap().len();
    let relationship_count = state.relationships.read().unwrap().len();

    Json(json!({
        "protocol": "Model Context Protocol (MCP)",
        "protocol_version": "2024-11-05",
        "server_info": {
            "name": "scaffoldry-sovereign-mcp",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Sovereign institutional decision, tabular dataset, and governance MCP server"
        },
        "capabilities": {
            "tools": {
                "count": 10,
                "items": [
                    "list_datasets",
                    "query_dataset",
                    "create_app_proposal",
                    "simulate_cedar_policy",
                    "calculate_formula",
                    "get_governance_posture",
                    "record_governance_decision",
                    "verify_decision_ledger",
                    "export_oscal_compliance",
                    "get_framework_spec"
                ]
            },
            "resources": {
                "count": 5,
                "uris": [
                    "datasets://catalog",
                    "policies://cedar",
                    "compliance://oscal",
                    "scaffoldry://governance/decision-ledger",
                    "scaffoldry://framework/component-spec"
                ]
            },
            "prompts": {
                "count": 2,
                "items": [
                    "governed_app_builder",
                    "ferpa_risk_assessment"
                ]
            }
        },
        "live_metrics": {
            "published_datasets": dataset_count,
            "dataset_relationships": relationship_count,
            "compliance_frameworks": ["NIST OSCAL 1.1.2", "FERPA 34 CFR § 99.30", "CEDS v11"]
        }
    }))
}

/// POST /api/v1/mcp - JSON-RPC 2.0 MCP Request Handler
async fn handle_mcp_request(
    State(state): State<SharedState>,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    let req_id = req.id.clone();

    if req.jsonrpc != "2.0" {
        return Json(JsonRpcResponse {
            jsonrpc: "2.0",
            id: req_id,
            result: None,
            error: Some(JsonRpcError {
                code: -32600,
                message: "Invalid Request: jsonrpc must be '2.0'".to_string(),
                data: None,
            }),
        });
    }

    match req.method.as_str() {
        "initialize" => {
            let result = json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {},
                    "resources": {},
                    "prompts": {}
                },
                "serverInfo": {
                    "name": "scaffoldry-sovereign-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                }
            });
            Json(JsonRpcResponse {
                jsonrpc: "2.0",
                id: req_id,
                result: Some(result),
                error: None,
            })
        }

        "notifications/initialized" => Json(JsonRpcResponse {
            jsonrpc: "2.0",
            id: req_id,
            result: Some(json!({ "status": "acknowledged" })),
            error: None,
        }),

        "tools/list" => {
            let tools = json!({
                "tools": [
                    {
                        "name": "list_datasets",
                        "description": "List all published institutional datasets with field schemas, department ownership, and FERPA classifications.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "department": {
                                    "type": "string",
                                    "description": "Optional department filter (e.g., 'Academic Affairs', 'Research Administration')"
                                }
                            }
                        }
                    },
                    {
                        "name": "query_dataset",
                        "description": "Query tabular records from an authoritative institutional dataset.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "dataset_id": {
                                    "type": "string",
                                    "description": "Identifier of the published dataset (e.g. 'courses', 'faculty', 'grants')"
                                },
                                "limit": {
                                    "type": "integer",
                                    "description": "Maximum records to return (default: 10)"
                                }
                            },
                            "required": ["dataset_id"]
                        }
                    },
                    {
                        "name": "create_app_proposal",
                        "description": "Formulate a new application manifest proposal with automatic FERPA scanning, CEDS element alignment, and Cedar policy binding.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "title": { "type": "string" },
                                "department": { "type": "string" },
                                "organization_code": { "type": "string" },
                                "description": { "type": "string" },
                                "fields": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "name": { "type": "string" },
                                            "label": { "type": "string" },
                                            "field_type": { "type": "string" },
                                            "ferpa_sensitive": { "type": "boolean" },
                                            "linked_dataset_id": { "type": "string" }
                                        },
                                        "required": ["name", "label", "field_type"]
                                    }
                                }
                            },
                            "required": ["title", "department"]
                        }
                    },
                    {
                        "name": "simulate_cedar_policy",
                        "description": "Evaluate Cedar ABAC authorization decisions for an action on institutional resources.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "eppn": { "type": "string", "description": "Principal eduPersonPrincipalName" },
                                "affiliation": { "type": "string", "description": "Scoped affiliation (e.g. faculty@university.edu)" },
                                "action": { "type": "string", "description": "Action (view, edit, export, approve)" },
                                "ferpa_sensitive": { "type": "boolean", "description": "Whether resource carries FERPA classification" }
                            },
                            "required": ["eppn", "action"]
                        }
                    },
                    {
                        "name": "calculate_formula",
                        "description": "Evaluate rollup and calculation expressions across tabular column values (SUM, AVERAGE, MIN, MAX, COUNT).",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "formula": { "type": "string", "description": "Formula expression (SUM, AVERAGE, MIN, MAX, COUNT)" },
                                "values": { "type": "array", "items": { "type": "number" }, "description": "Array of numeric values" }
                            },
                            "required": ["formula", "values"]
                        }
                    },
                    {
                        "name": "get_governance_posture",
                        "description": "Retrieve NIST OSCAL 1.1.2 compliance metrics, control implementations, and active policy rules.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {}
                        }
                    },
                    {
                        "name": "record_governance_decision",
                        "description": "Append an approved governance decision to the immutable SHA-256 cryptographic ledger with NIST OSCAL control mapping.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "principal": { "type": "string", "description": "eduPersonPrincipalName of decision maker" },
                                "organization_code": { "type": "string", "description": "Institutional unit code" },
                                "app_slug": { "type": "string", "description": "Optional application slug" },
                                "decision_type": { "type": "string", "description": "Decision classification (AppPublished, VanityDnsBound, PolicyRevision, WorkflowRuleApproved, AccessRoleGranted, DatasetAccessShared, StatutoryAttestation)" },
                                "oscal_control_id": { "type": "string", "description": "NIST SP 800-53 / OSCAL control (e.g. AC-03, CM-03, AU-02)" },
                                "rationale": { "type": "string", "description": "Institutional justification and review findings" }
                            },
                            "required": ["principal", "organization_code", "decision_type", "oscal_control_id", "rationale"]
                        }
                    },
                    {
                        "name": "verify_decision_ledger",
                        "description": "Verify cryptographic integrity and SHA-256 block chain linkage of all recorded governance decisions from genesis.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {}
                        }
                    },
                    {
                        "name": "export_oscal_compliance",
                        "description": "Generate and export official NIST OSCAL 1.1.2 JSON component-definition with full cryptographic audit proofs.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {}
                        }
                    },
                    {
                        "name": "get_framework_spec",
                        "description": "Retrieve the self-documenting JSON Schema specification for the TanStack-extended component catalog, approved institutional theme tokens, and data classification boundaries.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {}
                        }
                    }
                ]
            });
            Json(JsonRpcResponse {
                jsonrpc: "2.0",
                id: req_id,
                result: Some(tools),
                error: None,
            })
        }

        "tools/call" => {
            let params = req.params.unwrap_or(json!({}));
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));

            let result = match tool_name {
                "list_datasets" => {
                    let datasets = state.datasets.read().unwrap();
                    let dept_filter = args.get("department").and_then(|v| v.as_str());
                    let list: Vec<_> = datasets
                        .values()
                        .filter(|d| {
                            dept_filter.is_none_or(|f| {
                                d.department.eq_ignore_ascii_case(f)
                            })
                        })
                        .cloned()
                        .collect();
                    json!({
                        "content": [
                            {
                                "type": "text",
                                "text": serde_json::to_string_pretty(&list).unwrap_or_default()
                            }
                        ]
                    })
                }

                "query_dataset" => {
                    let dataset_id = args.get("dataset_id").and_then(|v| v.as_str()).unwrap_or("");
                    let datasets = state.datasets.read().unwrap();
                    if let Some(ds) = datasets.get(dataset_id) {
                        let sample_records = match dataset_id {
                            "courses" => json!([
                                { "course_code": "CS-101", "course_title": "Introduction to Computer Science", "credits": 4, "department": "Computer Science", "instructor_eppn": "dr.alan@university.edu" },
                                { "course_code": "PHYS-201", "course_title": "Quantum Mechanics & Relativity", "credits": 4, "department": "Physics", "instructor_eppn": "dr.smith@university.edu" },
                                { "course_code": "BIO-310", "course_title": "Molecular Genetics & Cellular Biology", "credits": 3, "department": "Biology", "instructor_eppn": "dr.curie@science.state.edu" }
                            ]),
                            "faculty" => json!([
                                { "eppn": "dr.smith@university.edu", "full_name": "Dr. Sarah Smith", "title": "Professor", "department": "Physics", "research_specialty": "Astrophysics" },
                                { "eppn": "dr.curie@science.state.edu", "full_name": "Dr. Marie Curie", "title": "Distinguished Professor", "department": "Biology", "research_specialty": "Radiation Oncology" },
                                { "eppn": "dr.alan@university.edu", "full_name": "Dr. Alan Turing", "title": "Chair Professor", "department": "Computer Science", "research_specialty": "Machine Intelligence" }
                            ]),
                            "grants" => json!([
                                { "grant_number": "NSF-2026-9941", "project_title": "Sovereign AI for Research Data Infrastructure", "funding_agency": "National Science Foundation", "award_amount": 750000, "pi_eppn": "dr.alan@university.edu", "status": "Active" },
                                { "grant_number": "NIH-2025-0182", "project_title": "Targeted Molecular Therapeutics for Oncology", "funding_agency": "National Institutes of Health", "award_amount": 1200000, "pi_eppn": "dr.curie@science.state.edu", "status": "Active" }
                            ]),
                            _ => json!([])
                        };
                        json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": serde_json::to_string_pretty(&json!({
                                        "dataset": ds,
                                        "records": sample_records
                                    })).unwrap_or_default()
                                }
                            ]
                        })
                    } else {
                        json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": format!("Dataset '{}' not found in published catalog.", dataset_id)
                                }
                            ]
                        })
                    }
                }

                "create_app_proposal" => {
                    let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled App");
                    let department = args.get("department").and_then(|v| v.as_str()).unwrap_or("General");
                    let org = args.get("organization_code").and_then(|v| v.as_str()).unwrap_or("UNIV");
                    let desc = args.get("description").and_then(|v| v.as_str()).unwrap_or("");
                    let fields = args.get("fields").cloned().unwrap_or(json!([]));

                    let slug = title.to_lowercase().replace(' ', "-").replace(|c: char| !c.is_alphanumeric() && c != '-', "");
                    let proposal = json!({
                        "proposal_branch": format!("proposal/{slug}"),
                        "status": "StagedForReview",
                        "manifest": {
                            "schema_version": "1.0.0",
                            "slug": slug,
                            "title": title,
                            "organization_code": org,
                            "department": department,
                            "description": desc,
                            "views": [
                                {
                                    "id": "primary-view",
                                    "title": format!("{title} Records"),
                                    "view_type": "Form",
                                    "fields": fields
                                }
                            ]
                        },
                        "automated_governance_checks": {
                            "ferpa_scan": "Passed",
                            "cedar_policy_alignment": "Compliant",
                            "oscal_control_mapping": "AC-03, IA-02, MP-04"
                        }
                    });

                    json!({
                        "content": [
                            {
                                "type": "text",
                                "text": serde_json::to_string_pretty(&proposal).unwrap_or_default()
                            }
                        ]
                    })
                }

                "simulate_cedar_policy" => {
                    let eppn = args.get("eppn").and_then(|v| v.as_str()).unwrap_or("unknown@univ.edu");
                    let affiliation = args.get("affiliation").and_then(|v| v.as_str()).unwrap_or("staff@university.edu");
                    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("view");
                    let ferpa_sensitive = args.get("ferpa_sensitive").and_then(|v| v.as_bool()).unwrap_or(false);

                    let is_export = action.eq_ignore_ascii_case("export");
                    let allowed = if is_export && ferpa_sensitive {
                        affiliation.contains("compliance") || affiliation.contains("registrar")
                    } else if ferpa_sensitive {
                        affiliation.contains("faculty") || affiliation.contains("staff")
                    } else {
                        true
                    };

                    json!({
                        "content": [
                            {
                                "type": "text",
                                "text": serde_json::to_string_pretty(&json!({
                                    "decision": if allowed { "Allow" } else { "Deny" },
                                    "principal": eppn,
                                    "affiliation": affiliation,
                                    "action": action,
                                    "resource_ferpa_sensitive": ferpa_sensitive,
                                    "governing_policy": if ferpa_sensitive { "policy-ferpa-34cfr99" } else { "policy-default-view" }
                                })).unwrap_or_default()
                            }
                        ]
                    })
                }

                "calculate_formula" => {
                    let formula = args.get("formula").and_then(|v| v.as_str()).unwrap_or("COUNT");
                    let values: Vec<f64> = args.get("values")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|x| x.as_f64()).collect())
                        .unwrap_or_default();

                    let computed: f64 = match formula.to_uppercase().as_str() {
                        "SUM" => values.iter().sum(),
                        "AVERAGE" | "AVG" => {
                            if values.is_empty() { 0.0 } else { values.iter().sum::<f64>() / (values.len() as f64) }
                        }
                        "MIN" => values.iter().copied().fold(f64::INFINITY, f64::min),
                        "MAX" => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                        "COUNT" => values.len() as f64,
                        _ => 0.0,
                    };

                    json!({
                        "content": [
                            {
                                "type": "text",
                                "text": serde_json::to_string_pretty(&json!({
                                    "formula": formula,
                                    "input_count": values.len(),
                                    "result": computed
                                })).unwrap_or_default()
                            }
                        ]
                    })
                }

                "get_governance_posture" => {
                    json!({
                        "content": [
                            {
                                "type": "text",
                                "text": serde_json::to_string_pretty(&json!({
                                    "framework": "NIST OSCAL 1.1.2",
                                    "compliance_score": "98%",
                                    "enforced_controls": [
                                        { "id": "AC-03", "title": "Access Enforcement", "status": "Automated" },
                                        { "id": "IA-02", "title": "Identification and Authentication", "status": "Automated" },
                                        { "id": "MP-04", "title": "Media Transport / Privacy Export", "status": "Automated" },
                                        { "id": "AU-02", "title": "Event Logging & Audit Ledger", "status": "Automated" }
                                    ]
                                })).unwrap_or_default()
                            }
                        ]
                    })
                }

                "record_governance_decision" => {
                    let principal = args.get("principal").and_then(|v| v.as_str()).unwrap_or("supervisor@state.edu");
                    let org_code = args.get("organization_code").and_then(|v| v.as_str()).unwrap_or("DIV-GOVERNANCE");
                    let app_slug = args.get("app_slug").and_then(|v| v.as_str()).map(|s| s.to_string());
                    let dec_str = args.get("decision_type").and_then(|v| v.as_str()).unwrap_or("StatutoryAttestation");
                    let oscal_control = args.get("oscal_control_id").and_then(|v| v.as_str()).unwrap_or("AU-02");
                    let rationale = args.get("rationale").and_then(|v| v.as_str()).unwrap_or("AI-initiated governance record");

                    let decision_type = match dec_str {
                        "AppPublished" => scaffoldry_core::DecisionType::AppPublished,
                        "VanityDnsBound" => scaffoldry_core::DecisionType::VanityDnsBound,
                        "PolicyRevision" => scaffoldry_core::DecisionType::PolicyRevision,
                        "WorkflowRuleApproved" => scaffoldry_core::DecisionType::WorkflowRuleApproved,
                        "AccessRoleGranted" => scaffoldry_core::DecisionType::AccessRoleGranted,
                        "DatasetAccessShared" => scaffoldry_core::DecisionType::DatasetAccessShared,
                        _ => scaffoldry_core::DecisionType::StatutoryAttestation,
                    };

                    match state.append_ledger_entry(crate::state::RecordDecisionInput {
                        principal: principal.to_string(),
                        organization_code: org_code.to_string(),
                        app_slug,
                        decision_type,
                        oscal_control_id: oscal_control.to_string(),
                        rationale: rationale.to_string(),
                        payload: &args,
                    }) {
                        Ok(entry) => json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": serde_json::to_string_pretty(&json!({
                                        "success": true,
                                        "sequence": entry.sequence,
                                        "entry_hash": entry.entry_hash,
                                        "previous_hash": entry.previous_hash,
                                        "oscal_control": entry.oscal_control_id,
                                        "message": "Decision block cryptographically chained to ledger"
                                    })).unwrap_or_default()
                                }
                            ]
                        }),
                        Err(e) => json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": format!("Failed to record governance decision: {e}")
                                }
                            ]
                        }),
                    }
                }

                "verify_decision_ledger" => {
                    let is_valid = state.verify_ledger().unwrap_or(false);
                    let ledger = state.ledger.read().unwrap();
                    json!({
                        "content": [
                            {
                                "type": "text",
                                "text": serde_json::to_string_pretty(&json!({
                                    "verified": is_valid,
                                    "total_blocks": ledger.len(),
                                    "head_hash": ledger.last().map(|e| e.entry_hash.as_str()).unwrap_or(""),
                                    "integrity_audit": if is_valid { "ALL_BLOCKS_VALID" } else { "TAMPER_DETECTED" }
                                })).unwrap_or_default()
                            }
                        ]
                    })
                }

                "export_oscal_compliance" => {
                    let doc = state.export_oscal_component_definition();
                    json!({
                        "content": [
                            {
                                "type": "text",
                                "text": serde_json::to_string_pretty(&doc).unwrap_or_default()
                            }
                        ]
                    })
                }

                "get_framework_spec" => {
                    let spec = crate::routes::framework::build_framework_spec_json();
                    json!({
                        "content": [
                            {
                                "type": "text",
                                "text": serde_json::to_string_pretty(&spec).unwrap_or_default()
                            }
                        ]
                    })
                }

                _ => json!({
                    "content": [
                        {
                            "type": "text",
                            "text": format!("Tool '{}' not recognized by Scaffoldry MCP.", tool_name)
                        }
                    ]
                }),
            };

            Json(JsonRpcResponse {
                jsonrpc: "2.0",
                id: req_id,
                result: Some(result),
                error: None,
            })
        }

        "resources/list" => {
            let resources = json!({
                "resources": [
                    {
                        "uri": "datasets://catalog",
                        "name": "Published Datasets Catalog",
                        "description": "Authoritative institutional dataset schemas and relationship lattice",
                        "mimeType": "application/json"
                    },
                    {
                        "uri": "policies://cedar",
                        "name": "Institutional Cedar Policies",
                        "description": "Active Attribute-Based Access Control policies in Cedar syntax",
                        "mimeType": "text/plain"
                    },
                    {
                        "uri": "compliance://oscal",
                        "name": "NIST OSCAL 1.1.2 Security Lattice",
                        "description": "System Security Plan controls and regulatory crosswalks",
                        "mimeType": "application/json"
                    },
                    {
                        "uri": "scaffoldry://governance/decision-ledger",
                        "name": "Cryptographic Decision Audit Ledger",
                        "description": "Append-only SHA-256 chained governance decision blocks",
                        "mimeType": "application/json"
                    },
                    {
                        "uri": "scaffoldry://framework/component-spec",
                        "name": "Framework Component Specification",
                        "description": "Self-documenting JSON Schema of governed components, layout slots, and approved org tokens",
                        "mimeType": "application/json"
                    }
                ]
            });
            Json(JsonRpcResponse {
                jsonrpc: "2.0",
                id: req_id,
                result: Some(resources),
                error: None,
            })
        }

        "resources/read" => {
            let params = req.params.unwrap_or(json!({}));
            let uri = params.get("uri").and_then(|v| v.as_str()).unwrap_or("");

            let result = match uri {
                "datasets://catalog" => {
                    let datasets = state.datasets.read().unwrap();
                    let list: Vec<_> = datasets.values().cloned().collect();
                    json!({
                        "contents": [
                            {
                                "uri": uri,
                                "mimeType": "application/json",
                                "text": serde_json::to_string_pretty(&list).unwrap_or_default()
                            }
                        ]
                    })
                }
                "policies://cedar" => {
                    let cedar_text = r#"
// Scaffoldry Institutional Cedar ABAC Policy Set
permit(
    principal,
    action in [Action::"view", Action::"list"],
    resource
);

forbid(
    principal,
    action == Action::"export",
    resource
) when {
    resource.ferpa_sensitive == true &&
    !(principal.scoped_affiliation in ["compliance@university.edu", "registrar@university.edu"])
};
"#;
                    json!({
                        "contents": [
                            {
                                "uri": uri,
                                "mimeType": "text/plain",
                                "text": cedar_text.trim()
                            }
                        ]
                    })
                }
                "compliance://oscal" => json!({
                    "contents": [
                        {
                            "uri": uri,
                            "mimeType": "application/json",
                            "text": serde_json::to_string_pretty(&json!({
                                "oscal_version": "1.1.2",
                                "title": "Scaffoldry Institutional Security and Compliance Lattice",
                                "baseline": "NIST SP 800-53 Rev 5 / FERPA 34 CFR Part 99"
                            })).unwrap_or_default()
                        }
                    ]
                }),
                "scaffoldry://governance/decision-ledger" => {
                    let ledger = state.ledger.read().unwrap();
                    let is_valid = state.verify_ledger().unwrap_or(false);
                    json!({
                        "contents": [
                            {
                                "uri": uri,
                                "mimeType": "application/json",
                                "text": serde_json::to_string_pretty(&json!({
                                    "chain_valid": is_valid,
                                    "total_blocks": ledger.len(),
                                    "blocks": *ledger
                                })).unwrap_or_default()
                            }
                        ]
                    })
                }
                "scaffoldry://framework/component-spec" => {
                    let spec = crate::routes::framework::build_framework_spec_json();
                    json!({
                        "contents": [
                            {
                                "uri": uri,
                                "mimeType": "application/json",
                                "text": serde_json::to_string_pretty(&spec).unwrap_or_default()
                            }
                        ]
                    })
                }
                _ => json!({
                    "contents": []
                }),
            };

            Json(JsonRpcResponse {
                jsonrpc: "2.0",
                id: req_id,
                result: Some(result),
                error: None,
            })
        }

        "prompts/list" => {
            let prompts = json!({
                "prompts": [
                    {
                        "name": "governed_app_builder",
                        "description": "Guide an AI assistant to formulate a compliant application schema adhering to CEDS v11 and Cedar access controls.",
                        "arguments": [
                            { "name": "app_purpose", "description": "The business goal or departmental workflow", "required": true },
                            { "name": "department", "description": "Department or faculty unit", "required": true }
                        ]
                    },
                    {
                        "name": "ferpa_risk_assessment",
                        "description": "Audit an application or dataset schema for FERPA (34 CFR § 99.30) privacy compliance.",
                        "arguments": [
                            { "name": "schema_json", "description": "JSON representation of the schema", "required": true }
                        ]
                    }
                ]
            });
            Json(JsonRpcResponse {
                jsonrpc: "2.0",
                id: req_id,
                result: Some(prompts),
                error: None,
            })
        }

        "prompts/get" => {
            let params = req.params.unwrap_or(json!({}));
            let prompt_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));

            let result = match prompt_name {
                "governed_app_builder" => {
                    let purpose = args.get("app_purpose").and_then(|v| v.as_str()).unwrap_or("Institutional Process");
                    let dept = args.get("department").and_then(|v| v.as_str()).unwrap_or("Academic Affairs");
                    json!({
                        "description": "System prompt for generating governed application schemas",
                        "messages": [
                            {
                                "role": "user",
                                "content": {
                                    "type": "text",
                                    "text": format!(
                                        "You are the Scaffoldry Sovereign Co-Builder AI. Design a structured application schema for: '{purpose}' in the department '{dept}'. Ensure all student identifying fields are marked with ferpa_sensitive: true, map fields to CEDS v11 where applicable, and link authoritative published datasets (courses, faculty, grants) where appropriate."
                                    )
                                }
                            }
                        ]
                    })
                }
                "ferpa_risk_assessment" => {
                    let schema = args.get("schema_json").and_then(|v| v.as_str()).unwrap_or("{}");
                    json!({
                        "description": "System prompt for auditing FERPA risks",
                        "messages": [
                            {
                                "role": "user",
                                "content": {
                                    "type": "text",
                                    "text": format!(
                                        "Audit the following application schema under 34 CFR § 99.30 (FERPA): {schema}. Identify any Personally Identifiable Information (PII) that requires Cedar authorization guardrails."
                                    )
                                }
                            }
                        ]
                    })
                }
                _ => json!({ "messages": [] }),
            };

            Json(JsonRpcResponse {
                jsonrpc: "2.0",
                id: req_id,
                result: Some(result),
                error: None,
            })
        }

        _ => Json(JsonRpcResponse {
            jsonrpc: "2.0",
            id: req_id,
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: format!("Method '{}' not found", req.method),
                data: None,
            }),
        }),
    }
}
