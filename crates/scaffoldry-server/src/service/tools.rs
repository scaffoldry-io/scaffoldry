//! The tool registry and its one gate.
//!
//! Every MCP tool is an entry in `TOOLS`. A tool does not sign anyone in, check who the caller
//! may act on, or check its arguments. `call` does that, the same way for every tool, and then
//! runs the tool. A tool that is added later cannot skip a step, because the tool does not
//! perform the step.

use crate::service::access::{authorize_app, authorize_workspace, AppAction};
use crate::service::{records, workspaces, ServiceError};
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::standards::eduperson::{EduPersonAffiliation, EduPersonIdentity};
use scaffoldry_core::DecisionType;
use scaffoldry_policy::PolicyDecision;
use serde_json::{json, Map, Value};

/// What a caller must be allowed to do before a tool runs.
#[derive(Debug, Clone, Copy)]
pub enum Scope {
    /// Any authenticated caller.
    SignedIn,
    /// Reads `app_slug` from the arguments and checks the caller's right to the app.
    App(AppAction),
    /// Reads `workspace_id` from the arguments and checks this Cedar action on that workspace.
    Workspace(&'static str),
    /// A Platform Admin only.
    PlatformAdmin,
}

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: fn() -> Value,
    pub scope: Scope,
    /// True when the tool changes nothing. It becomes the `readOnlyHint` annotation.
    pub read_only: bool,
    pub run: fn(&AuthUser, Value, &SharedState) -> Result<Value, ServiceError>,
}

pub static TOOLS: &[Tool] = &[
    Tool {
        name: "list_workspaces",
        description: "List all sovereign workspaces accessible to the authenticated principal under Cedar ABAC policies.",
        input_schema: schema_list_workspaces,
        scope: Scope::SignedIn,
        read_only: true,
        run: run_list_workspaces,
    },
    Tool {
        name: "get_workspace",
        description: "Retrieve workspace configuration and collaborator access list by ID (Cedar ABAC enforced).",
        input_schema: schema_get_workspace,
        scope: Scope::Workspace("access_workspace"),
        read_only: true,
        run: run_get_workspace,
    },
    Tool {
        name: "update_workspace",
        description: "Update workspace metadata, visibility, classification, and policies (Owner/Admin Cedar ABAC enforced).",
        input_schema: schema_update_workspace,
        scope: Scope::Workspace("manage_workspace"),
        read_only: false,
        run: run_update_workspace,
    },
    Tool {
        name: "manage_workspace_member",
        description: "Add, update role, or remove workspace collaborators with audit logging (Owner/Admin Cedar ABAC enforced).",
        input_schema: schema_manage_workspace_member,
        scope: Scope::Workspace("manage_workspace"),
        read_only: false,
        run: run_manage_workspace_member,
    },
    Tool {
        name: "create_record",
        description: "Submit a new tabular record into an application (departmental membership & Cedar ABAC enforced).",
        input_schema: schema_create_record,
        scope: Scope::App(AppAction::WriteRecords),
        read_only: false,
        run: run_create_record,
    },
    Tool {
        name: "list_datasets",
        description: "List all published institutional datasets with field schemas, department ownership, and FERPA classifications.",
        input_schema: schema_list_datasets,
        scope: Scope::SignedIn,
        read_only: true,
        run: run_list_datasets,
    },
    Tool {
        name: "simulate_cedar_policy",
        description: "Ask the institutional Cedar policy engine for its decision on an action over a record. Platform Admin only. Changes nothing.",
        input_schema: schema_simulate_cedar_policy,
        scope: Scope::PlatformAdmin,
        read_only: true,
        run: run_simulate_cedar_policy,
    },
    Tool {
        name: "record_governance_decision",
        description: "Append a governance decision to the immutable SHA-256 cryptographic ledger with NIST OSCAL control mapping. The decision is recorded in the name of the signed-in caller.",
        input_schema: schema_record_governance_decision,
        scope: Scope::SignedIn,
        read_only: false,
        run: run_record_governance_decision,
    },
    Tool {
        name: "verify_decision_ledger",
        description: "Verify cryptographic integrity and SHA-256 block chain linkage of all recorded governance decisions from genesis.",
        input_schema: schema_empty,
        scope: Scope::SignedIn,
        read_only: true,
        run: run_verify_decision_ledger,
    },
    Tool {
        name: "export_oscal_compliance",
        description: "Generate and export official NIST OSCAL 1.1.2 JSON component-definition with full cryptographic audit proofs.",
        input_schema: schema_empty,
        scope: Scope::SignedIn,
        read_only: true,
        run: run_export_oscal_compliance,
    },
    Tool {
        name: "get_framework_spec",
        description: "Retrieve the self-documenting JSON Schema specification for the TanStack-extended component catalog, approved institutional theme tokens, and data classification boundaries.",
        input_schema: schema_empty,
        scope: Scope::SignedIn,
        read_only: true,
        run: run_get_framework_spec,
    },
];

