//! A token that is narrower than its owner (integrations phase 2).
//!
//! A scope names apps, the operations allowed on each table, and optionally the tools and a cap on rows per call.
//! It only takes rights away. What a token may do is this scope intersected with what its owner may do.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenOp {
    Read,
    Create,
    Update,
    Delete,
}

impl TokenOp {
    /// The word a sentence uses for the operation.
    pub fn word(self) -> &'static str {
        match self {
            TokenOp::Read => "read",
            TokenOp::Create => "add",
            TokenOp::Update => "change",
            TokenOp::Delete => "delete",
        }
    }
}

/// How a data tool names the table it works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableFrom {
    /// The argument `table_id`.
    Argument,
    /// The `_table_id` inside the argument `data`.
    Data,
    /// The table of the record named by `record_id`.
    Record,
    /// The tool reads the app's definition and no rows.
    App,
}

/// The only tools a scoped token may call, with the operation each is. Every other tool is an admin tool or works
/// beyond the rows of one table, and is never allowed to a scoped token.
pub const DATA_TOOLS: &[(&str, TokenOp, TableFrom)] = &[
    ("describe_app", TokenOp::Read, TableFrom::App),
    ("list_records", TokenOp::Read, TableFrom::Argument),
    ("aggregate_records", TokenOp::Read, TableFrom::Argument),
    ("get_record", TokenOp::Read, TableFrom::Record),
    ("create_record", TokenOp::Create, TableFrom::Data),
    ("update_record", TokenOp::Update, TableFrom::Record),
    ("delete_record", TokenOp::Delete, TableFrom::Record),
];

pub fn data_tool(name: &str) -> Option<(TokenOp, TableFrom)> {
    DATA_TOOLS.iter().find(|(n, _, _)| *n == name).map(|(_, op, from)| (*op, *from))
}

const MAX_ROWS_LIMIT: u32 = 10_000;

/// What a person lets an AI tool do for a session (external apps phase 8). The tool acts within the person's own rights
/// in every app, so a scope with a level names no app and no table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLevel {
    /// Read.
    Read,
    /// Read and propose changes.
    Propose,
    /// Read, propose, and write records.
    Write,
}

impl AgentLevel {
    pub fn parse(word: &str) -> Option<AgentLevel> {
        match word {
            "read" => Some(AgentLevel::Read),
            "propose" => Some(AgentLevel::Propose),
            "write" => Some(AgentLevel::Write),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            AgentLevel::Read => "read",
            AgentLevel::Propose => "propose",
            AgentLevel::Write => "write",
        }
    }

    /// The choice as the person reads it.
    pub fn sentence(self) -> &'static str {
        match self {
            AgentLevel::Read => "Read",
            AgentLevel::Propose => "Read and propose changes",
            AgentLevel::Write => "Read, propose, and write records",
        }
    }

    pub const ALL: [AgentLevel; 3] = [AgentLevel::Read, AgentLevel::Propose, AgentLevel::Write];
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenScope {
    pub apps: Vec<String>,
    /// A table id and the operations allowed on it.
    pub tables: BTreeMap<String, Vec<TokenOp>>,
    /// Omitted: the data tools that fit the operations. Present: only these, and only data tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rows_per_call: Option<u32>,
    /// An AI tool's session. When set, `apps` and `tables` are empty and the level decides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<AgentLevel>,
}

impl TokenScope {
    /// Reads a scope from what a client sent. The error is a sentence for the person who sent it.
    pub fn parse(value: &serde_json::Value) -> Result<TokenScope, String> {
        let scope: TokenScope = serde_json::from_value(value.clone()).map_err(|e| format!("The scope is not valid: {e}"))?;
        scope.validate()?;
        Ok(scope)
    }

