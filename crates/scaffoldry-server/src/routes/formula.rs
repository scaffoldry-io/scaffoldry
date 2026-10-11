//! Formula preview endpoint with rate limiting

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use crate::guard::session_user;
use crate::service::access::{authorize_app, AppAction};
use crate::service::ServiceError;
use crate::state::SharedState;

#[derive(Debug, Deserialize)]
pub struct FormulaPreviewRequest {
    pub expression: String,
    pub table_id: String,
    pub app_slug: String,
    pub record: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct FormulaProblem {
    pub message: String,
    pub position: usize,
}

#[derive(Debug, Serialize)]
pub struct FormulaPreviewResponse {
    pub valid: bool,
    pub result: Value,
    pub problems: Vec<FormulaProblem>,
    pub error: Option<String>,
    pub error_pos: Option<usize>,
}

pub fn router() -> Router<SharedState> {
    Router::new().route("/formula/preview", post(preview_formula))
}

pub async fn preview_formula(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<FormulaPreviewRequest>,
) -> Result<Json<FormulaPreviewResponse>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    authorize_app(&user, &payload.app_slug, AppAction::Read, &state)
        .map_err(ServiceError::into_pair)?;

    // Rate-limited to 20 a second per person
    if !state.check_formula_rate_limit(&user.eppn) {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "code": "rate_limit_exceeded",
                "message": "Rate limit exceeded: maximum 20 formula previews per second"
            })),
        ));
    }

    match scaffoldry_engine::validate_formula(&payload.expression) {
        Err(err) => Ok(Json(FormulaPreviewResponse {
            valid: false,
            result: Value::Null,
            problems: vec![FormulaProblem {
                message: err.message.clone(),
                position: err.position,
            }],
            error: Some(err.message),
            error_pos: Some(err.position),
        })),
        Ok(_) => {
            let record = match payload.record {
                Some(r) => r,
                None => {
                    let mut sample = json!({});
                    use scaffoldry_engine::HostRouter;
                    if let Ok(engine_guard) = state.engine.read() {
                        if let Some(manifest) = engine_guard.resolve_by_slug(&payload.app_slug) {
                            if let Some(table) = manifest.tables.iter().find(|t| t.id == payload.table_id) {
                                if let Some(first_sample) = table.sample_records.first() {
                                    sample = first_sample.clone();
                                }
                            }
                        }
                    }
                    sample
                }
            };

            let eval_result = scaffoldry_engine::evaluate_formula(&payload.expression, &record);
            Ok(Json(FormulaPreviewResponse {
                valid: true,
                result: eval_result,
                problems: vec![],
                error: None,
                error_pos: None,
            }))
        }
    }
}