/// The one gate. Finds the tool, checks its scope, checks the arguments, and runs it.
pub fn call(caller: &AuthUser, name: &str, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let tool = TOOLS
        .iter()
        .find(|t| t.name == name)
        .ok_or_else(|| ServiceError::NotFound(format!("Tool '{name}' is not registered")))?;
    let args = if args.is_null() { json!({}) } else { args };

    match tool.scope {
        Scope::SignedIn => {}
        Scope::App(action) => {
            let slug = required_str(&args, "app_slug")?;
            authorize_app(caller, slug, action, state)?;
        }
        Scope::Workspace(cedar_action) => {
            let ws_id = required_str(&args, "workspace_id")?;
            authorize_workspace(caller, ws_id, cedar_action, state)?;
        }
        Scope::PlatformAdmin => {
            crate::service::admin::require_platform_admin(caller, state)?;
        }
    }

    validate_args(&(tool.input_schema)(), &args)?;
    (tool.run)(caller, args, state)
}

fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ServiceError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ServiceError::BadRequest(format!("Argument '{key}' is required")))
}

/// Checks the arguments against the tool's own schema: an object, no unknown names, every
/// required name present, and each value of the type the schema gives.
fn validate_args(schema: &Value, args: &Value) -> Result<(), ServiceError> {
    let given = args
        .as_object()
        .ok_or_else(|| ServiceError::BadRequest("Arguments must be an object".to_string()))?;
    let empty = Map::new();
    let properties = schema["properties"].as_object().unwrap_or(&empty);

    for key in given.keys() {
        if !properties.contains_key(key) {
            let allowed: Vec<&str> = properties.keys().map(String::as_str).collect();
            let hint = if allowed.is_empty() {
                "This tool takes no arguments".to_string()
            } else {
                format!("Allowed: {}", allowed.join(", "))
            };
            return Err(ServiceError::BadRequest(format!("Unknown argument '{key}'. {hint}")));
        }
    }
    if let Some(required) = schema["required"].as_array() {
        for name in required.iter().filter_map(|v| v.as_str()) {
            if given.get(name).is_none_or(Value::is_null) {
                return Err(ServiceError::BadRequest(format!("Argument '{name}' is required")));
            }
        }
    }
    for (key, value) in given {
        let Some(kind) = properties.get(key).and_then(|p| p["type"].as_str()) else {
            continue;
        };
        let fits = match kind {
            "string" => value.is_string(),
            "object" => value.is_object(),
            "array" => value.is_array(),
            "boolean" => value.is_boolean(),
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            _ => true,
        };
        if !fits {
            return Err(ServiceError::BadRequest(format!("Argument '{key}' must be of type {kind}")));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Schemas
// ---------------------------------------------------------------------------------------------

fn schema_empty() -> Value {
    json!({ "type": "object", "properties": {} })
}

fn schema_list_workspaces() -> Value {
    schema_empty()
}

fn schema_get_workspace() -> Value {
    json!({
        "type": "object",
        "properties": {
            "workspace_id": { "type": "string", "description": "Unique workspace identifier" }
        },
        "required": ["workspace_id"]
    })
}

fn schema_update_workspace() -> Value {
    json!({
        "type": "object",
        "properties": {
            "workspace_id": { "type": "string", "description": "Unique workspace identifier" },
            "name": { "type": "string" },
            "description": { "type": "string" },
            "department": { "type": "string" },
            "visibility": { "type": "string", "description": "restricted, departmental, or institutional" },
            "allowed_affiliations": { "type": "array", "items": { "type": "string" } },
            "data_classification": { "type": "string" },
            "icon": { "type": "string" },
            "cedar_policy_guard": { "type": "string" }
        },
        "required": ["workspace_id"]
    })
}

fn schema_manage_workspace_member() -> Value {
    json!({
        "type": "object",
        "properties": {
            "workspace_id": { "type": "string", "description": "Unique workspace identifier" },
            "action": { "type": "string", "description": "add, update, or remove" },
            "eppn": { "type": "string", "description": "eduPersonPrincipalName of collaborator" },
            "role": { "type": "string", "description": "owner, admin, editor, or viewer" },
            "name": { "type": "string", "description": "Full display name" }
        },
        "required": ["workspace_id", "action", "eppn"]
    })
}

fn schema_create_record() -> Value {
    json!({
        "type": "object",
        "properties": {
            "app_slug": { "type": "string", "description": "Application slug identifier" },
            "data": { "type": "object", "description": "Record field key-value pairs" }
        },
        "required": ["app_slug", "data"]
    })
}

fn schema_list_datasets() -> Value {
    json!({
        "type": "object",
        "properties": {
            "department": {
                "type": "string",
                "description": "Optional department filter (e.g., 'Academic Affairs', 'Research Administration')"
            }
        }
    })
}

fn schema_simulate_cedar_policy() -> Value {
    json!({
        "type": "object",
        "properties": {
            "eppn": { "type": "string", "description": "Principal eduPersonPrincipalName" },
            "affiliation": { "type": "string", "description": "Affiliation: faculty, student, staff, employee, member, affiliate, or alum" },
            "action": { "type": "string", "description": "Cedar action, such as read, write, export, or approve" },
            "ferpa_sensitive": { "type": "boolean", "description": "Whether the record is FERPA sensitive" },
            "department": { "type": "string", "description": "Department of the principal and of the record. Defaults to the first label of the eppn's domain" }
        },
        "required": ["eppn", "action"]
    })
}

fn schema_record_governance_decision() -> Value {
    json!({
        "type": "object",
        "properties": {
            "organization_code": { "type": "string", "description": "Institutional unit code" },
            "app_slug": { "type": "string", "description": "Optional application slug" },
            "decision_type": { "type": "string", "description": "Decision classification (AppPublished, VanityDnsBound, PolicyRevision, WorkflowRuleApproved, AccessRoleGranted, DatasetAccessShared, StatutoryAttestation)" },
            "oscal_control_id": { "type": "string", "description": "NIST SP 800-53 / OSCAL control (e.g. AC-03, CM-03, AU-02)" },
            "rationale": { "type": "string", "description": "Institutional justification and review findings" }
        },
        "required": ["organization_code", "decision_type", "oscal_control_id", "rationale"]
    })
}

// ---------------------------------------------------------------------------------------------
// Tools. Each one does its work and nothing else. The gate has already run.
// ---------------------------------------------------------------------------------------------

fn text(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

fn to_value<T: serde::Serialize>(v: &T) -> Result<Value, ServiceError> {
    serde_json::to_value(v).map_err(|e| ServiceError::Internal(e.to_string()))
}

fn run_list_workspaces(caller: &AuthUser, _args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    to_value(&workspaces::list_workspaces(caller, state)?)
}

fn run_get_workspace(caller: &AuthUser, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let ws_id = required_str(&args, "workspace_id")?;
    to_value(&workspaces::get_workspace(caller, ws_id, state)?)
}

fn run_update_workspace(caller: &AuthUser, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let ws_id = required_str(&args, "workspace_id")?.to_string();
    let input = workspaces::UpdateWorkspacePayload {
        name: text(&args, "name"),
        description: text(&args, "description"),
        department: text(&args, "department"),
        visibility: text(&args, "visibility"),
        allowed_affiliations: args
            .get("allowed_affiliations")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()),
        data_classification: text(&args, "data_classification"),
        icon: text(&args, "icon"),
        cedar_policy_guard: text(&args, "cedar_policy_guard"),
    };
    to_value(&workspaces::update_workspace(caller, &ws_id, input, state)?)
}

fn run_manage_workspace_member(caller: &AuthUser, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let ws_id = required_str(&args, "workspace_id")?;
    let action = required_str(&args, "action")?;
    let eppn = required_str(&args, "eppn")?;
    let role = args.get("role").and_then(|v| v.as_str());
    let name = args.get("name").and_then(|v| v.as_str());
    workspaces::manage_workspace_member(caller, ws_id, action, eppn, role, name, state)
}

fn run_create_record(caller: &AuthUser, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let app_slug = required_str(&args, "app_slug")?;
    let data = args.get("data").cloned().unwrap_or_else(|| json!({}));
    to_value(&records::create_record(caller, app_slug, &data, state)?)
}

fn run_list_datasets(_caller: &AuthUser, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let datasets = state.datasets.read().unwrap_or_else(|p| p.into_inner());
    let dept_filter = args.get("department").and_then(|v| v.as_str());
    let list: Vec<_> = datasets
        .values()
        .filter(|d| dept_filter.is_none_or(|f| d.department.eq_ignore_ascii_case(f)))
        .cloned()
        .collect();
    to_value(&list)
}

fn run_simulate_cedar_policy(_caller: &AuthUser, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let eppn = required_str(&args, "eppn")?;
    let action = required_str(&args, "action")?;
    let affiliation_word = text(&args, "affiliation").unwrap_or_else(|| "staff".to_string());
    // Accept "faculty" and the scoped form "faculty@university.edu".
    let affiliation_word = affiliation_word.split('@').next().unwrap_or("staff").to_string();
    let ferpa_sensitive = args.get("ferpa_sensitive").and_then(|v| v.as_bool()).unwrap_or(false);

    let domain = eppn.split('@').nth(1).unwrap_or("university.edu").to_string();
    let department = text(&args, "department")
        .unwrap_or_else(|| domain.split('.').next().unwrap_or("university").to_string());

    let affiliation = match affiliation_word.as_str() {
        "faculty" => EduPersonAffiliation::Faculty,
        "student" => EduPersonAffiliation::Student,
        "staff" => EduPersonAffiliation::Staff,
        "employee" => EduPersonAffiliation::Employee,
        "affiliate" => EduPersonAffiliation::Affiliate,
        "alum" => EduPersonAffiliation::Alum,
        _ => EduPersonAffiliation::Member,
    };
    let identity = EduPersonIdentity {
        eppn: eppn.to_string(),
        realm: domain,
        affiliations: vec![affiliation],
    };

    let result = state
        .policy_engine
        .authorize_record_action(&identity, &department, action, "simulation", &department, ferpa_sensitive)
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    Ok(json!({
        "decision": if result.decision == PolicyDecision::Allow { "Allow" } else { "Deny" },
        "principal": eppn,
        "affiliation": affiliation_word,
        "action": action,
        "resource_ferpa_sensitive": ferpa_sensitive,
        "reasons": result.reasons,
        "diagnostics": result.diagnostics,
        "deciding_policy": result.deciding_policy,
    }))
}

fn run_record_governance_decision(caller: &AuthUser, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    // The same Cedar rule the REST route applies. A student may not record a decision.
    let auth = state
        .policy_engine
        .authorize_institutional_action(
            &caller.eppn,
            &caller.affiliation,
            &caller.department,
            "record_decision",
            "governance-ledger",
        )
        .map_err(|e| ServiceError::Internal(e.to_string()))?;
    if auth.decision == PolicyDecision::Deny {
        return Err(ServiceError::Forbidden {
            message: "Forbidden: Cedar policy denied ledger append".to_string(),
            reasons: auth.reasons,
            diagnostics: auth.diagnostics,
            policy: auth.deciding_policy,
        });
    }

    let org_code = required_str(&args, "organization_code")?;
    let decision_type = match required_str(&args, "decision_type")? {
        "AppPublished" => DecisionType::AppPublished,
        "VanityDnsBound" => DecisionType::VanityDnsBound,
        "PolicyRevision" => DecisionType::PolicyRevision,
        "WorkflowRuleApproved" => DecisionType::WorkflowRuleApproved,
        "AccessRoleGranted" => DecisionType::AccessRoleGranted,
        "DatasetAccessShared" => DecisionType::DatasetAccessShared,
        "StatutoryAttestation" => DecisionType::StatutoryAttestation,
        _ => DecisionType::PolicyRevision,
    };
    let payload = json!({ "recorded_via": "mcp_jsonrpc", "tool": "record_governance_decision" });
    let entry = state
        .append_ledger_entry(crate::state::RecordDecisionInput {
            // Identity comes from the session, never from an argument.
            principal: caller.eppn.clone(),
            organization_code: org_code.to_string(),
            app_slug: text(&args, "app_slug"),
            decision_type,
            oscal_control_id: required_str(&args, "oscal_control_id")?.to_string(),
            rationale: required_str(&args, "rationale")?.to_string(),
            payload: &payload,
        })
        .map_err(|e| ServiceError::Internal(format!("Failed to append ledger entry: {e}")))?;

    Ok(json!({
        "status": "RECORDED",
        "entry_hash": entry.entry_hash,
        "sequence": entry.sequence,
        "oscal_control": entry.oscal_control_id,
    }))
}

fn run_verify_decision_ledger(_caller: &AuthUser, _args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    crate::service::governance::verify_decision_ledger(state)
}

fn run_export_oscal_compliance(_caller: &AuthUser, _args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    Ok(state.export_oscal_component_definition())
}

fn run_get_framework_spec(_caller: &AuthUser, _args: Value, _state: &SharedState) -> Result<Value, ServiceError> {
    Ok(crate::routes::framework::build_framework_spec_json())
}
