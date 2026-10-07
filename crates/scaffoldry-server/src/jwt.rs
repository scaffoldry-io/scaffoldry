//! OAuth 2.1 / OIDC JWT Verification, Claims Mapping, and Test Issuer
//!
//! Enforces:
//! - RFC 7519 (JWT), RFC 7515 (JWS), RFC 7517 (JWK) compliance.
//! - Cryptographic HMAC-SHA256 signature verification.
//! - Mapping of institutional claims (`sub`/`eppn`, `name`, `affiliation`, `department`) to `AuthUser`.
//! - Fail-closed validation (invalid signatures, expired tokens, untrusted issuers).

use crate::state::AuthUser;
use base64::prelude::{Engine as _, BASE64_URL_SAFE_NO_PAD};
use chrono::Utc;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub const DEFAULT_ISSUER: &str = "https://auth.state.edu";
pub const DEFAULT_SECRET: &str = "scaffoldry-institutional-oauth2-dev-signing-secret-key-2026";

#[derive(Debug, thiserror::Error)]
pub enum JwtError {
    #[error("Malformed JWT: expected 3 dot-separated parts")]
    Malformed,
    #[error("Base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Cryptographic signature mismatch")]
    InvalidSignature,
    #[error("Unsupported algorithm: expected HS256")]
    UnsupportedAlgorithm,
    #[error("Token expired at {0}")]
    Expired(i64),
    #[error("Untrusted issuer: expected {expected}, got {got}")]
    UntrustedIssuer { expected: String, got: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtHeader {
    pub alg: String,
    pub typ: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    pub iss: String,
    pub sub: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aud: Option<String>,
    pub exp: i64,
    pub iat: i64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub role_title: Option<String>,
    #[serde(default)]
    pub affiliation: Option<String>,
    #[serde(default)]
    pub department: Option<String>,
    #[serde(default)]
    pub original_admin: Option<AuthUser>,
}

impl JwtClaims {
    pub fn to_auth_user(&self) -> AuthUser {
        AuthUser {
            eppn: self.sub.clone(),
            name: self.name.clone().unwrap_or_else(|| self.sub.clone()),
            role_title: self.role_title.clone().unwrap_or_else(|| "Member".to_string()),
            affiliation: self.affiliation.clone().unwrap_or_else(|| "staff".to_string()),
            department: self.department.clone().unwrap_or_else(|| "general".to_string()),
        }
    }
}

pub fn get_jwt_secret() -> Vec<u8> {
    std::env::var("SCAFFOLDRY_JWT_SECRET")
        .unwrap_or_else(|_| DEFAULT_SECRET.to_string())
        .into_bytes()
}

pub fn get_jwt_issuer() -> String {
    std::env::var("SCAFFOLDRY_OIDC_ISSUER").unwrap_or_else(|_| DEFAULT_ISSUER.to_string())
}

/// Validates a bearer JWT string against the configured issuer and signing secret.
pub fn validate_jwt(token: &str, secret: &[u8], expected_issuer: &str) -> Result<JwtClaims, JwtError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(JwtError::Malformed);
    }

    let header_b64 = parts[0];
    let payload_b64 = parts[1];
    let signature_b64 = parts[2];

    // 1. Verify header
    let header_bytes = BASE64_URL_SAFE_NO_PAD.decode(header_b64)?;
    let header: JwtHeader = serde_json::from_slice(&header_bytes)?;
    if header.alg != "HS256" {
        return Err(JwtError::UnsupportedAlgorithm);
    }

    // 2. Verify HMAC-SHA256 signature
    let decoded_sig = BASE64_URL_SAFE_NO_PAD.decode(signature_b64)?;
    let mut mac = HmacSha256::new_from_slice(secret)
        .map_err(|_| JwtError::InvalidSignature)?;
    let signed_data = format!("{header_b64}.{payload_b64}");
    mac.update(signed_data.as_bytes());

    if mac.verify_slice(&decoded_sig).is_err() {
        return Err(JwtError::InvalidSignature);
    }

    // 3. Parse claims
    let payload_bytes = BASE64_URL_SAFE_NO_PAD.decode(payload_b64)?;
    let claims: JwtClaims = serde_json::from_slice(&payload_bytes)?;

    // 4. Verify expiration
    let now = Utc::now().timestamp();
    if claims.exp < now {
        return Err(JwtError::Expired(claims.exp));
    }

    // 5. Verify issuer
    if claims.iss != expected_issuer {
        return Err(JwtError::UntrustedIssuer {
            expected: expected_issuer.to_string(),
            got: claims.iss,
        });
    }

    Ok(claims)
}

/// Encodes and signs a JWT with HS256 using the provided secret.
pub fn sign_jwt(claims: &JwtClaims, secret: &[u8]) -> Result<String, JwtError> {
    let header = JwtHeader {
        alg: "HS256".to_string(),
        typ: "JWT".to_string(),
        kid: Some("test-key-1".to_string()),
    };

    let header_json = serde_json::to_vec(&header)?;
    let header_b64 = BASE64_URL_SAFE_NO_PAD.encode(header_json);

    let payload_json = serde_json::to_vec(claims)?;
    let payload_b64 = BASE64_URL_SAFE_NO_PAD.encode(payload_json);

    let signed_data = format!("{header_b64}.{payload_b64}");
    let mut mac = HmacSha256::new_from_slice(secret)
        .map_err(|_| JwtError::InvalidSignature)?;
    mac.update(signed_data.as_bytes());
    let sig = mac.finalize().into_bytes();
    let sig_b64 = BASE64_URL_SAFE_NO_PAD.encode(sig);

    Ok(format!("{signed_data}.{sig_b64}"))
}

#[derive(Debug, Clone)]
pub struct TestJwtParams {
    pub eppn: String,
    pub name: String,
    pub role_title: String,
    pub affiliation: String,
    pub department: String,
    pub expires_in_secs: i64,
}

/// Mints a signed JWT for local development and test suites.
pub fn mint_test_jwt(params: TestJwtParams) -> Result<String, JwtError> {
    let now = Utc::now().timestamp();
    let claims = JwtClaims {
        iss: get_jwt_issuer(),
        sub: params.eppn,
        aud: Some("scaffoldry".to_string()),
        exp: now + params.expires_in_secs,
        iat: now,
        name: Some(params.name),
        role_title: Some(params.role_title),
        affiliation: Some(params.affiliation),
        department: Some(params.department),
        original_admin: None,
    };
    sign_jwt(&claims, &get_jwt_secret())
}

/// Mints an impersonation JWT signed by the test issuer.
pub fn mint_impersonation_jwt(
    target_user: &AuthUser,
    real_admin: &AuthUser,
    expires_in_secs: i64,
) -> Result<String, JwtError> {
    let now = Utc::now().timestamp();
    let claims = JwtClaims {
        iss: get_jwt_issuer(),
        sub: target_user.eppn.clone(),
        aud: Some("scaffoldry".to_string()),
        exp: now + expires_in_secs,
        iat: now,
        name: Some(target_user.name.clone()),
        role_title: Some(target_user.role_title.clone()),
        affiliation: Some(target_user.affiliation.clone()),
        department: Some(target_user.department.clone()),
        original_admin: Some(real_admin.clone()),
    };
    sign_jwt(&claims, &get_jwt_secret())
}
