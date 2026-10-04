//! Framework Specification Route
//! Exposes self-documenting JSON Schema and component catalog for AI co-builders and visual inspectors.

use crate::state::SharedState;
use axum::{extract::State, response::IntoResponse, routing::get, Json, Router};
use serde_json::json;

pub fn router() -> Router<SharedState> {
    Router::new().route("/framework/spec", get(get_framework_specification))
}

pub fn build_framework_spec_json() -> serde_json::Value {
    json!({
        "framework_version": "1.0.0",
        "framework_name": "Scaffoldry Governed Application Framework",
        "standards_alignment": {
            "tabular_standard": "TanStack Table v8 Headless Architecture",
            "governance_standard": "NIST OSCAL 1.1.2 & NCES CEDS v11.0",
            "authorization_standard": "Cedar Policy Engine ABAC",
            "identity_standard": "RFC 7643 / RFC 7644 SCIM 2.0"
        },
        "design_system": {
            "approved_palette": [
                { "name": "primary-navy", "hex": "#1e3a8a", "label": "Institutional Navy", "role": "primary" },
                { "name": "academic-emerald", "hex": "#059669", "label": "Academic Emerald", "role": "accent-success" },
                { "name": "compliance-amber", "hex": "#d97706", "label": "Compliance Amber", "role": "accent-warning" },
                { "name": "digital-indigo", "hex": "#4f46e5", "label": "Digital Indigo", "role": "accent-interactive" },
                { "name": "neutral-slate", "hex": "#475569", "label": "Neutral Slate", "role": "neutral" }
            ],
            "typography": {
                "font_family": "Inter, system-ui, sans-serif",
                "scale": ["text-xs", "text-sm", "text-base", "text-lg", "text-xl", "text-2xl"]
            },
            "border_radius": ["rounded-sm", "rounded-md", "rounded-lg", "rounded-xl"],
            "dark_mode_supported": true
        },
        "data_security": {
            "classification_tiers": [
                { "id": "public", "label": "Public Directory", "description": "Unrestricted institutional information" },
                { "id": "internal", "label": "Internal Operational", "description": "Authorized staff and faculty access only" },
                { "id": "restricted", "label": "Restricted FERPA/PII", "description": "Strict Cedar authorization required" }
            ],
            "ferpa_guardrails": "Restricted student and employee data cannot be displayed in widgets without explicit Cedar policy authorization."
        },
        "component_catalog": [
            {
                "type": "stat-metric",
                "title": "KPI Metric Card",
                "description": "Displays a single dynamic aggregated metric with label, trend indicator, and theme accent",
                "layout_slots": ["header", "main"],
                "width_options": ["third", "half", "full"],
                "configurable_props": {
                    "title": { "type": "string", "description": "Card heading" },
                    "subtitle": { "type": "string", "description": "Contextual subtitle" },
                    "aggregation": { "type": "string", "enum": ["count", "sum", "avg"], "description": "Aggregation function" },
                    "target_field": { "type": "string", "description": "Numeric field to aggregate when using sum or avg" },
                    "filter_status": { "type": "string", "description": "Optional status filter predicate" },
                    "accent_color": { "type": "string", "enum": ["blue", "emerald", "amber", "indigo", "slate"] },
                    "trend_text": { "type": "string", "description": "Optional percentage or delta trend label" }
                }
            },
            {
                "type": "tabular-grid",
                "title": "Governed Tabular Grid",
                "description": "Full TanStack Table v8 spreadsheet grid with sorting, search, column visibility, and inline cell editing",
                "layout_slots": ["main"],
                "width_options": ["full", "two-thirds"],
                "configurable_props": {
                    "title": { "type": "string", "description": "Grid title" },
                    "dataset_source": { "type": "string", "description": "Source dataset or internal app records" },
                    "visible_columns": { "type": "array", "items": { "type": "string" }, "description": "TanStack column visibility list" },
                    "enable_search": { "type": "boolean", "description": "Show global text filter" },
                    "enable_inline_edit": { "type": "boolean", "description": "Allow direct cell updates" },
                    "rollup_type": { "type": "string", "enum": ["none", "sum", "avg", "count"], "description": "Column footer aggregation" },
                    "default_sort_column": { "type": "string", "description": "Default sorting column" }
                }
            },
            {
                "type": "kanban-stage",
                "title": "Kanban Stage Board",
                "description": "Card workflow board grouped into stages with drag-and-drop moves and Cedar authorization checks",
                "layout_slots": ["main"],
                "width_options": ["full"],
                "configurable_props": {
                    "title": { "type": "string", "description": "Board title" },
                    "stage_field": { "type": "string", "description": "Field representing stage or status" },
                    "stages": { "type": "array", "items": { "type": "string" }, "description": "Ordered stage columns" },
                    "card_title_field": { "type": "string", "description": "Field to display on card header" },
                    "card_badge_field": { "type": "string", "description": "Field to display as secondary badge" }
                }
            },
            {
                "type": "intake-form",
                "title": "Governed Intake Form",
                "description": "Structured record creation form with field-level CEDS validation and Cedar policy enforcement",
                "layout_slots": ["main", "sidebar"],
                "width_options": ["half", "two-thirds", "full"],
                "configurable_props": {
                    "title": { "type": "string", "description": "Form title" },
                    "submit_button_label": { "type": "string", "description": "Label for submission action" },
                    "included_fields": { "type": "array", "items": { "type": "string" }, "description": "Fields included in the form" },
                    "success_message": { "type": "string", "description": "Notification shown after record creation" }
                }
            },
            {
                "type": "calendar-view",
                "title": "Calendar Timeline View",
                "description": "Date-mapped view plotting deadlines, academic terms, and review dates",
                "layout_slots": ["main"],
                "width_options": ["full"],
                "configurable_props": {
                    "title": { "type": "string", "description": "Calendar title" },
                    "date_field": { "type": "string", "description": "Date field for event placement" },
                    "title_field": { "type": "string", "description": "Field displayed as event title" }
                }
            },
            {
                "type": "rich-banner",
                "title": "Instructional Banner",
                "description": "Institutional notice or policy banner with Markdown formatting",
                "layout_slots": ["header", "main"],
                "width_options": ["full"],
                "configurable_props": {
                    "title": { "type": "string", "description": "Banner heading" },
                    "content": { "type": "string", "description": "Markdown body text" },
                    "variant": { "type": "string", "enum": ["info", "warning", "success"] }
                }
            },
            {
                "type": "action-toolbar",
                "title": "Action Toolbar",
                "description": "Command toolbar with buttons triggering workflow automations, exports, or new records",
                "layout_slots": ["header", "main"],
                "width_options": ["full"],
                "configurable_props": {
                    "actions": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "label": { "type": "string" },
                                "action_type": { "type": "string", "enum": ["create-record", "trigger-automation", "export-oscal"] },
                                "variant": { "type": "string", "enum": ["primary", "secondary", "danger"] }
                            }
                        }
                    }
                }
            }
        ]
    })
}

async fn get_framework_specification(
    State(_state): State<SharedState>,
) -> impl IntoResponse {
    Json(build_framework_spec_json())
}