    fn validate(&self) -> Result<(), String> {
        if self.apps.is_empty() || self.apps.iter().any(|a| a.trim().is_empty()) {
            return Err("A scope must name at least one app.".to_string());
        }
        if self.tables.is_empty() {
            return Err("A scope must name at least one table and what the token may do there.".to_string());
        }
        if let Some((table, _)) = self.tables.iter().find(|(t, ops)| t.trim().is_empty() || ops.is_empty()) {
            return Err(format!("The table '{table}' must allow at least one operation."));
        }
        if let Some(tools) = &self.tools {
            if let Some(bad) = tools.iter().find(|t| data_tool(t).is_none()) {
                let allowed: Vec<&str> = DATA_TOOLS.iter().map(|(n, _, _)| *n).collect();
                return Err(format!("The tool '{bad}' cannot be given to a limited token. It may use: {}.", allowed.join(", ")));
            }
        }
        if let Some(cap) = self.max_rows_per_call {
            if cap == 0 || cap > MAX_ROWS_LIMIT {
                return Err(format!("max_rows_per_call must be between 1 and {MAX_ROWS_LIMIT}."));
            }
        }
        Ok(())
    }

    pub fn allows_app(&self, app: &str) -> bool {
        self.apps.iter().any(|a| a == app)
    }

    pub fn allows(&self, app: &str, table: &str, op: TokenOp) -> bool {
        self.allows_app(app) && self.tables.get(table).is_some_and(|ops| ops.contains(&op))
    }

    /// Whether the token may call this tool at all. An admin tool never.
    pub fn allows_tool(&self, name: &str) -> bool {
        let Some((op, from)) = data_tool(name) else { return false };
        match &self.tools {
            Some(list) => list.iter().any(|t| t == name),
            None => from == TableFrom::App || self.tables.values().any(|ops| ops.contains(&op)),
        }
    }

    /// What the token may do, as a sentence. It names ids. The desk shows the same sentence with titles.
    pub fn sentence(&self) -> String {
        if let Some(level) = self.level {
            return format!("This session acts as the person, within their own rights, and may only: {}.", level.sentence().to_lowercase());
        }
        let mut ops: Vec<TokenOp> = self.tables.values().flatten().copied().collect();
        ops.sort();
        ops.dedup();
        let words: Vec<&str> = ops.iter().map(|o| o.word()).collect();
        let tables: Vec<&str> = self.tables.keys().map(String::as_str).collect();
        format!("This token can {} {} in {}. It cannot do anything else.", join_and(&words), join_and(&tables), join_and(&self.apps.iter().map(String::as_str).collect::<Vec<_>>()))
    }
}

fn join_and(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_string(),
        [a, b] => format!("{a} and {b}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scope() -> TokenScope {
        TokenScope::parse(&json!({ "apps": ["admissions"], "tables": { "applicants": ["read", "create"] } })).expect("a scope")
    }

    #[test]
    fn a_scope_allows_only_what_it_names() {
        let s = scope();
        assert!(s.allows("admissions", "applicants", TokenOp::Read));
        assert!(s.allows("admissions", "applicants", TokenOp::Create));
        assert!(!s.allows("admissions", "applicants", TokenOp::Update));
        assert!(!s.allows("admissions", "notes", TokenOp::Read));
        assert!(!s.allows("housing", "applicants", TokenOp::Read));
    }

    #[test]
    fn only_data_tools_are_ever_allowed() {
        let s = scope();
        assert!(s.allows_tool("list_records") && s.allows_tool("create_record") && s.allows_tool("describe_app"));
        assert!(!s.allows_tool("update_record"), "no table may be changed");
        for admin in ["propose_app_change", "decide_proposal", "manage_workspace_member", "record_governance_decision", "list_workspaces"] {
            assert!(!s.allows_tool(admin), "{admin}");
        }
    }

    #[test]
    fn a_bad_scope_says_what_is_wrong() {
        for bad in [
            json!({ "apps": [], "tables": { "t": ["read"] } }),
            json!({ "apps": ["a"], "tables": {} }),
            json!({ "apps": ["a"], "tables": { "t": [] } }),
            json!({ "apps": ["a"], "tables": { "t": ["read"] }, "tools": ["decide_proposal"] }),
            json!({ "apps": ["a"], "tables": { "t": ["read"] }, "max_rows_per_call": 0 }),
            json!({ "apps": ["a"], "tables": { "t": ["read"] }, "extra": true }),
        ] {
            assert!(TokenScope::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_sentence_names_the_operations_tables_and_apps() {
        assert_eq!(scope().sentence(), "This token can read and add applicants in admissions. It cannot do anything else.");
    }
}
