//! Native Model Context Protocol (MCP) Server Endpoint
//!
//! Exposes JSON-RPC 2.0 compliant Model Context Protocol tools, resources,
//! and prompt templates for sovereign, AI-native application co-building,
//! workspace operations, and governance inspection.

use crate::service::tools;
use crate::state::{AuthUser, SharedState};
use axum::{
    extract::{Extension, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/mcp", post(handle_mcp_request))
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

/// The protocol versions this server speaks, newest first.
const SUPPORTED_VERSIONS: [&str; 2] = ["2025-06-18", "2024-11-05"];

fn rpc(response: JsonRpcResponse) -> Response {
    Json(response).into_response()
}

fn tool_error(message: impl Into<String>) -> Value {
    json!({ "isError": true, "content": [{ "type": "text", "text": message.into() }] })
}

/// A tool error that carries the error's `code` for an agent to branch on, and the same text.
fn coded_tool_error(code: &str, message: &str) -> Value {
    json!({
        "isError": true,
        "content": [{ "type": "text", "text": message }],
        "structuredContent": { "code": code, "message": message }
    })
}

/// The same for a `ServiceError`, with the `reason` and `remedy` of a denial when it has them.
fn service_tool_error(err: &crate::service::ServiceError) -> Value {
    let mut value = coded_tool_error(err.code(), err.message());
    let body = err.body();
    for key in ["reason", "remedy", "fields"] {
        if let Some(v) = body.get(key) {
            value["structuredContent"][key] = v.clone();
        }
    }
    value
}

/// POST /api/v1/mcp - JSON-RPC 2.0 MCP Request Handler
async fn handle_mcp_request(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    // The body is read here, not by an extractor, so a bad one gets a JSON-RPC error and not a plain-text 400.
    // Not JSON at all is -32700. JSON that is not a request is -32600. Neither names an id.
    let req: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => {
            let is_json = serde_json::from_slice::<Value>(&body).is_ok();
            let (code, message) = if is_json {
                (-32600, "Invalid Request: the body is not a JSON-RPC request")
            } else {
                (-32700, "Parse error: the body is not valid JSON")
            };
            return rpc(JsonRpcResponse {
                jsonrpc: "2.0",
                id: None,
                result: None,
                error: Some(JsonRpcError { code, message: message.to_string(), data: None }),
            });
        }
    };
    let req_id = req.id.clone();

    if req.jsonrpc != "2.0" {
        return rpc(JsonRpcResponse {
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

    let caller = match user {
        Some(Extension(u)) => u,
        None => match crate::guard::session_user(&state, &headers) {
            Some(u) => u,
            None => {
                return rpc(JsonRpcResponse {
                    jsonrpc: "2.0",
                    id: req_id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32001,
                        message: "Unauthorized: valid session required".to_string(),
                        data: None,
                    }),
                });
            }
        },
    };

    // A notification carries no id. The server does not answer it.
    if req.id.is_none() {
        return StatusCode::ACCEPTED.into_response();
    }

    // A view-as session reaches the tools and nothing else here.
    if crate::guard::view_as_token(&state, &headers).is_some() && !["initialize", "ping", "tools/list", "tools/call"].contains(&req.method.as_str()) {
        let err = crate::service::ServiceError::forbidden("This view reaches one app only. Exit it to open anything else.");
        return rpc(JsonRpcResponse { jsonrpc: "2.0", id: req_id, result: None, error: Some(JsonRpcError { code: -32001, message: err.message().to_string(), data: None }) });
    }

    match req.method.as_str() {
        "initialize" => {
            let asked = req
                .params
                .as_ref()
                .and_then(|p| p.get("protocolVersion"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let version = SUPPORTED_VERSIONS.iter().find(|v| **v == asked).unwrap_or(&SUPPORTED_VERSIONS[0]);
            let result = json!({
                "protocolVersion": version,
                "capabilities": {
                    "tools": {},
                    "resources": {},
                    "prompts": {}
                },
                "serverInfo": {
                    "name": "scaffoldry-sovereign-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "instructions": crate::service::guidance::instructions(&caller)
            });
            rpc(JsonRpcResponse {
                jsonrpc: "2.0",
                id: req_id,
                result: Some(result),
                error: None,
            })
        }

        // A ping is answered with an empty result.
        "ping" => rpc(JsonRpcResponse { jsonrpc: "2.0", id: req_id, result: Some(json!({})), error: None }),

        "notifications/initialized" => rpc(JsonRpcResponse {
            jsonrpc: "2.0",
            id: req_id,
            result: Some(json!({ "status": "acknowledged" })),
            error: None,
        }),

        "tools/list" => {
            let listed: Vec<Value> = tools::TOOLS
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": (t.input_schema)(),
                        "annotations": { "readOnlyHint": t.read_only }
                    })
                })
                .collect();
            rpc(JsonRpcResponse {
                jsonrpc: "2.0",
                id: req_id,
                result: Some(json!({ "tools": listed })),
                error: None,
            })
        }

        "tools/call" => {
            let params = req.params.unwrap_or(json!({}));
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));

            if let Some(row) = crate::guard::view_as_token(&state, &headers) {
                if let Err(err) = crate::guard::view_as_tool_gate(&row, &tool_name, &args) {
                    return rpc(JsonRpcResponse { jsonrpc: "2.0", id: req_id, result: Some(service_tool_error(&err)), error: None });
                }
            }

            // A tool does database work, so it runs off the async threads.
            let origin = match crate::guard::origin(&state, &headers) {
                Ok(o) => o,
                Err(err) => {
                    return rpc(JsonRpcResponse { jsonrpc: "2.0", id: req_id, result: Some(tool_error(err.message())), error: None });
                }
            };
            let worker_state = state.clone();
            let outcome =
                tokio::task::spawn_blocking(move || tools::call(&caller, &origin, &tool_name, args, &worker_state)).await;

            let result = match outcome {
                Ok(Ok(value)) => {
                    // The spec says structuredContent is an object. A client validates it and discards a bare list.
                    // A list goes under `items`, a lone value under `value`. The text keeps the plain result.
                    let structured = match &value {
                        Value::Object(_) => value.clone(),
                        Value::Array(_) => json!({ "items": value }),
                        other => json!({ "value": other }),
                    };
                    json!({
                        "structuredContent": structured,
                        "content": [{ "type": "text", "text": serde_json::to_string(&value).unwrap_or_default() }]
                    })
                }
                Ok(Err(err)) => service_tool_error(&err),
                Err(_) => tool_error("The tool stopped before it answered"),
            };
            rpc(JsonRpcResponse {
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
                    },
                    {
                        "uri": "scaffoldry://schema/app-manifest",
                        "name": "App Manifest Schema",
                        "description": "JSON Schema 2020-12 for the app manifest an agent builds against",
                        "mimeType": "application/json"
                    },
                    {
                        "uri": "ui://workspaces/{id}/settings",
                        "name": "Workspace Security & Access Controls",
                        "description": "Interactive sovereign MCP App UI for workspace sharing and OSCAL AC-02/AC-03 settings",
                        "mimeType": "text/html;profile=mcp-app"
                    },
                    {
                        "uri": "ui://governance/decision-ledger",
                        "name": "Cryptographic Decision Ledger Inspection",
                        "description": "Interactive sovereign MCP App UI for SHA-256 block chain inspection and OSCAL audit proofs",
                        "mimeType": "text/html;profile=mcp-app"
                    }
                ]
            });
            rpc(JsonRpcResponse {
                jsonrpc: "2.0",
                id: req_id,
                result: Some(resources),
                error: None,
            })
        }

        "resources/read" => {
            let params = req.params.unwrap_or(json!({}));
            let uri = params.get("uri").and_then(|v| v.as_str()).unwrap_or("");

            let result = if uri.starts_with("ui://workspaces/") && uri.ends_with("/settings") {
                let ws_id = uri
                    .strip_prefix("ui://workspaces/")
                    .and_then(|s| s.strip_suffix("/settings"))
                    .unwrap_or("");
                match crate::service::governance::render_ui_workspace_settings(&caller, ws_id, &state) {
                    Ok(html) => json!({
                        "contents": [
                            {
                                "uri": uri,
                                "mimeType": "text/html;profile=mcp-app",
                                "text": html
                            }
                        ]
                    }),
                    Err(err) => json!({
                        "isError": true,
                        "contents": [
                            {
                                "uri": uri,
                                "mimeType": "text/plain",
                                "text": err.message()
                            }
                        ]
                    }),
                }
            } else if uri == "ui://governance/decision-ledger" {
                match crate::service::governance::render_ui_decision_ledger(&caller, &state) {
                    Ok(html) => json!({
                        "contents": [
                            {
                                "uri": uri,
                                "mimeType": "text/html;profile=mcp-app",
                                "text": html
                            }
                        ]
                    }),
                    Err(err) => json!({
                        "isError": true,
                        "contents": [
                            {
                                "uri": uri,
                                "mimeType": "text/plain",
                                "text": err.message()
                            }
                        ]
                    }),
                }
            } else {
                match uri {
                    "datasets://catalog" => {
                        let datasets = state.datasets.read().unwrap_or_else(|p| p.into_inner());
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
                    "policies://cedar" => json!({
                        "contents": [
                            {
                                "uri": uri,
                                "mimeType": "text/plain",
                                "text": state.policy_engine.policy_source()
                            }
                        ]
                    }),
                    "scaffoldry://governance/decision-ledger" => {
                        let ledger = state.ledger.read().unwrap_or_else(|p| p.into_inner());
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
                    "scaffoldry://schema/app-manifest" => json!({
                        "contents": [
                            {
                                "uri": uri,
                                "mimeType": "application/json",
                                "text": include_str!("../../../../governance/schema/app-manifest.schema.json")
                            }
                        ]
                    }),
                    _ => json!({
                        "contents": []
                    }),
                }
            };

            rpc(JsonRpcResponse {
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
            rpc(JsonRpcResponse {
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

            rpc(JsonRpcResponse {
                jsonrpc: "2.0",
                id: req_id,
                result: Some(result),
                error: None,
            })
        }

        _ => rpc(JsonRpcResponse {
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
