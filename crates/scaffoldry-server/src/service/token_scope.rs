//! What a limited token may do (integrations phase 2).
//!
//! A token's scope only takes rights away. These checks run before a request or a tool call reaches its handler, and the
//! handler then checks the person's own rights, so the effective right is the intersection of the two.

use crate::service::ServiceError;
use crate::state::SharedState;
use scaffoldry_core::token_scope::{data_tool, TableFrom};
use scaffoldry_core::{AgentLevel, TokenOp, TokenScope};
use scaffoldry_engine::HostRouter;
use serde_json::Value;

fn refuse(scope: &TokenScope, what: &str) -> ServiceError {
    ServiceError::TokenScope(format!("{what} {}", scope.sentence()))
}

/// The table a record belongs to, for a token that names only the record.
fn table_of_record(app: &str, record_id: &str, state: &SharedState) -> Option<String> {
    let record = state.find_record(app, record_id).ok().flatten()?;
    record.data.get("_table_id").and_then(Value::as_str).map(str::to_string)
}

fn check_table(scope: &TokenScope, app: &str, table: &str, op: TokenOp) -> Result<(), ServiceError> {
    if !scope.allows_app(app) {
        return Err(refuse(scope, &format!("This token may not use the app '{app}'.")));
    }
    if !scope.allows(app, table, op) {
        return Err(refuse(scope, &format!("This token may not {} the table '{table}'.", op.word())));
    }
    Ok(())
}

/// The level a session needs to call a tool, or None for a tool no session may call: a person does that.
fn level_needed(name: &str) -> Option<AgentLevel> {
    match name {
        "begin_task" => Some(AgentLevel::Read),
        "propose_app_change" | "preview_proposal" => Some(AgentLevel::Propose),
        "create_record" | "update_record" | "delete_record" => Some(AgentLevel::Write),
        _ => crate::service::tools::TOOLS.iter().find(|t| t.name == name).filter(|t| t.read_only).map(|_| AgentLevel::Read),
    }
}

fn rank(level: AgentLevel) -> u8 {
    match level {
        AgentLevel::Read => 0,
        AgentLevel::Propose => 1,
        AgentLevel::Write => 2,
    }
}

/// Whether a session at `level` may call the tool.
fn session_allows_tool(level: AgentLevel, name: &str) -> bool {
    level_needed(name).is_some_and(|needed| rank(needed) <= rank(level))
}

/// Whether `tools/list` shows the tool to a token with this scope.
pub fn lists_tool(scope: &TokenScope, name: &str) -> bool {
    match scope.level {
        Some(level) => session_allows_tool(level, name),
        None => scope.allows_tool(name),
    }
}

/// Why a session may not call the tool, in words that tell the AI tool what the person can do.
fn session_refusal(scope: &TokenScope, level: AgentLevel, name: &str) -> ServiceError {
    match level_needed(name) {
        Some(needed) => refuse(
            scope,
            &format!(
                "This session was given '{}'. The tool '{name}' needs '{}'. Ask the person to reconnect this tool and choose that, if they want it.",
                level.sentence(),
                needed.sentence()
            ),
        ),
        None => refuse(scope, &format!("A tool may not call '{name}'. A person does that.")),
    }
}

/// A session may use the record routes it can read, and write them at the Write level. It reads whatever its person reads.
fn session_request(scope: &TokenScope, level: AgentLevel, method: &axum::http::Method, path: &str) -> Result<(), ServiceError> {
    use axum::http::Method;
    let read = method == Method::GET || method == Method::HEAD;
    let segments: Vec<&str> = path.trim_start_matches("/api/v1/").trim_matches('/').split('/').collect();
    let records = matches!(segments.as_slice(), ["apps", _, "records"] | ["apps", _, "tables", _, "records"] | ["apps", _, "tables", _, "records", _] | ["apps", _, "records", _]);
    if read || segments.as_slice() == ["auth", "logout"] || (records && level == AgentLevel::Write) {
        Ok(())
    } else {
        Err(refuse(scope, &format!("This session was given '{}'. Ask the person to reconnect this tool and choose 'Read, propose, and write records' to change records.", level.sentence())))
    }
}

