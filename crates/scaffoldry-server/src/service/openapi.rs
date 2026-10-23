//! An API description for each app (integrations phase 3).
//!
//! `describe` builds an OpenAPI 3.1 document for one caller. It lists the tables the caller can read, with one JSON
//! Schema per table built from its fields. A field hidden from this caller is left out, so the document never reveals
//! it. The REST route and the MCP resource `scaffoldry://apps/{slug}/openapi` both call it.

use crate::service::access::{authorize_app, AppAction};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_engine::{FieldSpec, FieldType, HostRouter};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

/// The URI an agent reads to get the document.
pub fn resource_uri(slug: &str) -> String {
    format!("scaffoldry://apps/{slug}/openapi")
}

/// The slug in a resource URI such as `scaffoldry://apps/admissions/openapi`.
pub fn slug_of_resource(uri: &str) -> Option<&str> {
    uri.strip_prefix("scaffoldry://apps/")?.strip_suffix("/openapi").filter(|s| !s.is_empty() && !s.contains('/'))
}

/// The JSON Schema of one field's value.
fn field_schema(f: &FieldSpec) -> Value {
    let mut s = Map::new();
    let kind = |s: &mut Map<String, Value>, t: &str| {
        s.insert("type".into(), json!(t));
    };
    let ids: Vec<&str> = f.select_options.iter().map(|o| o.id.as_str()).collect();
    let labels: Map<String, Value> = f.select_options.iter().map(|o| (o.id.clone(), json!(o.label))).collect();
    match &f.field_type {
        FieldType::Text | FieldType::LongText | FieldType::Relation | FieldType::Unit => kind(&mut s, "string"),
        FieldType::Email => {
            kind(&mut s, "string");
            s.insert("format".into(), json!("email"));
        }
        FieldType::Url => {
            kind(&mut s, "string");
            s.insert("format".into(), json!("uri"));
        }
        FieldType::Phone => kind(&mut s, "string"),
        FieldType::Number | FieldType::Currency | FieldType::Percent | FieldType::Rating | FieldType::Duration { .. } | FieldType::Count => kind(&mut s, "number"),
        FieldType::Autonumber => kind(&mut s, "integer"),
        FieldType::Boolean | FieldType::Checkbox => kind(&mut s, "boolean"),
        FieldType::Date => {
            kind(&mut s, "string");
            s.insert("format".into(), json!("date"));
        }
        FieldType::DateTime | FieldType::CreatedTime | FieldType::LastModifiedTime => {
            kind(&mut s, "string");
            s.insert("format".into(), json!("date-time"));
        }
        FieldType::Select => {
            kind(&mut s, "string");
            s.insert("enum".into(), json!(ids));
            s.insert("x-labels".into(), Value::Object(labels));
        }
        FieldType::MultiSelect => {
            kind(&mut s, "array");
            s.insert("items".into(), json!({ "type": "string", "enum": ids }));
            s.insert("uniqueItems".into(), json!(true));
            s.insert("x-labels".into(), Value::Object(labels));
        }
        FieldType::Link => {
            kind(&mut s, "array");
            s.insert("items".into(), json!({ "type": "string" }));
            s.insert("description".into(), json!("Ids of records in the linked table."));
        }
        FieldType::Collaborator { allow_multiple } => {
            if *allow_multiple {
                kind(&mut s, "array");
                s.insert("items".into(), json!({ "type": "string" }));
            } else {
                kind(&mut s, "string");
            }
        }
        FieldType::CreatedBy | FieldType::LastModifiedBy => kind(&mut s, "string"),
        FieldType::Attachment { .. } => {
            kind(&mut s, "array");
            s.insert(
                "items".into(),
                json!({ "type": "object", "properties": { "ref_id": { "type": "string" }, "name": { "type": "string" } }, "required": ["ref_id", "name"] }),
            );
        }
        // The value follows from other values, and its type depends on them.
        FieldType::Lookup | FieldType::Rollup | FieldType::Formula => {}
    }
    s.insert("title".into(), json!(f.label));
    if let Some(c) = &f.constraints {
        if matches!(s.get("type").and_then(Value::as_str), Some("number" | "integer")) {
            if let Some(min) = c.min {
                s.insert("minimum".into(), json!(min));
            }
            if let Some(max) = c.max {
                s.insert("maximum".into(), json!(max));
            }
        }
    }
    if f.field_type.is_computed() {
        s.insert("readOnly".into(), json!(true));
    }
    Value::Object(s)
}

/// The key a table's schema has under `components.schemas`.
fn schema_name(table_id: &str) -> String {
    table_id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' { c } else { '_' }).collect()
}

fn table_schema(table: &scaffoldry_engine::AppTable) -> Value {
    let properties: Map<String, Value> = table.fields.iter().map(|f| (f.name.clone(), field_schema(f))).collect();
    let required: Vec<&str> = table
        .fields
        .iter()
        .filter(|f| !f.field_type.is_computed() && (f.required || f.constraints.as_ref().is_some_and(|c| c.required)))
        .map(|f| f.name.as_str())
        .collect();
    let mut schema = json!({ "type": "object", "title": table.name, "properties": properties, "additionalProperties": false });
    if !required.is_empty() {
        schema["required"] = json!(required);
    }
    schema
}

fn record_of(name: &str) -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string" },
            "version": { "type": "integer" },
            "data": { "$ref": format!("#/components/schemas/{name}") }
        },
        "required": ["id", "version", "data"]
    })
}

