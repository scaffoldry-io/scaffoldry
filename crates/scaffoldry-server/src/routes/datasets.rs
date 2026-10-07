use crate::guard::session_user;
use crate::state::SharedState;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use chrono::Utc;
use scaffoldry_core::{DatasetField, DatasetRelationship, PublishedDataset, RelationshipType};
use scaffoldry_policy::PolicyDecision;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/datasets", get(list_datasets).post(publish_dataset))
        .route("/datasets/{id}", get(get_dataset))
        .route(
            "/datasets/{id}/relationships",
            get(list_relationships).post(create_relationship),
        )
}

async fn list_datasets(State(state): State<SharedState>) -> impl IntoResponse {
    let datasets = state.datasets.read().unwrap();
    let rels = state.relationships.read().unwrap();

    let mut list = Vec::new();
    for ds in datasets.values() {
        let rel_count = rels
            .values()
            .filter(|r| r.source_dataset_id == ds.id || r.target_dataset_id == ds.id)
            .count();

        list.push(json!({
            "id": ds.id,
            "name": ds.name,
            "description": ds.description,
            "department": ds.department,
            "organization": ds.organization,
            "sensitivity_level": ds.sensitivity_level,
            "herm_capability_id": ds.herm_capability_id,
            "fields": ds.fields,
            "record_count": ds.record_count,
            "published_at": ds.published_at,
            "relationship_count": rel_count,
            "sample_data": ds.sample_data,
        }));
    }

    Json(json!({
        "total": list.len(),
        "datasets": list
    }))
}

async fn get_dataset(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let datasets = state.datasets.read().unwrap();
    let ds = datasets.get(&id).ok_or(StatusCode::NOT_FOUND)?;
    let rels = state.relationships.read().unwrap();

    let related_rels: Vec<DatasetRelationship> = rels
        .values()
        .filter(|r| r.source_dataset_id == id || r.target_dataset_id == id)
        .cloned()
        .collect();

    Ok(Json(json!({
        "id": ds.id,
        "name": ds.name,
        "description": ds.description,
        "department": ds.department,
        "organization": ds.organization,
        "sensitivity_level": ds.sensitivity_level,
        "herm_capability_id": ds.herm_capability_id,
        "fields": ds.fields,
        "record_count": ds.record_count,
        "published_at": ds.published_at,
        "relationships": related_rels,
        "sample_data": ds.sample_data,
    })))
}

async fn publish_dataset(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<PublishedDataset>), (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(json!({"error": "Unauthorized"}))))?;

    let name = payload["name"]
        .as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(json!({"error": "name is required"}))))?
        .to_string();

    let id = payload
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| name.to_lowercase().replace(' ', "_"));

    let description = payload
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let department = payload
        .get("department")
        .and_then(|v| v.as_str())
        .unwrap_or("Academic Department")
        .to_string();

    let auth = state
        .policy_engine
        .authorize_departmental_action(
            &user.eppn,
            &user.affiliation,
            &user.department,
            "publish_dataset",
            &id,
            &department,
        )
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    if auth.decision == PolicyDecision::Deny {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "Forbidden: Cedar policy denied dataset publishing"})),
        ));
    }

    let organization = payload
        .get("organization")
        .and_then(|v| v.as_str())
        .unwrap_or("University")
        .to_string();

    let sensitivity = payload
        .get("sensitivity_level")
        .and_then(|v| v.as_str())
        .unwrap_or("Directory")
        .to_string();

    let herm_id = payload
        .get("herm_capability_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let fields: Vec<DatasetField> = payload
        .get("fields")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let sample_data = payload
        .get("sample_data")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let record_count = payload
        .get("record_count")
        .and_then(|v| v.as_u64())
        .map(|c| c as usize)
        .unwrap_or(sample_data.len());

    let dataset = PublishedDataset {
        id: id.clone(),
        name,
        description,
        department,
        organization,
        sensitivity_level: sensitivity,
        herm_capability_id: herm_id,
        fields,
        record_count,
        published_at: Utc::now().to_rfc3339(),
        sample_data,
    };

    state.datasets.write().unwrap().insert(id, dataset.clone());
    Ok((StatusCode::CREATED, Json(dataset)))
}

async fn list_relationships(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let rels = state.relationships.read().unwrap();
    let matching: Vec<DatasetRelationship> = rels
        .values()
        .filter(|r| r.source_dataset_id == id || r.target_dataset_id == id)
        .cloned()
        .collect();

    Json(json!({
        "dataset_id": id,
        "total": matching.len(),
        "relationships": matching
    }))
}

async fn create_relationship(
    State(state): State<SharedState>,
    Path(source_id): Path<String>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<DatasetRelationship>), (StatusCode, Json<Value>)> {
    let target_dataset_id = payload["target_dataset_id"]
        .as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(json!({"error": "target_dataset_id is required"}))))?
        .to_string();

    let source_field = payload["source_field"]
        .as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(json!({"error": "source_field is required"}))))?
        .to_string();

    let target_field = payload["target_field"]
        .as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(json!({"error": "target_field is required"}))))?
        .to_string();

    let name = payload
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Relational Lookup")
        .to_string();

    let display_field = payload
        .get("display_field")
        .and_then(|v| v.as_str())
        .unwrap_or(&target_field)
        .to_string();

    let rel_type_str = payload
        .get("relationship_type")
        .and_then(|v| v.as_str())
        .unwrap_or("OneToMany");

    let rel_type = match rel_type_str {
        "OneToOne" => RelationshipType::OneToOne,
        "ManyToMany" => RelationshipType::ManyToMany,
        _ => RelationshipType::OneToMany,
    };

    let id = format!("rel_{}", Uuid::new_v4().simple());
    let rel = DatasetRelationship {
        id: id.clone(),
        name,
        source_dataset_id: source_id,
        target_dataset_id,
        source_field,
        target_field,
        relationship_type: rel_type,
        display_field,
    };

    state.relationships.write().unwrap().insert(id, rel.clone());
    Ok((StatusCode::CREATED, Json(rel)))
}
