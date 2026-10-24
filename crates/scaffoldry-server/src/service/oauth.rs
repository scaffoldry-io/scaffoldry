//! Signing in to an outside app (external apps phase 2).
//!
//! The authorization code flow with PKCE (S256 only), the one flow for people (OAuth 2.1). A user who is signed in to
//! the desk sees the client, its operator and its ceiling, and allows or denies. The client trades a single-use code
//! for an opaque one-hour token. The token is stored as a hash like every other token. Scaffoldry signs nothing.
//!
//! A client token carries the scope its ceiling comes to (`clients::scope_of`). Phase 3 enforces it.

use crate::repository_clients::{ClientRow, CodeRow, CodeUse};
use crate::service::clients::{ceiling_sentences, ClientView};
use crate::service::identity::{generate_raw_token, hash_token, resolve_user};
use crate::service::ServiceError;
use crate::state::{ApiToken, AuthUser, RecordDecisionInput, SharedState};
use base64::Engine;
use scaffoldry_core::DecisionType;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const TOKEN_SECONDS: i64 = 3600;

fn internal(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

/// This server's public address, from the address the request was made to. A proxy that terminates TLS sets
/// `X-Forwarded-Proto` and `X-Forwarded-Host`. Without them the scheme is https.
pub fn issuer_of(headers: &axum::http::HeaderMap) -> String {
    let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(|s| s.split(',').next().unwrap_or_default().trim().to_string());
    let host = get("x-forwarded-host").or_else(|| get("host")).filter(|h| !h.is_empty()).unwrap_or_else(|| "localhost".to_string());
    let scheme = get("x-forwarded-proto").filter(|p| p == "http" || p == "https").unwrap_or_else(|| "https".to_string());
    format!("{scheme}://{host}")
}

/// RFC 8414 authorization server metadata.
pub fn authorization_server_metadata(issuer: &str) -> Value {
    json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/oauth/authorize"),
        "token_endpoint": format!("{issuer}/oauth/token"),
        "revocation_endpoint": format!("{issuer}/oauth/revoke"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["client_secret_basic", "none"],
        "revocation_endpoint_auth_methods_supported": ["client_secret_basic", "none"]
    })
}

/// An error in the shape RFC 6749 gives the token and revocation endpoints.
#[derive(Debug, Serialize)]
pub struct OAuthError {
    pub error: &'static str,
    pub error_description: String,
    #[serde(skip)]
    pub status: u16,
}

