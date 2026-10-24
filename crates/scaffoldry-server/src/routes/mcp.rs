//! Native Model Context Protocol (MCP) Server Endpoint
//!
//! Exposes JSON-RPC 2.0 compliant Model Context Protocol tools, resources,
//! and prompt templates for sovereign, AI-native application co-building,
//! workspace operations, and governance inspection.

use crate::service::tools;
use crate::state::{AuthUser, SharedState};
use axum::{
    extract::{DefaultBodyLimit, Extension, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// A proposal may carry a 2 MB bundle. The JSON that holds it can be several times larger than the source it escapes.
const MAX_MCP_BODY: usize = 8 * 1024 * 1024;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/mcp", post(handle_mcp_request).layer(DefaultBodyLimit::max(MAX_MCP_BODY)))
        .route("/apps/{slug}/mcp", post(handle_mcp_request).layer(DefaultBodyLimit::max(MAX_MCP_BODY)))
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
fn coded_tool_error(code: &str, message: &str, hint: &str) -> Value {
    json!({
        "isError": true,
        "content": [{ "type": "text", "text": message }, { "type": "text", "text": hint }],
        "structuredContent": { "code": code, "message": message, "hint": hint }
    })
}

/// The same for a `ServiceError`, with the `reason` and `remedy` of a denial when it has them.
fn service_tool_error(err: &crate::service::ServiceError) -> Value {
    let mut value = coded_tool_error(err.code(), err.message(), &crate::service::hints::hint_for(err));
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
    let held = crate::guard::token_row(&state, &headers);
    let from_client = held.as_ref().is_some_and(|r| r.kind == "client");
    let scope = held.and_then(|r| r.scope);
    // An AI tool's session reads resources and prompts as its person does. Other limited tokens do not.
    let session = scope.as_ref().is_some_and(|s| s.level.is_some());
    if !session && (scope.is_some() || crate::guard::view_as_token(&state, &headers).is_some()) && !["initialize", "ping", "tools/list", "tools/call"].contains(&req.method.as_str()) {
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
                // A limited token sees the tools its scope allows and no others.
                .filter(|t| scope.as_ref().is_none_or(|s| crate::service::token_scope::lists_tool(s, t.name)))
                .map(|t| {
                    // The schema is closed (`validate_args` refuses an unknown argument) and carries one valid call.
                    let mut schema = (t.input_schema)();
                    schema["additionalProperties"] = json!(false);
                    schema["examples"] = json!([crate::service::tool_meta::example(t.name)]);
                    let annotations = if t.read_only {
                        json!({ "readOnlyHint": true })
                    } else {
                        let (destructive, idempotent) = crate::service::tool_meta::hints(t.name);
                        json!({ "readOnlyHint": false, "destructiveHint": destructive, "idempotentHint": idempotent })
                    };
                    let mut entry = json!({ "name": t.name, "description": t.description, "inputSchema": schema, "annotations": annotations });
                    // MCP Apps: a tool links to its screen on its definition. The flat `ui/resourceUri` key is deprecated.
                    if let Some(screen) = crate::service::mcp_ui::tool_screen(t.name) {
                        entry["_meta"] = json!({ "ui": { "resourceUri": screen } });
                    }
                    entry
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
            let mut args = params.get("arguments").cloned().unwrap_or(json!({}));

            // A limited token calls the data tools its scope names. The person's own rights are checked by the tool.
            if let Some(scope) = &scope {
                if let Err(err) = crate::service::token_scope::check_tool(scope, &tool_name, &mut args, &state) {
                    let err = crate::guard::as_client_refusal(err, from_client);
                    return rpc(JsonRpcResponse { jsonrpc: "2.0", id: req_id, result: Some(service_tool_error(&err)), error: None });
                }
            }

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
            let mut resources = json!({
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
                        "uri": crate::service::prompts::GUIDE_URI,
                        "name": "Guide: building apps",
                        "description": "How to build an app here: describe, propose, wait for approval, field types, selects and links by id, paging, versions, jobs, and what an agent must never do",
                        "mimeType": "text/markdown"
                    },
                    {
                        "uri": crate::service::prompts::OUTSIDE_GUIDE_URI,
                        "name": "Guide: building outside Scaffoldry",
                        "description": "How an app hosted elsewhere gets institutional data: register a client, sign in with PKCE, use the generated module, keep calls small",
                        "mimeType": "text/markdown"
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
            if let Some(list) = resources["resources"].as_array_mut() {
                list.extend(crate::service::mcp_ui::listed(&caller, &state));
            }
            rpc(JsonRpcResponse {
                jsonrpc: "2.0",
                id: req_id,
                result: Some(resources),
                error: None,
            })
        }

        "resources/templates/list" => rpc(JsonRpcResponse {
            jsonrpc: "2.0",
            id: req_id,
            result: Some(json!({
                "resourceTemplates": [{
                    "uriTemplate": "scaffoldry://apps/{slug}/openapi",
                    "name": "App API description",
                    "description": "The OpenAPI 3.1 description of one app, for you. It lists the tables you can read and the fields you can see",
                    "mimeType": "application/json"
                }]
            })),
            error: None,
        }),

        "resources/read" => {
            let params = req.params.unwrap_or(json!({}));
            let uri = params.get("uri").and_then(|v| v.as_str()).unwrap_or("");

            // The screens of mcp-apps phase 8. A screen the caller may not read is not found, as one that does not exist.
            if let Some(found) = crate::service::mcp_ui::read(&caller, uri, &state) {
                return match found {
                    Some(html) => rpc(JsonRpcResponse {
                        jsonrpc: "2.0",
                        id: req_id,
                        result: Some(json!({ "contents": [{ "uri": uri, "mimeType": crate::service::mcp_ui::MIME, "text": html }] })),
                        error: None,
                    }),
                    None => rpc(JsonRpcResponse {
                        jsonrpc: "2.0",
                        id: req_id,
                        result: None,
                        error: Some(JsonRpcError { code: -32002, message: "Resource not found".to_string(), data: None }),
                    }),
                };
            }

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
            } else if let Some(slug) = crate::service::openapi::slug_of_resource(uri) {
                // The shape of a record, for this caller, before an agent writes one.
                match crate::service::openapi::describe(&caller, slug, &state) {
                    Ok(document) => json!({
                        "contents": [{ "uri": uri, "mimeType": "application/json", "text": serde_json::to_string(&document).unwrap_or_default() }]
                    }),
                    Err(err) => json!({
                        "isError": true,
                        "contents": [{ "uri": uri, "mimeType": "text/plain", "text": err.message() }]
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
                        // The newest page, not the chain: `total_blocks` says how long the chain is.
                        let total = state.repository.ledger_summary().map(|(n, _)| n).unwrap_or(0);
                        let blocks = state.repository.ledger_page(None, crate::repository::LEDGER_BATCH).unwrap_or_default();
                        let is_valid = state.verify_ledger().unwrap_or(false);
                        json!({
                            "contents": [
                                {
                                    "uri": uri,
                                    "mimeType": "application/json",
                                    "text": serde_json::to_string_pretty(&json!({
                                        "chain_valid": is_valid,
                                        "total_blocks": total,
                                        "blocks": blocks
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
                    crate::service::prompts::GUIDE_URI => json!({
                        "contents": [
                            {
                                "uri": uri,
                                "mimeType": "text/markdown",
                                "text": crate::service::prompts::GUIDE
                            }
                        ]
                    }),
                    crate::service::prompts::OUTSIDE_GUIDE_URI => json!({
                        "contents": [
                            {
                                "uri": uri,
                                "mimeType": "text/markdown",
                                "text": crate::service::prompts::OUTSIDE_GUIDE
                            }
                        ]
                    }),
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
            let mut all = vec![
                json!({
                    "name": "governed_app_builder",
                    "description": "Guide an AI assistant to formulate a compliant application schema adhering to CEDS v11 and Cedar access controls.",
                    "arguments": [
                        { "name": "app_purpose", "description": "The business goal or departmental workflow", "required": true },
                        { "name": "department", "description": "Department or faculty unit", "required": true }
                    ]
                }),
                json!({
                    "name": "ferpa_risk_assessment",
                    "description": "Audit an application or dataset schema for FERPA (34 CFR § 99.30) privacy compliance.",
                    "arguments": [
                        { "name": "schema_json", "description": "JSON representation of the schema", "required": true }
                    ]
                }),
            ];
            all.extend(crate::service::prompts::listed());
            let prompts = json!({ "prompts": all });
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
                other => crate::service::prompts::render(other, &args).unwrap_or_else(|| json!({ "messages": [] })),
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
