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

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, Clone)]
pub enum ServiceError {
    Unauthorized(String),
    Forbidden {
        message: String,
        reasons: Vec<String>,
        diagnostics: Vec<String>,
    },
    NotFound(String),
    BadRequest(String),
    Internal(String),
}

impl ServiceError {
    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::Forbidden {
            message: msg.into(),
            reasons: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden { .. } => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Unauthorized(msg) => msg,
            Self::Forbidden { message, .. } => message,
            Self::NotFound(msg) => msg,
            Self::BadRequest(msg) => msg,
            Self::Internal(msg) => msg,
        }
    }
}

impl IntoResponse for ServiceError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let body = match self {
            Self::Forbidden {
                message,
                reasons,
                diagnostics,
            } => {
                json!({
                    "error": message,
                    "reasons": reasons,
                    "diagnostics": diagnostics
                })
            }
            other => {
                json!({
                    "error": other.message()
                })
            }
        };
        (status, Json(body)).into_response()
    }
}
