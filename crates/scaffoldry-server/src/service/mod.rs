//! Sovereign Service Layer
//!
//! Centralizes authorization (Cedar Policy ABAC), cryptographic ledger recording,
//! and domain mutations. Both MCP tool handlers and REST route adapters delegate
//! to this service layer.

pub mod governance;
pub mod authority;
pub mod authority_changes;
pub mod authority_load;
pub mod library;
pub mod library_public;
pub mod library_review;
pub mod library_tools;
pub mod history;
pub mod organizations;
pub mod records;
pub mod access;
pub mod access_drafts;
pub mod access_grants;
pub mod access_requests;
pub mod hierarchies;
pub mod supervision;
pub mod access_review;
pub mod receipts;
pub mod coordinates;
pub mod catalog;
pub mod guidance;
pub mod comments;
pub mod disclosure;
pub mod workspaces;
pub mod identity;
pub mod admin;
pub mod approvers;
pub mod people;
pub mod positions;
pub mod processes;
pub mod pages;
pub mod proposals;
pub mod hints;
pub mod prompts;
pub mod tool_meta;
pub mod tools;
pub mod calc;
pub mod agent_kit;
pub mod clients;
pub mod oauth;
pub mod openapi;
pub mod mcp_ui;
pub mod client_ts;
pub mod preview;
pub mod token_scope;
pub mod view_as;
pub mod views;
pub mod link_options;
pub mod grants;
pub mod members;
pub mod my_apps;
pub mod scope;
pub mod synthesize;
pub mod trash;
pub mod undo;

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
    "too_many_indexes",
    "storage_unavailable",
    "grant_outside_scope",
    "grant_too_broad",
    "proposal_changed",
    "preview_read_only",
    "client_sensitive_field",
    "client_scope",
    "impersonation_not_synthetic",
    "grant_conflict",
    "citation_not_found",
    "authority_loosens",
];

/// What a person who tried to act as a real person should do instead.
pub const IMPERSONATION_REMEDY: &str = "Use view-as to see what they see. View-as is read-only and works for real people.";