impl OAuthError {
    fn new(status: u16, error: &'static str, description: impl Into<String>) -> Self {
        OAuthError { error, error_description: description.into(), status }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct AuthorizeParams {
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub redirect_uri: String,
    #[serde(default)]
    pub response_type: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub code_challenge: String,
    #[serde(default)]
    pub code_challenge_method: String,
    #[serde(default)]
    pub resource: String,
    #[serde(default)]
    pub decision: String,
}

fn origin_of(uri: &str) -> String {
    let (scheme, rest) = uri.split_once("://").unwrap_or(("https", uri));
    format!("{scheme}://{}", rest.split('/').next().unwrap_or_default())
}

/// The origins of the active clients' redirect addresses. A browser app at one of them may call the server.
pub fn client_origins(state: &SharedState) -> Vec<String> {
    state.repository.active_client_redirects().unwrap_or_default().iter().map(|u| origin_of(u)).collect()
}

fn pkce_charset(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~'))
}

fn digest_of(client: &ClientRow) -> String {
    let bytes = Sha256::digest(client.ceiling.to_string().as_bytes());
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The client, once every parameter has been checked. A bad client or address is never redirected to.
fn checked(p: &AuthorizeParams, issuer: &str, state: &SharedState) -> Result<ClientRow, ServiceError> {
    let client = state.repository.get_client(&p.client_id).map_err(internal)?.filter(|c| c.status == "Active");
    let client = client.ok_or_else(|| ServiceError::bad_request("This app is not registered here, or has been turned off."))?;
    if !client.redirect_uris.contains(&p.redirect_uri) {
        return Err(ServiceError::bad_request("The return address does not match one this app registered."));
    }
    if p.response_type != "code" {
        return Err(ServiceError::bad_request("response_type must be code."));
    }
    if p.state.trim().is_empty() {
        return Err(ServiceError::bad_request("state is required."));
    }
    if p.code_challenge_method != "S256" {
        return Err(ServiceError::bad_request("code_challenge_method must be S256."));
    }
    if !(43..=128).contains(&p.code_challenge.len()) || !pkce_charset(&p.code_challenge) {
        return Err(ServiceError::bad_request("code_challenge must be a 43 to 128 character base64url value."));
    }
    let resource = p.resource.trim_end_matches('/');
    if resource != issuer && resource != format!("{issuer}/api/mcp") {
        return Err(ServiceError::bad_request("resource must be this server."));
    }
    Ok(client)
}

/// What the consent screen shows: the client, its operator, the callback origin, and the ceiling as sentences.
pub fn consent(user: &AuthUser, p: &AuthorizeParams, issuer: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let client = checked(p, issuer, state)?;
    let remembered = state.repository.consent_digest(&user.eppn, &client.client_id).map_err(internal)?.as_deref() == Some(digest_of(&client).as_str());
    Ok(json!({
        "client": { "client_id": client.client_id, "name": client.name, "operator": client.operator, "callback_origin": origin_of(&p.redirect_uri) },
        "sentences": ceiling_sentences(&ClientView(&client), state),
        "remembered": remembered,
        "user": { "eppn": user.eppn, "name": user.name }
    }))
}

fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// The user's decision. The reply names the address to send them to, with `code` and `state`, or `error` and `state`.
pub fn decide(user: &AuthUser, p: &AuthorizeParams, issuer: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let client = checked(p, issuer, state)?;
    let state_param = encode(&p.state);
    let target = match p.decision.as_str() {
        "deny" => format!("{}?error=access_denied&state={state_param}", p.redirect_uri),
        "allow" => {
            let raw = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
            let digest = digest_of(&client);
            state
                .repository
                .insert_code(
                    &hash_token(&raw),
                    &CodeRow { client_id: client.client_id.clone(), eppn: user.eppn.clone(), redirect_uri: p.redirect_uri.clone(), code_challenge: p.code_challenge.clone(), ceiling_digest: digest.clone() },
                )
                .map_err(internal)?;
            state.repository.upsert_consent(&user.eppn, &client.client_id, &digest).map_err(internal)?;
            format!("{}?code={raw}&state={state_param}", p.redirect_uri)
        }
        _ => return Err(ServiceError::bad_request("decision must be allow or deny.")),
    };
    Ok(json!({ "redirect_to": target }))
}

/// A client, from the Basic header or the `client_id` of a public client.
fn authenticate(basic: Option<(String, String)>, form_client_id: Option<&str>, state: &SharedState) -> Result<ClientRow, OAuthError> {
    let invalid = |why: &str| OAuthError::new(401, "invalid_client", why);
    let id = basic.as_ref().map(|(id, _)| id.as_str()).or(form_client_id).unwrap_or_default();
    if let (Some((basic_id, _)), Some(form_id)) = (&basic, form_client_id) {
        if basic_id != form_id {
            return Err(invalid("The client_id differs from the one in the Authorization header."));
        }
    }
    let client = state.repository.get_client(id).ok().flatten().filter(|c| c.status == "Active").ok_or_else(|| invalid("Unknown client."))?;
    if client.kind == "confidential" {
        let given = basic.map(|(_, secret)| hash_token(&secret));
        let stored = state.repository.secret_hash_of_client(&client.client_id).ok().flatten().map(|s| s.trim().to_string());
        if given.is_none() || given != stored {
            return Err(invalid("This client must authenticate with its secret."));
        }
    } else if basic.is_some() {
        return Err(invalid("This client is public and holds no secret."));
    }
    Ok(client)
}

/// Splits an `Authorization: Basic` value into the client id and secret.
pub fn basic_of(header: Option<&str>) -> Option<(String, String)> {
    let encoded = header?.strip_prefix("Basic ")?;
    let decoded = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(encoded.trim()).ok()?).ok()?;
    let (id, secret) = decoded.split_once(':')?;
    Some((id.to_string(), secret.to_string()))
}

#[derive(Debug, Default, Deserialize)]
pub struct TokenForm {
    #[serde(default)]
    pub grant_type: String,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub redirect_uri: String,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub code_verifier: String,
}

/// Trades a code for a token.
pub fn token(basic: Option<(String, String)>, form: &TokenForm, state: &SharedState) -> Result<Value, OAuthError> {
    if form.grant_type != "authorization_code" {
        return Err(OAuthError::new(400, "unsupported_grant_type", "Only authorization_code is supported."));
    }
    let client = authenticate(basic, form.client_id.as_deref(), state)?;
    let grant = |why: &str| OAuthError::new(400, "invalid_grant", why);
    if form.code.is_empty() || form.redirect_uri.is_empty() || form.code_verifier.is_empty() {
        return Err(OAuthError::new(400, "invalid_request", "code, redirect_uri and code_verifier are required."));
    }
    let code_hash = hash_token(&form.code);
    let row = match state.repository.take_code(&code_hash).map_err(|e| OAuthError::new(500, "server_error", e.to_string()))? {
        CodeUse::Fresh(row) => row,
        CodeUse::Reused => return Err(grant("This code was already used. The tokens it issued have been revoked.")),
        CodeUse::Missing => return Err(grant("This code is unknown or has expired.")),
    };
    if row.client_id != client.client_id || row.redirect_uri != form.redirect_uri {
        return Err(grant("This code was not issued to this client and address."));
    }
    let verifier_ok = (43..=128).contains(&form.code_verifier.len()) && pkce_charset(&form.code_verifier);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(form.code_verifier.as_bytes()));
    if !verifier_ok || challenge != row.code_challenge {
        return Err(grant("The code_verifier does not match."));
    }
    if row.ceiling_digest != digest_of(&client) {
        return Err(grant("The ceiling changed since you agreed. Sign in again."));
    }
    if resolve_user(&row.eppn, state).is_none() {
        return Err(grant("This person cannot sign in now."));
    }

    let raw = generate_raw_token();
    let now = chrono::Utc::now();
    let token = ApiToken {
        token_hash: hash_token(&raw),
        id: uuid::Uuid::new_v4(),
        kind: "client".to_string(),
        eppn: row.eppn.clone(),
        // The label is what the history shows for a write made through this app, so it names the client id.
        label: format!("Client {} [{}]", client.name, client.client_id),
        original_admin: None,
        created_at: now,
        expires_at: now + chrono::Duration::seconds(TOKEN_SECONDS),
        last_used_at: None,
        revoked_at: None,
        app_slug: None,
        scope: Some(crate::service::clients::scope_of(&crate::service::clients::ceiling_of(&client))),
    };
    let server_error = |e: &dyn std::fmt::Display| OAuthError::new(500, "server_error", e.to_string());
    state.persist_api_token(&token).map_err(|e| server_error(&e))?;
    state.repository.insert_oauth_token(&token.token_hash, &client.client_id, &row.eppn, &code_hash).map_err(|e| server_error(&e))?;
    Ok(json!({ "access_token": raw, "token_type": "Bearer", "expires_in": TOKEN_SECONDS }))
}

/// RFC 7009. The client revokes one of its own tokens. The answer is the same for a token it does not own.
pub fn revoke(basic: Option<(String, String)>, client_id: Option<&str>, token: &str, state: &SharedState) -> Result<(), OAuthError> {
    let client = authenticate(basic, client_id, state)?;
    if !token.is_empty() {
        let _ = state.repository.revoke_client_token(&hash_token(token), &client.client_id);
    }
    Ok(())
}

/// The client behind a token of kind `client`, when that client is still active.
pub fn active_client_of(token_hash: &str, state: &SharedState) -> Option<ClientRow> {
    state.repository.client_of_token(token_hash).ok().flatten().filter(|c| c.status == "Active")
}

/// The clients a person has agreed to.
pub fn connections(user: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    let mut out = Vec::new();
    for (client_id, granted_at) in state.repository.consents_of(&user.eppn).map_err(internal)? {
        if let Some(client) = state.repository.get_client(&client_id).map_err(internal)? {
            out.push(json!({
                "client_id": client.client_id, "name": client.name, "operator": client.operator, "status": client.status,
                "sentences": ceiling_sentences(&ClientView(&client), state), "connected_at": granted_at
            }));
        }
    }
    Ok(Value::Array(out))
}

/// A person disconnects a client. Their consent goes and the client's tokens for them stop. Ledger first.
pub fn disconnect(user: &AuthUser, client_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let known = state.repository.consents_of(&user.eppn).map_err(internal)?.iter().any(|(id, _)| id == client_id);
    let client = state.repository.get_client(client_id).map_err(internal)?.filter(|_| known);
    let client = client.ok_or_else(|| ServiceError::not_found("You are not connected to that app."))?;
    let payload = json!({ "client_id": client.client_id, "name": client.name, "user": user.eppn });
    state
        .append_ledger_entry(RecordDecisionInput {
            principal: user.eppn.clone(),
            organization_code: String::new(),
            app_slug: None,
            decision_type: DecisionType::ClientDisconnected,
            oscal_control_id: "AC-02".to_string(),
            rationale: format!("{} disconnected the outside app '{}'", user.name, client.name),
            payload: &payload,
        })
        .map_err(|e| ServiceError::internal(format!("Could not record the ledger entry: {e}")))?;
    state.repository.disconnect_client(&user.eppn, &client.client_id).map_err(internal)?;
    Ok(json!({ "status": "disconnected" }))
}
