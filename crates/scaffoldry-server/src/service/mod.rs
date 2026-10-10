//! Sovereign Service Layer
//!
//! Centralizes authorization (Cedar Policy ABAC), cryptographic ledger recording,
//! and domain mutations. Both MCP tool handlers and REST route adapters delegate
//! to this service layer.

pub mod governance;
pub mod organizations;
pub mod records;
pub mod access;
pub mod workspaces;
pub mod identity;
pub mod admin;
pub mod approvers;
pub mod people;
pub mod positions;
pub mod tools;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use scaffoldry_policy::PolicyRef;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Every `code` an error body may carry. A brief that adds a code adds it here in the same
/// change, and gives it a row in `apps/web/src/ui/explainError.ts`.
pub const ERROR_CODES: &[&str] = &[
    "unauthorized",
    "forbidden",
    "not_found",
    "bad_request",
    "version_conflict",
    "no_approver",
    "position_full",
    "too_large",
    "internal",
    "job_failed",
    "index_building",
    "calculating",
    "quota_exceeded",
    "rate_limited",
    "undo_conflict",
    "field_hidden",
    "field_read_only",
    "constraint_violated",
    "file_too_large",
    "file_type_blocked",
    "scanner_unavailable",
    "master_key_missing",
    "token_scope",
    "read_only_source",
    "view_as_read_only",
    "share_not_allowed",
    "archived",
];

#[derive(Debug, Clone)]
pub enum ServiceError {
    Unauthorized(String),
    Forbidden {
        message: String,
        reasons: Vec<String>,
        diagnostics: Vec<String>,
        /// The policy that decided, when a Cedar `forbid` did.
        policy: Option<PolicyRef>,
    },
    NotFound(String),
    BadRequest(String),
    /// Bad input that names the fields at fault, such as `{ "reason": "required" }`.
    Invalid {
        message: String,
        fields: BTreeMap<String, String>,
    },
    /// Someone else changed the thing first.
    Conflict(String),
    /// The position already has as many holders as it allows.
    PositionFull(String),
    /// A save named a version that is no longer the stored one. `version` is the current one.
    StaleVersion { message: String, version: i32 },
    TooLarge(String),
    Internal(String),
}

impl ServiceError {
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::Unauthorized(msg.into())
    }

    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::Forbidden {
            message: msg.into(),
            reasons: Vec::new(),
            diagnostics: Vec::new(),
            policy: None,
        }
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::NotFound(msg.into())
    }

    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::BadRequest(msg.into())
    }

    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::Conflict(msg.into())
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }

    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden { .. } => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::BadRequest(_) | Self::Invalid { .. } => StatusCode::BAD_REQUEST,
            Self::Conflict(_) | Self::PositionFull(_) | Self::StaleVersion { .. } => StatusCode::CONFLICT,
            Self::TooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// The `code` in the body. It is always one of `ERROR_CODES`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unauthorized(_) => "unauthorized",
            Self::Forbidden { .. } => "forbidden",
            Self::NotFound(_) => "not_found",
            Self::BadRequest(_) | Self::Invalid { .. } => "bad_request",
            Self::Conflict(_) | Self::StaleVersion { .. } => "version_conflict",
            Self::PositionFull(_) => "position_full",
            Self::TooLarge(_) => "too_large",
            Self::Internal(_) => "internal",
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Unauthorized(msg)
            | Self::NotFound(msg)
            | Self::BadRequest(msg)
            | Self::Conflict(msg)
            | Self::PositionFull(msg)
            | Self::TooLarge(msg)
            | Self::Internal(msg) => msg,
            Self::Forbidden { message, .. } | Self::Invalid { message, .. } | Self::StaleVersion { message, .. } => message,
        }
    }

    /// The JSON body. Built field by field, so no handler builds an error body by hand.
    pub fn body(&self) -> Value {
        let mut body = Map::new();
        body.insert("error".to_string(), Value::String(self.message().to_string()));
        body.insert("code".to_string(), Value::String(self.code().to_string()));
        match self {
            Self::Forbidden { reasons, diagnostics, policy, .. } => {
                body.insert("reasons".to_string(), serde_json::to_value(reasons).unwrap_or(Value::Null));
                body.insert("diagnostics".to_string(), serde_json::to_value(diagnostics).unwrap_or(Value::Null));
                if let Some(policy) = policy {
                    body.insert("policy".to_string(), serde_json::to_value(policy).unwrap_or(Value::Null));
                }
            }
            Self::Invalid { fields, .. } => {
                body.insert("fields".to_string(), serde_json::to_value(fields).unwrap_or(Value::Null));
            }
            Self::StaleVersion { version, .. } => {
                body.insert("version".to_string(), Value::from(*version));
            }
            _ => {}
        }
        Value::Object(body)
    }

    /// For handlers whose error type is a status and a JSON body.
    pub fn into_pair(self) -> (StatusCode, Json<Value>) {
        (self.status_code(), Json(self.body()))
    }
}

impl IntoResponse for ServiceError {
    fn into_response(self) -> Response {
        self.into_pair().into_response()
    }
}