#[derive(Debug, Clone)]
pub enum ServiceError {
    Unauthorized(String),
    Forbidden {
        message: String,
        reasons: Vec<String>,
        diagnostics: Vec<String>,
        /// The policy that decided, when a Cedar `forbid` did.
        policy: Option<Box<PolicyRef>>,
        /// The Cedar action that was refused, when a Cedar decision refused it. A receipt keeps it so the decision can be asked again.
        action: Option<String>,
        /// What to send to `POST /access/requests` to ask for the access that was refused, filled in.
        request: Option<Box<Value>>,
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
    /// An upload is longer than `files.max_mb`.
    FileTooLarge(String),
    /// An upload is an executable, a script, or a page, by its bytes or by its name.
    FileTypeBlocked(String),
    /// Uploads need a scan and no scanner answers.
    ScannerUnavailable(String),
    /// The database cannot be read, so nothing the server holds can be trusted.
    StorageUnavailable(String),
    /// A sort or filter names a field whose index is still being built.
    IndexBuilding(String),
    /// An undo found records that have changed since the batch. `conflicts` lists them.
    UndoConflict { message: String, conflicts: Vec<Value> },
    /// A rule the app set stops the change, such as a delete while records still link to the record.
    ConstraintViolated(String),
    /// A record violates a field constraint (required, unique, min, max).
    ConstraintViolation {
        message: String,
        field: String,
        constraint: String,
    },
    /// A filter or sort names a field whose values are being calculated again.
    Calculating(String),
    /// A person or the server has too much queued work. They can try again when it clears.
    QuotaExceeded(String),
    /// A view-as session tried to change something. View-as only reads.
    ViewAsReadOnly(String),
    /// A grant or an appointment would give one person both sides of a declared conflict.
    GrantConflict(String),
    /// Full impersonation was asked for a real person. Only synthetic people can be acted as.
    ImpersonationNotSynthetic(String),
    /// A cited passage is not in the snapshot it names. A passage is copied word for word.
    CitationNotFound(String),
    /// A change to the decision-authority matrix would ask for less than the built-in answer.
    AuthorityLoosens(String),
    /// A preview of a proposal tried to change something. A preview only reads.
    PreviewReadOnly(String),
    /// A token limited to some apps, tables or operations asked for more than its scope allows.
    TokenScope(String),
    /// An outside app asked for something its ceiling does not cover.
    ClientScope(String),
    /// A client's ceiling names a field that is sensitive. Sensitive data stays inside the platform.
    ClientSensitiveField(String),
    /// A filter, sort, or total names a field the person cannot see. It would let them read it by inference.
    FieldHidden { message: String, reason: String, remedy: String, request: Option<Box<Value>> },
    /// A table may index 20 fields. `unused` lists the indexed fields no view uses, which can go.
    TooManyIndexes { message: String, unused: Vec<String> },
    /// A write names fields the person may not change.
    FieldReadOnly { message: String, fields: Vec<String>, reason: String, remedy: String },
    /// The caller may not grant this: the anchor is outside their subtree, or the role is above what they may give.
    GrantOutsideScope(String),
    /// A grant to everyone below a hierarchy's root. Institutional visibility already says that.
    GrantTooBroad(String),
    /// An approval named a digest that is not the proposal's. Nothing was applied.
    ProposalChanged(String),
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
            action: None,
            request: None,
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
            Self::Forbidden { .. } | Self::GrantOutsideScope(_) | Self::FieldHidden { .. } | Self::FieldReadOnly { .. } | Self::ViewAsReadOnly(_) | Self::ImpersonationNotSynthetic(_) | Self::PreviewReadOnly(_) | Self::TokenScope(_) | Self::ClientScope(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::ConstraintViolation { .. } | Self::GrantTooBroad(_) | Self::ClientSensitiveField(_) | Self::BadRequest(_) | Self::CitationNotFound(_) | Self::AuthorityLoosens(_) | Self::Invalid { .. } | Self::TooManyIndexes { .. } | Self::FileTypeBlocked(_) => StatusCode::BAD_REQUEST,
            Self::Conflict(_) | Self::GrantConflict(_) | Self::ProposalChanged(_) | Self::PositionFull(_) | Self::StaleVersion { .. } | Self::IndexBuilding(_) | Self::Calculating(_) | Self::ConstraintViolated(_) | Self::UndoConflict { .. } => StatusCode::CONFLICT,
            Self::QuotaExceeded(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::TooLarge(_) | Self::FileTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Self::ScannerUnavailable(_) | Self::StorageUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
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
            Self::FileTooLarge(_) => "file_too_large",
            Self::FileTypeBlocked(_) => "file_type_blocked",
            Self::ScannerUnavailable(_) => "scanner_unavailable",
            Self::StorageUnavailable(_) => "storage_unavailable",
            Self::IndexBuilding(_) => "index_building",
            Self::Calculating(_) => "calculating",
            Self::ConstraintViolation { .. } | Self::ConstraintViolated(_) => "constraint_violated",
            Self::QuotaExceeded(_) => "quota_exceeded",
            Self::ViewAsReadOnly(_) => "view_as_read_only",
            Self::ImpersonationNotSynthetic(_) => "impersonation_not_synthetic",
            Self::GrantConflict(_) => "grant_conflict",
            Self::CitationNotFound(_) => "citation_not_found",
            Self::AuthorityLoosens(_) => "authority_loosens",
            Self::PreviewReadOnly(_) => "preview_read_only",
            Self::TokenScope(_) => "token_scope",
            Self::ClientScope(_) => "client_scope",
            Self::ClientSensitiveField(_) => "client_sensitive_field",
            Self::FieldHidden { .. } => "field_hidden",
            Self::TooManyIndexes { .. } => "too_many_indexes",
            Self::FieldReadOnly { .. } => "field_read_only",
            Self::UndoConflict { .. } => "undo_conflict",
            Self::GrantOutsideScope(_) => "grant_outside_scope",
            Self::GrantTooBroad(_) => "grant_too_broad",
            Self::ProposalChanged(_) => "proposal_changed",
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
            | Self::FileTooLarge(msg)
            | Self::FileTypeBlocked(msg)
            | Self::ScannerUnavailable(msg)
            | Self::StorageUnavailable(msg)
            | Self::IndexBuilding(msg)
            | Self::Calculating(msg)
            | Self::QuotaExceeded(msg)
            | Self::ViewAsReadOnly(msg)
            | Self::ImpersonationNotSynthetic(msg)
            | Self::GrantConflict(msg)
            | Self::CitationNotFound(msg)
            | Self::AuthorityLoosens(msg)
            | Self::PreviewReadOnly(msg)
            | Self::TokenScope(msg)
            | Self::ClientScope(msg)
            | Self::ClientSensitiveField(msg)
            | Self::ConstraintViolated(msg)
            | Self::GrantOutsideScope(msg)
            | Self::GrantTooBroad(msg)
            | Self::ProposalChanged(msg)
            | Self::Internal(msg) => msg,
            Self::ConstraintViolation { message, .. } | Self::FieldHidden { message, .. } | Self::TooManyIndexes { message, .. } | Self::FieldReadOnly { message, .. } | Self::Forbidden { message, .. } | Self::Invalid { message, .. } | Self::StaleVersion { message, .. } | Self::UndoConflict { message, .. } => message,
        }
    }

    /// The JSON body. Built field by field, so no handler builds an error body by hand.
    pub fn body(&self) -> Value {
        let mut body = Map::new();
        body.insert("error".to_string(), Value::String(self.message().to_string()));
        body.insert("code".to_string(), Value::String(self.code().to_string()));
        match self {
            Self::Forbidden { reasons, diagnostics, policy, action, request, .. } => {
                if let Some(request) = request {
                    body.insert("request".to_string(), (**request).clone());
                }
                if let Some(action) = action {
                    body.insert("action".to_string(), Value::String(action.clone()));
                }
                body.insert("reasons".to_string(), serde_json::to_value(reasons).unwrap_or(Value::Null));
                body.insert("diagnostics".to_string(), serde_json::to_value(diagnostics).unwrap_or(Value::Null));
                if let Some(policy) = policy {
                    body.insert("policy".to_string(), serde_json::to_value(policy).unwrap_or(Value::Null));
                }
            }
            Self::ConstraintViolation { field, constraint, .. } => {
                body.insert("field".to_string(), Value::String(field.clone()));
                body.insert("constraint".to_string(), Value::String(constraint.clone()));
            }
            Self::Invalid { fields, .. } => {
                body.insert("fields".to_string(), serde_json::to_value(fields).unwrap_or(Value::Null));
            }
            Self::StaleVersion { version, .. } => {
                body.insert("version".to_string(), Value::from(*version));
            }
            Self::UndoConflict { conflicts, .. } => {
                body.insert("conflicts".to_string(), Value::Array(conflicts.clone()));
            }
            Self::TooManyIndexes { unused, .. } => {
                body.insert("unused".to_string(), serde_json::to_value(unused).unwrap_or(Value::Null));
            }
            Self::ImpersonationNotSynthetic(_) => {
                body.insert("remedy".to_string(), Value::String(IMPERSONATION_REMEDY.to_string()));
            }
            Self::FieldHidden { reason, remedy, request, .. } => {
                if let Some(request) = request {
                    body.insert("request".to_string(), (**request).clone());
                }
                body.insert("reason".to_string(), Value::String(reason.clone()));
                body.insert("remedy".to_string(), Value::String(remedy.clone()));
            }
            Self::FieldReadOnly { fields, reason, remedy, .. } => {
                body.insert("fields".to_string(), serde_json::to_value(fields.iter().map(|f| (f.clone(), "read_only".to_string())).collect::<BTreeMap<_, _>>()).unwrap_or(Value::Null));
                body.insert("reason".to_string(), Value::String(reason.clone()));
                body.insert("remedy".to_string(), Value::String(remedy.clone()));
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

/// After a write to a cached collection: this instance reads the database again, so the request that wrote sees its
/// own change. The write has committed. When the reload fails, the answer says so, and the next request tries again.
pub fn reload_after_write(state: &crate::state::ServerState) -> Result<(), ServiceError> {
    state.refresh().map_err(|e| ServiceError::StorageUnavailable(format!("The change was saved, but this server could not reload: {e}")))
}
