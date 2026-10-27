//! Signing in to an outside app. The rules are in `service::oauth`.

use crate::guard::session_user;
use crate::service::oauth::{self, AuthorizeParams, OAuthError, TokenForm};
use crate::service::ServiceError;
use crate::state::SharedState;
use axum::{
    extract::{Form, Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::Value;

/// Routes at the root of the server: the metadata, the consent calls and the token endpoints.
pub fn root_router() -> Router<SharedState> {
    Router::new()
        .route("/.well-known/oauth-authorization-server", get(authorization_server))
        .route("/oauth/authorize", get(show_consent).post(decide_consent))
        .route("/oauth/token", post(token))
        .route("/oauth/revoke", post(revoke))
}

/// Routes under `/api/v1`: a person's own connections, and the consent calls the desk makes. The desk reaches the
/// server through `/api/v1`, so the consent screen is served from there as well as from the root.
pub fn me_router() -> Router<SharedState> {
    Router::new()
        .route("/me/connections", get(list_connections))
        .route("/me/connections/{client_id}", delete(disconnect))
        .route("/oauth/authorize", get(show_consent).post(decide_consent))
}

type Reply = Result<Json<Value>, (StatusCode, Json<Value>)>;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ServiceError> + Send + 'static) -> Result<T, (StatusCode, Json<Value>)> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| ServiceError::internal("The request stopped").into_pair())?
        .map_err(ServiceError::into_pair)
}

async fn authorization_server(State(state): State<SharedState>, headers: HeaderMap) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let issuer = oauth::issuer_of(&headers);
    blocking(move || {
        let any = state.repository.any_service_identity().map_err(|e| ServiceError::internal(e.to_string()))?;
        Ok(oauth::authorization_server_metadata(&issuer, any))
    })
    .await
    .map(Json)
}

async fn show_consent(State(state): State<SharedState>, headers: HeaderMap, Query(params): Query<AuthorizeParams>) -> Reply {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Sign in to the desk first").into_pair())?;
    let issuer = oauth::issuer_of(&headers);
    blocking(move || oauth::consent(&user, &params, &issuer, &state)).await.map(Json)
}

async fn decide_consent(State(state): State<SharedState>, headers: HeaderMap, Json(params): Json<AuthorizeParams>) -> Reply {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Sign in to the desk first").into_pair())?;
    let issuer = oauth::issuer_of(&headers);
    blocking(move || oauth::decide(&user, &params, &issuer, &state)).await.map(Json)
}

fn oauth_reply(result: Result<Value, OAuthError>) -> Response {
    let mut response = match result {
        Ok(body) => Json(body).into_response(),
        Err(e) => {
            let status = StatusCode::from_u16(e.status).unwrap_or(StatusCode::BAD_REQUEST);
            let mut r = (status, Json(&e)).into_response();
            if status == StatusCode::UNAUTHORIZED {
                r.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Basic realm=\"oauth\""));
            }
            r
        }
    };
    // A token response is never stored (RFC 6749 section 5.1).
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

fn authorization(headers: &HeaderMap) -> Option<(String, String)> {
    oauth::basic_of(headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()))
}

async fn token(State(state): State<SharedState>, headers: HeaderMap, Form(form): Form<TokenForm>) -> Response {
    let basic = authorization(&headers);
    let result = tokio::task::spawn_blocking(move || oauth::token(basic, &form, &state)).await;
    oauth_reply(result.unwrap_or_else(|_| Err(OAuthError { error: "server_error", error_description: "The request stopped".to_string(), status: 500 })))
}

#[derive(Deserialize)]
struct RevokeForm {
    #[serde(default)]
    token: String,
    #[serde(default)]
    client_id: Option<String>,
}

async fn revoke(State(state): State<SharedState>, headers: HeaderMap, Form(form): Form<RevokeForm>) -> Response {
    let basic = authorization(&headers);
    let result = tokio::task::spawn_blocking(move || oauth::revoke(basic, form.client_id.as_deref(), &form.token, &state).map(|()| Value::Object(Default::default()))).await;
    oauth_reply(result.unwrap_or_else(|_| Err(OAuthError { error: "server_error", error_description: "The request stopped".to_string(), status: 500 })))
}

async fn list_connections(State(state): State<SharedState>, headers: HeaderMap) -> Reply {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    blocking(move || oauth::connections(&user, &state)).await.map(Json)
}

async fn disconnect(State(state): State<SharedState>, headers: HeaderMap, Path(client_id): Path<String>) -> Reply {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    blocking(move || oauth::disconnect(&user, &client_id, &state)).await.map(Json)
}