fn error_response(description: &str) -> Value {
    json!({ "description": description, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Error" } } } })
}

fn paths_of(slug: &str, table: &scaffoldry_engine::AppTable) -> Vec<(String, Value)> {
    let name = schema_name(&table.id);
    let base = format!("/apps/{slug}/tables/{}", table.id);
    let record = format!("#/components/schemas/{name}Record");
    let data = format!("#/components/schemas/{name}");
    let id_param = json!({ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } });
    let list = json!({
        "operationId": format!("list_{name}"),
        "summary": format!("List {}", table.name),
        "parameters": [
            { "name": "limit", "in": "query", "schema": { "type": "integer", "minimum": 1 } },
            { "name": "cursor", "in": "query", "schema": { "type": "string" } }
        ],
        "responses": {
            "200": { "description": "A page of records.", "content": { "application/json": { "schema": {
                "type": "object",
                "properties": { "records": { "type": "array", "items": { "$ref": record } }, "next_cursor": { "type": ["string", "null"] } },
                "required": ["records"]
            } } } },
            "403": error_response("The caller may not read this table.")
        }
    });
    let create = json!({
        "operationId": format!("create_{name}"),
        "summary": format!("Add a record to {}", table.name),
        "requestBody": { "required": true, "content": { "application/json": { "schema": {
            "type": "object", "properties": { "data": { "$ref": data } }, "required": ["data"]
        } } } },
        "responses": {
            "201": { "description": "The record that was added.", "content": { "application/json": { "schema": { "$ref": record } } } },
            "400": error_response("The record is not valid."),
            "403": error_response("The caller may not add records here.")
        }
    });
    let get = json!({
        "operationId": format!("get_{name}"),
        "summary": format!("Read one record of {}", table.name),
        "parameters": [id_param.clone()],
        "responses": {
            "200": { "description": "The record.", "content": { "application/json": { "schema": { "$ref": record } } } },
            "404": error_response("No such record, or the caller may not see it.")
        }
    });
    let update = json!({
        "operationId": format!("update_{name}"),
        "summary": format!("Change a record of {}", table.name),
        "parameters": [id_param.clone()],
        "requestBody": { "required": true, "content": { "application/json": { "schema": {
            "type": "object", "properties": { "data": { "$ref": data }, "version": { "type": "integer" } }, "required": ["data", "version"]
        } } } },
        "responses": {
            "200": { "description": "The record after the change.", "content": { "application/json": { "schema": { "$ref": record } } } },
            "409": error_response("The record changed since this version.")
        }
    });
    let delete = json!({
        "operationId": format!("delete_{name}"),
        "summary": format!("Delete a record of {}", table.name),
        "parameters": [id_param, { "name": "version", "in": "query", "schema": { "type": "integer" } }],
        "responses": { "200": { "description": "The record was deleted." }, "409": error_response("The record changed since this version.") }
    });
    let aggregate = json!({
        "operationId": format!("aggregate_{name}"),
        "summary": format!("Count or total {}", table.name),
        "parameters": [
            { "name": "function", "in": "query", "schema": { "type": "string" } },
            { "name": "field", "in": "query", "schema": { "type": "string" } },
            { "name": "group_by", "in": "query", "schema": { "type": "string" } }
        ],
        "responses": { "200": { "description": "The result.", "content": { "application/json": { "schema": { "type": "object" } } } } }
    });
    vec![
        (format!("{base}/records"), json!({ "get": list, "post": create })),
        (format!("{base}/records/{{id}}"), json!({ "get": get, "patch": update, "delete": delete })),
        (format!("{base}/aggregate"), json!({ "get": aggregate })),
    ]
}

/// The document for `caller`. Tables are the app's own; a field the caller may not see is not in them.
pub fn describe(caller: &AuthUser, slug: &str, state: &SharedState) -> Result<Value, ServiceError> {
    authorize_app(caller, slug, AppAction::Read, state)?;
    let (mut manifest, version) = {
        let engine = state.engine.read().map_err(|e| ServiceError::Internal(e.to_string()))?;
        let manifest = engine.resolve_by_slug(slug).cloned().ok_or_else(|| ServiceError::not_found("App not found"))?;
        (manifest, engine.versions().get(slug).copied().unwrap_or(1))
    };
    crate::service::records::without_hidden_fields(caller, slug, &mut manifest, state);

    let mut paths = Map::new();
    let mut schemas = Map::new();
    schemas.insert(
        "Error".into(),
        json!({ "type": "object", "properties": { "error": { "type": "string" }, "code": { "type": "string" } }, "required": ["error", "code"] }),
    );
    for table in &manifest.tables {
        let name = schema_name(&table.id);
        schemas.insert(name.clone(), table_schema(table));
        schemas.insert(format!("{name}Record"), record_of(&name));
        for (path, item) in paths_of(slug, table) {
            paths.insert(path, item);
        }
    }
    Ok(json!({
        "openapi": "3.1.0",
        "info": { "title": manifest.title, "description": manifest.description, "version": version.to_string() },
        "servers": [{ "url": "/api/v1" }],
        "security": [{ "bearerAuth": [] }],
        "paths": paths,
        "components": {
            "securitySchemes": { "bearerAuth": { "type": "http", "scheme": "bearer" } },
            "schemas": schemas
        }
    }))
}

/// The entity tag of a document: a hash of its content, so it changes when the app does and differs per caller.
pub fn etag_of(document: &Value) -> String {
    let bytes = serde_json::to_vec(document).unwrap_or_default();
    let hex: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    format!("\"{hex}\"")
}