/// Checks a tool call against the scope, and lowers a list limit to the cap. `args` is changed in place.
pub fn check_tool(scope: &TokenScope, name: &str, args: &mut Value, state: &SharedState) -> Result<(), ServiceError> {
    if let Some(level) = scope.level {
        return if session_allows_tool(level, name) { Ok(()) } else { Err(session_refusal(scope, level, name)) };
    }
    let Some((op, from)) = data_tool(name).filter(|_| scope.allows_tool(name)) else {
        return Err(refuse(scope, &format!("This token may not call the tool '{name}'.")));
    };
    let app = args.get("app_slug").and_then(Value::as_str).unwrap_or_default().to_string();
    if !scope.allows_app(&app) {
        return Err(refuse(scope, &format!("This token may not use the app '{app}'.")));
    }
    let table = match from {
        TableFrom::App => None,
        TableFrom::Argument => Some(args.get("table_id").and_then(Value::as_str).map(str::to_string)),
        TableFrom::Data => Some(args.get("data").and_then(|d| d.get("_table_id")).and_then(Value::as_str).map(str::to_string)),
        TableFrom::Record => Some(args.get("record_id").and_then(Value::as_str).and_then(|id| table_of_record(&app, id, state))),
    };
    if let Some(table) = table {
        let table = table.ok_or_else(|| refuse(scope, "This token must name the table it works on."))?;
        check_table(scope, &app, &table, op)?;
    }
    if let (Some(cap), "list_records") = (scope.max_rows_per_call, name) {
        let asked = args.get("limit").and_then(Value::as_u64).unwrap_or(u64::MAX);
        args["limit"] = Value::from(asked.min(u64::from(cap)));
    }
    Ok(())
}

/// Checks a REST request against the scope. Only the record routes are open to a limited token.
pub fn check_request(scope: &TokenScope, method: &axum::http::Method, path: &str, state: &SharedState) -> Result<(), ServiceError> {
    use axum::http::Method;
    if let Some(level) = scope.level {
        return session_request(scope, level, method, path);
    }
    let segments: Vec<&str> = path.trim_start_matches("/api/v1/").trim_matches('/').split('/').collect();
    let read = method == Method::GET || method == Method::HEAD;
    match segments.as_slice() {
        ["auth", "me"] if read => Ok(()),
        ["auth", "logout"] if method == Method::POST => Ok(()),
        ["apps", app, "openapi.json"] if read => scope.allows_app(app).then_some(()).ok_or_else(|| refuse(scope, &format!("This token may not use the app '{app}'."))),
        ["apps", app, "client.ts"] if read => scope.allows_app(app).then_some(()).ok_or_else(|| refuse(scope, &format!("This token may not use the app '{app}'."))),
        ["apps", app, "schema"] if read => scope.allows_app(app).then_some(()).ok_or_else(|| refuse(scope, &format!("This token may not use the app '{app}'."))),
        ["apps", app, "tables", table, "aggregate"] if read => check_table(scope, app, table, TokenOp::Read),
        ["apps", app, "tables", table, "records"] if read => check_table(scope, app, table, TokenOp::Read),
        ["apps", app, "tables", table, "records"] if method == Method::POST => check_table(scope, app, table, TokenOp::Create),
        ["apps", app, "tables", table, "records", _] => check_table(scope, app, table, op_of(method)?),
        ["apps", app, "records", id] => {
            let table = table_of_record(app, id, state).ok_or_else(|| refuse(scope, "This token must name the table it works on."))?;
            check_table(scope, app, &table, op_of(method)?)
        }
        _ => Err(refuse(scope, "This token may not use that route.")),
    }
}

fn op_of(method: &axum::http::Method) -> Result<TokenOp, ServiceError> {
    use axum::http::Method;
    match *method {
        Method::GET | Method::HEAD => Ok(TokenOp::Read),
        Method::PATCH | Method::PUT => Ok(TokenOp::Update),
        Method::DELETE => Ok(TokenOp::Delete),
        _ => Err(ServiceError::TokenScope("This token may not use that route.".to_string())),
    }
}

/// The query string of a list request with `limit` lowered to the cap. None when the request is not a list or has no cap.
pub fn capped_query(scope: &TokenScope, method: &axum::http::Method, path: &str, query: Option<&str>) -> Option<String> {
    let cap = scope.max_rows_per_call?;
    if method != axum::http::Method::GET || !path.ends_with("/records") {
        return None;
    }
    let mut pairs: Vec<String> = query.unwrap_or_default().split('&').filter(|p| !p.is_empty()).map(str::to_string).collect();
    let mut found = false;
    for pair in pairs.iter_mut() {
        if let Some(value) = pair.strip_prefix("limit=") {
            let asked: u64 = value.parse().unwrap_or(u64::MAX);
            *pair = format!("limit={}", asked.min(u64::from(cap)));
            found = true;
        }
    }
    if !found {
        pairs.push(format!("limit={cap}"));
    }
    Some(pairs.join("&"))
}

/// Whether the app is one the engine holds. A scope that names an app that does not exist still limits the token.
pub fn app_exists(app: &str, state: &SharedState) -> bool {
    state.engine.read().map(|e| e.resolve_by_slug(app).is_some()).unwrap_or(false)
}
