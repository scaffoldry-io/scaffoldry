//! The tool registry and its one gate.
//!
//! Every MCP tool is an entry in `TOOLS`. A tool does not sign anyone in, check who the caller
//! may act on, or check its arguments. `call` does that, the same way for every tool, and then
//! runs the tool. A tool that is added later cannot skip a step, because the tool does not
//! perform the step.

use crate::service::access::{authorize_app, authorize_workspace, AppAction};
use crate::service::history::Origin;
use crate::service::{records, workspaces, ServiceError};
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::DecisionType;
use scaffoldry_engine::HostRouter;
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
    /// Reads `proposal_id` from the arguments and checks this Cedar action on the workspace the
    /// proposal belongs to.
    Proposal(&'static str),
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
    pub run: fn(&AuthUser, &Origin, Value, &SharedState) -> Result<Value, ServiceError>,
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
        description: "Append a governance decision to the SHA-256 ledger, mapped to a NIST OSCAL control. It is recorded in the name of the signed-in caller.",
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
        description: "Retrieve the JSON Schema of the component catalog, the approved institutional theme tokens, and the data classification boundaries.",
        input_schema: schema_empty,
        scope: Scope::SignedIn,
        read_only: true,
        run: run_get_framework_spec,
    },
    Tool {
        name: "list_apps",
        description: "List the apps in a workspace: slug, title, and table names.",
        input_schema: schema_list_apps,
        scope: Scope::Workspace("access_workspace"),
        read_only: true,
        run: run_list_apps,
    },
    Tool {
        name: "describe_app",
        description: "Read the stored manifest of an app and its version.",
        input_schema: schema_describe_app,
        scope: Scope::App(AppAction::Read),
        read_only: true,
        run: run_describe_app,
    },
    Tool {
        name: "aggregate_records",
        description: "Aggregate records in a table using count, sum, avg, min, or max, optionally grouped by an indexed field.",
        input_schema: schema_aggregate_records,
        scope: Scope::App(AppAction::Read),
        read_only: true,
        run: run_aggregate_records,
    },
    Tool {
        name: "list_records",
        description: "List one page of a table's records, up to 200. Pass next_cursor back as cursor for the next page. A filter or sort works on a table of 10,000 rows or fewer.",
        input_schema: schema_list_records,
        scope: Scope::App(AppAction::Read),
        read_only: true,
        run: run_list_records,
    },
        Tool {
        name: "get_view_data",
        description: "Return grouped records for a kanban or group-by view with option labels and colors.",
        input_schema: schema_get_view_data,
        scope: Scope::App(AppAction::Read),
        read_only: true,
        run: run_get_view_data,
    },
Tool {
        name: "get_record",
        description: "Get one record, with its version.",
        input_schema: schema_get_record,
        scope: Scope::App(AppAction::Read),
        read_only: true,
        run: run_get_record,
    },
    Tool {
        name: "get_record_history",
        description: "List the changes made to a record, newest first, with who made each one. A field you may not read shows only that it changed.",
        input_schema: schema_get_record_history,
        scope: Scope::App(AppAction::Read),
        read_only: true,
        run: run_get_record_history,
    },
    Tool {
        name: "list_pages",
        description: "List the pages of an app: id, title, kind, and for a custom page its grants, format and libraries.",
        input_schema: schema_describe_app,
        scope: Scope::App(AppAction::Read),
        read_only: true,
        run: run_list_pages,
    },
    Tool {
        name: "get_page",
        description: "Get a custom page as one HTML document to show in a sandboxed frame. It holds no record data. The page reads and writes only through page_call.",
        input_schema: schema_get_page,
        scope: Scope::App(AppAction::Read),
        read_only: true,
        run: run_get_page,
    },
    Tool {
        name: "page_call",
        description: "Read or change data as a custom page. op is list, get, create, update, remove, settings_get, or settings_set. The page's grants bound it, and the caller's own rights still apply.",
        input_schema: schema_page_call,
        scope: Scope::App(AppAction::Read),
        read_only: false,
        run: run_page_call,
    },
    Tool {
        name: "update_record",
        description: "Change fields of a record. Send the version you read. Fields you do not name keep their value. A stale version is an error that names the current one.",
        input_schema: schema_update_record,
        scope: Scope::App(AppAction::WriteRecords),
        read_only: false,
        run: run_update_record,
    },
    Tool {
        name: "delete_record",
        description: "Move a record to trash. It can be restored. Send the version you read.",
        input_schema: schema_delete_record,
        scope: Scope::App(AppAction::WriteRecords),
        read_only: false,
        run: run_delete_record,
    },
    Tool {
        name: "propose_app_change",
        description: "Propose a new app or a change to one. Send the whole manifest and the source of each custom page. Nothing changes until a person approves it. Returns the proposal id, the checks, and the change list.",
        input_schema: schema_propose_app_change,
        scope: Scope::Workspace("access_workspace"),
        read_only: false,
        run: run_propose_app_change,
    },
    Tool {
        name: "list_proposals",
        description: "List the proposals of a workspace, pending first.",
        input_schema: schema_list_proposals,
        scope: Scope::Workspace("access_workspace"),
        read_only: true,
        run: run_list_proposals,
    },
    Tool {
        name: "get_proposal",
        description: "Get one proposal with its checks and its change list.",
        input_schema: schema_get_proposal,
        scope: Scope::Proposal("access_workspace"),
        read_only: true,
        run: run_get_proposal,
    },
    Tool {
        name: "decide_proposal",
        description: "Approve or reject a proposal. To approve, send the digest you reviewed. A workspace owner or admin only. A failed sensitivity, data loss, or page sensitivity check needs a second person.",
        input_schema: schema_decide_proposal,
        scope: Scope::Proposal("manage_workspace"),
        read_only: false,
        run: run_decide_proposal,
    },
    Tool {
        name: "preview_proposal",
        description: "Show a proposed change as an app before it is live. Returns a 30-minute read-only token, the app as proposed, and each custom page rendered. Live data is read through the stricter of the live and proposed rules.",
        input_schema: schema_get_proposal,
        scope: Scope::Proposal("access_workspace"),
        read_only: false,
        run: run_preview_proposal,
    },
    Tool {
        name: "get_agent_kit",
        description: "Get the setup text for an AI tool: an AGENTS.md section, a skill folder, and the server version. It writes nothing. Show the person the files and ask before you write them.",
        input_schema: schema_empty,
        scope: Scope::SignedIn,
        read_only: true,
        run: run_get_agent_kit,
    },
    Tool {
        name: "explain_access",
        description: "Show why you have the role you have on a workspace, app or table: the grants that reached it, and those that did not. Your own access only.",
        input_schema: schema_explain_access,
        scope: Scope::SignedIn,
        read_only: true,
        run: run_explain_access,
    },
    Tool {
        name: "list_waiting_decisions",
        description: "List the process steps waiting for you to decide, in apps you can read. Up to 200.",
        input_schema: schema_empty,
        scope: Scope::SignedIn,
        read_only: true,
        run: run_list_waiting_decisions,
    },
    Tool {
        name: "decide_process",
        description: "Approve or reject a waiting process step. Only the person the step names may decide. The submitter never may.",
        input_schema: schema_decide_process,
        scope: Scope::App(AppAction::Read),
        read_only: false,
        run: run_decide_process,
    },
    Tool {
        name: "list_views",
        description: "List the views of an app: your personal views and every collaborative and locked one.",
        input_schema: schema_list_views,
        scope: Scope::App(AppAction::Read),
        read_only: true,
        run: run_list_views,
    },
    Tool {
        name: "save_view",
        description: "Create a view, or change one by naming its view_id and its current version. A personal view is yours. A collaborative view needs an editor. A locked view needs an admin.",
        input_schema: schema_save_view,
        scope: Scope::App(AppAction::Read),
        read_only: false,
        run: run_save_view,
    },
    Tool {
        name: "delete_view",
        description: "Delete a view. The rights are those of saving it.",
        input_schema: schema_delete_view,
        scope: Scope::App(AppAction::Read),
        read_only: false,
        run: run_delete_view,
    },
    Tool {
        name: "get_job",
        description: "Get status, progress, result, error, and logs of a job (only creator or Platform Admin).",
        input_schema: schema_get_job,
        scope: Scope::SignedIn,
        read_only: true,
        run: run_get_job,
    },
    Tool {
        name: "begin_task",
        description: "Add one sentence, 200 characters at most, naming what you are doing for the person. Call it first.",
        input_schema: schema_begin_task,
        scope: Scope::SignedIn,
        read_only: false,
        run: run_begin_task,
    },
    Tool {
        name: "cancel_job",
        description: "Cancel a running or queued job (only creator or Platform Admin).",
        input_schema: schema_cancel_job,
        scope: Scope::SignedIn,
        read_only: false,
        run: run_cancel_job,
    },
    Tool {
        name: "export_records",
        description: "Enqueue an export job for records in an app table or view. Returns a job ID and download URL. Does not return record bytes directly.",
        input_schema: schema_export_records,
        scope: Scope::App(AppAction::Read),
        read_only: false,
        run: run_export_records,
    },
];

/// The one gate. Finds the tool, checks its scope, checks the arguments, and runs it.
pub fn call(caller: &AuthUser, origin: &Origin, name: &str, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
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
        Scope::Proposal(cedar_action) => {
            let id = required_str(&args, "proposal_id")?;
            let ws_id = crate::service::proposals::workspace_of(id, state)?;
            authorize_workspace(caller, &ws_id, cedar_action, state)?;
        }
        Scope::PlatformAdmin => {
            crate::service::admin::require_platform_admin(caller, state)?;
        }
    }

    validate_args(&(tool.input_schema)(), &args)?;
    (tool.run)(caller, origin, args, state)
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
            "data_classification": { "type": "string" },
            "icon": { "type": "string" }
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
            "role": { "type": "string", "description": "owner, admin, editor, commenter, or viewer" },
            "name": { "type": "string", "description": "Full display name" }
        },
        "required": ["workspace_id", "action", "eppn"]
    })
}

fn schema_create_record() -> Value {
    json!({
        "type": "object",
        "properties": {
            "app_slug": { "type": "string" },
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

fn run_list_workspaces(caller: &AuthUser, _origin: &Origin, _args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    to_value(&workspaces::list_workspaces(caller, state)?)
}

fn run_get_workspace(caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let ws_id = required_str(&args, "workspace_id")?;
    to_value(&workspaces::get_workspace(caller, ws_id, state)?)
}

fn run_update_workspace(caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let ws_id = required_str(&args, "workspace_id")?.to_string();
    let input = workspaces::UpdateWorkspacePayload {
        name: text(&args, "name"),
        description: text(&args, "description"),
        department: text(&args, "department"),
        visibility: text(&args, "visibility"),
        data_classification: text(&args, "data_classification"),
        icon: text(&args, "icon"),
        organization_id: text(&args, "organization_id").and_then(|s| uuid::Uuid::parse_str(&s).ok()),
        reason: text(&args, "reason"),
    };
    to_value(&workspaces::update_workspace(caller, &ws_id, input, state)?)
}

fn run_manage_workspace_member(caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let ws_id = required_str(&args, "workspace_id")?;
    let action = required_str(&args, "action")?;
    let eppn = required_str(&args, "eppn")?;
    let role = args.get("role").and_then(|v| v.as_str());
    let name = args.get("name").and_then(|v| v.as_str());
    workspaces::manage_workspace_member(caller, ws_id, action, eppn, role, name, state)
}

fn run_create_record(caller: &AuthUser, origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let app_slug = required_str(&args, "app_slug")?;
    let data = args.get("data").cloned().unwrap_or_else(|| json!({}));
    to_value(&records::create_record(caller, origin, app_slug, &data, state)?)
}

fn run_list_datasets(caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let datasets = state.datasets.read().unwrap_or_else(|p| p.into_inner());
    let dept_filter = args.get("department").and_then(|v| v.as_str());
    // Sample rows follow the catalog's rule, the same as the HTTP route. Where it withholds them, the key is absent.
    let list: Vec<Value> = datasets
        .values()
        .filter(|d| dept_filter.is_none_or(|f| d.department.eq_ignore_ascii_case(f)))
        .map(|d| {
            let mut v = serde_json::to_value(d).unwrap_or(Value::Null);
            if !crate::service::catalog::sample_visible_to(caller, &d.sensitivity_level) {
                if let Some(map) = v.as_object_mut() {
                    map.remove("sample_data");
                }
            }
            v
        })
        .collect();
    to_value(&list)
}

fn run_simulate_cedar_policy(_caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
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
        word @ ("faculty" | "student" | "staff" | "employee" | "affiliate" | "alum") => word,
        _ => "member",
    };
    let result = crate::service::access::simulate_record_action(state, eppn, affiliation, &department, action, "simulation", &department, ferpa_sensitive)?;

    Ok(json!({
        "decision": if result.allowed { "Allow" } else { "Deny" },
        "principal": eppn,
        "affiliation": affiliation_word,
        "action": action,
        "resource_ferpa_sensitive": ferpa_sensitive,
        "reasons": result.reasons,
        "diagnostics": result.diagnostics,
        "deciding_policy": result.policy,
    }))
}

fn run_record_governance_decision(caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    // The same Cedar rule the REST route applies. A student may not record a decision.
    crate::service::access::require(
        state,
        caller,
        "record_decision",
        &scaffoldry_policy::entities::Resource::System(crate::service::access::ledger_resource()),
        "Forbidden: Cedar policy denied ledger append",
    )?;

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

fn run_verify_decision_ledger(_caller: &AuthUser, _origin: &Origin, _args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    crate::service::governance::verify_decision_ledger(state)
}

fn run_export_oscal_compliance(_caller: &AuthUser, _origin: &Origin, _args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    Ok(state.export_oscal_component_definition())
}

fn run_get_framework_spec(_caller: &AuthUser, _origin: &Origin, _args: Value, _state: &SharedState) -> Result<Value, ServiceError> {
    Ok(crate::routes::framework::build_framework_spec_json())
}

fn schema_list_views() -> Value {
    json!({
        "type": "object",
        "properties": { "app_slug": { "type": "string" } },
        "required": ["app_slug"]
    })
}

fn schema_save_view() -> Value {
    json!({
        "type": "object",
        "properties": {
            "app_slug": { "type": "string" },
            "view_id": { "type": "string", "description": "The view to change. Leave out to create one" },
            "name": { "type": "string", "description": "1 to 120 characters" },
            "kind": { "type": "string", "enum": ["grid", "kanban", "calendar", "gallery", "form", "summary", "dashboard"], "description": "Required to create" },
            "ownership": { "type": "string", "enum": ["personal", "collaborative", "locked"], "description": "To create. Default collaborative" },
            "table_id": { "type": "string", "description": "The table the view is over, from describe_app" },
            "config": { "type": "object", "description": "The sort, filters, grouping and layout" },
            "version": { "type": "integer", "description": "The version you read. Required to change a view" }
        },
        "required": ["app_slug", "name"]
    })
}

fn schema_delete_view() -> Value {
    json!({
        "type": "object",
        "properties": {
            "app_slug": { "type": "string" },
            "view_id": { "type": "string", "description": "The view to delete" }
        },
        "required": ["app_slug", "view_id"]
    })
}

fn view_uuid(args: &Value) -> Result<uuid::Uuid, ServiceError> {
    uuid::Uuid::parse_str(required_str(args, "view_id")?).map_err(|_| ServiceError::NotFound("View not found".to_string()))
}

fn run_list_views(caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    crate::service::views::list(caller, required_str(&args, "app_slug")?, state)
}

fn run_save_view(caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let app = required_str(&args, "app_slug")?.to_string();
    if args.get("view_id").is_some() {
        let id = view_uuid(&args)?;
        let body: crate::service::views::UpdateBody = parse(json!({
            "name": args.get("name"),
            "config": args.get("config").cloned().unwrap_or_else(|| json!({})),
            "version": args.get("version"),
        }))?;
        return crate::service::views::update(caller, &app, id, body, state).map_err(stale_as_text);
    }
    let body: crate::service::views::CreateBody = parse(args)?;
    crate::service::views::create(caller, &app, body, state)
}

fn run_delete_view(caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    crate::service::views::delete(caller, required_str(&args, "app_slug")?, view_uuid(&args)?, state)
}

fn schema_get_job() -> Value {
    json!({
        "type": "object",
        "properties": {
            "job_id": { "type": "string", "description": "UUID of the job" }
        },
        "required": ["job_id"]
    })
}

fn schema_begin_task() -> Value {
    json!({
        "type": "object",
        "properties": { "task": { "type": "string", "maxLength": 200 } },
        "required": ["task"]
    })
}

/// Records the task on the AI tool's session. Every history entry, disclosure and receipt made in the session carries it.
fn run_begin_task(_caller: &AuthUser, origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let task = required_str(&args, "task")?.trim();
    if task.is_empty() || task.chars().count() > 200 {
        return Err(ServiceError::BadRequest("Argument 'task' must be one sentence of 1 to 200 characters".to_string()));
    }
    let session = origin.session.ok_or_else(|| ServiceError::BadRequest("begin_task is for an AI tool that signed in through OAuth".to_string()))?;
    let stored = state.repository.set_session_task(session, task).map_err(|e| ServiceError::Internal(e.to_string()))?;
    if !stored {
        return Err(ServiceError::unauthorized("This session has ended. Sign in again."));
    }
    Ok(json!({ "status": "recorded", "task": task }))
}

fn schema_cancel_job() -> Value {
    json!({
        "type": "object",
        "properties": {
            "job_id": { "type": "string", "description": "UUID of the job" }
        },
        "required": ["job_id"]
    })
}

fn run_get_job(caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let job_id_str = required_str(&args, "job_id")?;
    let uuid = uuid::Uuid::parse_str(job_id_str)
        .map_err(|_| ServiceError::NotFound("Job not found".to_string()))?;
    let repo = &*state.repository;
    let job = crate::jobs::get_job(repo, uuid)
        .map_err(|e| ServiceError::Internal(e.to_string()))?
        .ok_or_else(|| ServiceError::NotFound("Job not found".to_string()))?;
    if job.created_by.eq_ignore_ascii_case(&caller.eppn)
        || crate::service::admin::require_platform_admin(caller, state).is_ok()
    {
        serde_json::to_value(&job).map_err(|e| ServiceError::Internal(e.to_string()))
    } else {
        Err(ServiceError::NotFound("Job not found".to_string()))
    }
}

fn run_cancel_job(caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let job_id_str = required_str(&args, "job_id")?;
    let uuid = uuid::Uuid::parse_str(job_id_str)
        .map_err(|_| ServiceError::NotFound("Job not found".to_string()))?;
    let repo = &*state.repository;
    let job = crate::jobs::get_job(repo, uuid)
        .map_err(|e| ServiceError::Internal(e.to_string()))?
        .ok_or_else(|| ServiceError::NotFound("Job not found".to_string()))?;
    if job.created_by.eq_ignore_ascii_case(&caller.eppn)
        || crate::service::admin::require_platform_admin(caller, state).is_ok()
    {
        let updated = crate::jobs::cancel_job(repo, uuid)
            .map_err(|e| ServiceError::Internal(e.to_string()))?
            .ok_or_else(|| ServiceError::NotFound("Job not found".to_string()))?;
        serde_json::to_value(&updated).map_err(|e| ServiceError::Internal(e.to_string()))
    } else {
        Err(ServiceError::NotFound("Job not found".to_string()))
    }
}

// ---------------------------------------------------------------------------------------------
// Data tools
// ---------------------------------------------------------------------------------------------

/// No tool returns more than this many rows or this many bytes of rows.
const MAX_ROWS: usize = 200;
const MAX_BYTES: usize = 256 * 1024;

/// Reads the arguments into a struct. An unknown or ill-typed argument names itself.
fn parse<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, ServiceError> {
    serde_json::from_value(args).map_err(|e| ServiceError::BadRequest(e.to_string()))
}

fn stale_as_text(err: ServiceError) -> ServiceError {
    match err {
        ServiceError::StaleVersion { message, version } => {
            ServiceError::Conflict(format!("{message} The current version is {version}."))
        }
        other => other,
    }
}

fn schema_list_apps() -> Value {
    json!({
        "type": "object",
        "properties": { "workspace_id": { "type": "string", "description": "Unique workspace identifier" } },
        "required": ["workspace_id"]
    })
}

fn schema_describe_app() -> Value {
    json!({
        "type": "object",
        "properties": { "app_slug": { "type": "string" } },
        "required": ["app_slug"]
    })
}

fn schema_get_page() -> Value {
    json!({
        "type": "object",
        "properties": {
            "app_slug": { "type": "string" },
            "page_id": { "type": "string", "description": "Page id from list_pages. It must be a custom page" }
        },
        "required": ["app_slug", "page_id"]
    })
}

fn schema_page_call() -> Value {
    json!({
        "type": "object",
        "properties": {
            "app_slug": { "type": "string" },
            "page_id": { "type": "string", "description": "Page id from list_pages" },
            "op": { "type": "string", "description": "list, get, create, update, remove, settings_get, or settings_set" },
            "table_id": { "type": "string", "description": "The table. The page needs a grant for it" },
            "record_id": { "type": "string", "description": "For get, update, and remove" },
            "version": { "type": "integer", "description": "For update and remove: the version you read" },
            "data": { "type": "object", "description": "For create and update: only fields the page may write" },
            "filter": { "type": "object", "description": "For list. Same shape as list_records, on fields the page may read" },
            "sort": { "type": "object", "description": "For list: { field_name, direction: Asc|Desc }" },
            "limit": { "type": "integer", "description": "For list: rows per page, 1 to 200" },
            "cursor": { "type": "string", "description": "For list: the next_cursor of the previous page" },
            "key": { "type": "string", "description": "For settings_get and settings_set" },
            "value": { "description": "For settings_set: any JSON value. A person keeps 64 KB per page" }
        },
        "required": ["app_slug", "page_id", "op"]
    })
}

fn run_list_pages(_caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    crate::service::pages::list_pages(required_str(&args, "app_slug")?, state)
}

fn run_get_page(_caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    crate::service::pages::get_page(required_str(&args, "app_slug")?, required_str(&args, "page_id")?, state)
}

fn run_page_call(caller: &AuthUser, origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    crate::service::pages::page_call(caller, origin, args, state)
}

fn schema_list_records() -> Value {
    json!({
        "type": "object",
        "properties": {
            "app_slug": { "type": "string" },
            "table_id": { "type": "string", "description": "Table id or slug from describe_app" },
            "filter": { "type": "object", "description": "{ conjunction: And|Or, clauses: [{ id, field_name, operator, value }] }. Operators: Equals, NotEquals, Contains, NotContains, GreaterThan, LessThan, IsEmpty, IsNotEmpty" },
            "sort": { "type": "object", "description": "{ field_name, direction: Asc|Desc }" },
            "limit": { "type": "integer", "description": "Rows per page, 1 to 200. Default 50" },
            "cursor": { "type": "string", "description": "The next_cursor of the previous page" },
            "search": { "type": "string", "description": "Full-text search query across record fields" }
        },
        "required": ["app_slug", "table_id"]
    })
}

fn schema_get_record() -> Value {
    json!({
        "type": "object",
        "properties": {
            "app_slug": { "type": "string" },
            "record_id": { "type": "string", "description": "Record id" }
        },
        "required": ["app_slug", "record_id"]
    })
}

fn schema_update_record() -> Value {
    json!({
        "type": "object",
        "properties": {
            "app_slug": { "type": "string" },
            "record_id": { "type": "string", "description": "Record id" },
            "version": { "type": "integer", "description": "The version you read" },
            "data": { "type": "object", "description": "Fields to change. Set a field to null to clear it" }
        },
        "required": ["app_slug", "record_id", "version", "data"]
    })
}

fn schema_delete_record() -> Value {
    json!({
        "type": "object",
        "properties": {
            "app_slug": { "type": "string" },
            "record_id": { "type": "string", "description": "Record id" },
            "version": { "type": "integer", "description": "The version you read" }
        },
        "required": ["app_slug", "record_id", "version"]
    })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ListAppsArgs {
    workspace_id: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DescribeAppArgs {
    app_slug: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SortArg {
    field_name: String,
    direction: scaffoldry_engine::SortDirection,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ListRecordsArgs {
    app_slug: String,
    table_id: String,
    filter: Option<scaffoldry_engine::CompoundFilter>,
    sort: Option<SortArg>,
    limit: Option<usize>,
    cursor: Option<String>,
    search: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct GetRecordArgs {
    app_slug: String,
    record_id: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateRecordArgs {
    app_slug: String,
    record_id: String,
    version: i32,
    data: Map<String, Value>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteRecordArgs {
    app_slug: String,
    record_id: String,
    version: i32,
}

fn run_list_apps(_caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let args: ListAppsArgs = parse(args)?;
    let engine = state.engine.read().map_err(|e| ServiceError::Internal(e.to_string()))?;
    let mut apps: Vec<Value> = engine
        .list_manifests()
        .into_iter()
        .filter(|m| m.workspace_id.as_deref() == Some(args.workspace_id.as_str()))
        .map(|m| json!({ "slug": m.slug, "title": m.title, "tables": m.tables.iter().map(|t| t.name.clone()).collect::<Vec<_>>() }))
        .collect();
    apps.sort_by(|a, b| a["slug"].as_str().cmp(&b["slug"].as_str()));
    Ok(Value::Array(apps))
}

fn run_describe_app(caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let args: DescribeAppArgs = parse(args)?;
    let mut manifest = state
        .engine
        .read()
        .map_err(|e| ServiceError::Internal(e.to_string()))?
        .resolve_by_slug(&args.app_slug)
        .cloned()
        .ok_or_else(|| ServiceError::NotFound(format!("App '{}' not found", args.app_slug)))?;
    // An agent working for this person is not told about a field they cannot see.
    records::without_hidden_fields(caller, &args.app_slug, &mut manifest, state);
    Ok(crate::routes::apps::with_version(state, &manifest))
}

fn run_list_records(caller: &AuthUser, origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let args: ListRecordsArgs = parse(args)?;
    if let Some(f) = &args.filter {
        scaffoldry_engine::check_filter(f).map_err(ServiceError::BadRequest)?;
    }
    let (app_slug, table_id) = (args.app_slug.clone(), args.table_id.clone());
    let limit = args.limit.unwrap_or(50);
    if limit == 0 || limit > MAX_ROWS {
        return Err(ServiceError::BadRequest(format!("Argument 'limit' must be from 1 to {MAX_ROWS}")));
    }
    let page = records::list_page(
        caller,
        &args.app_slug,
        records::PageQuery {
            table_id: args.table_id,
            filter: args.filter,
            sort: args.sort.map(|s| records::SortSpec {
                field_name: s.field_name,
                descending: s.direction == scaffoldry_engine::SortDirection::Desc,
            }),
            limit,
            cursor: args.cursor,
            max_bytes: Some(MAX_BYTES),
            must_exist: true,
            search: args.search,
        },
        state,
    )?;
    crate::service::disclosure::record_agent_read(caller, origin, &app_slug, Some(&table_id), &page.records, state)?;
    let mut body = json!({ "records": page.records, "next_cursor": page.next_cursor });
    records::add_field_states(&mut body, &page.calculating);
    Ok(body)
}

fn run_get_record(caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let args: GetRecordArgs = parse(args)?;
    to_value(&records::get_record(caller, &args.app_slug, &args.record_id, state)?)
}

fn run_update_record(caller: &AuthUser, origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let args: UpdateRecordArgs = parse(args)?;
    // The agent names the fields to change. The rest of the row stays as it is.
    let current = records::get_record(caller, &args.app_slug, &args.record_id, state)?;
    let mut data = current.data.as_object().cloned().unwrap_or_default();
    for (name, value) in args.data {
        data.insert(name, value);
    }
    let payload = json!({ "data": data, "version": args.version });
    let saved = records::update_record(caller, origin, &args.app_slug, &args.record_id, &payload, state).map_err(stale_as_text)?;
    to_value(&saved)
}

fn run_delete_record(caller: &AuthUser, origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let args: DeleteRecordArgs = parse(args)?;
    records::delete_record_checked(caller, origin, &args.app_slug, &args.record_id, args.version, state).map_err(stale_as_text)?;
    Ok(json!({ "deleted": true }))
}

fn schema_get_record_history() -> Value {
    json!({
        "type": "object",
        "properties": {
            "app_slug": { "type": "string" },
            "record_id": { "type": "string", "description": "Record id" },
            "limit": { "type": "integer", "description": "Entries per page, 1 to 200. Default 50" },
            "cursor": { "type": "string", "description": "The next_cursor of the previous page" }
        },
        "required": ["app_slug", "record_id"]
    })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct GetRecordHistoryArgs {
    app_slug: String,
    record_id: String,
    limit: Option<usize>,
    cursor: Option<String>,
}

fn run_get_record_history(caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let args: GetRecordHistoryArgs = parse(args)?;
    let limit = args.limit.unwrap_or(crate::service::history::DEFAULT_PAGE);
    if limit == 0 || limit > MAX_ROWS {
        return Err(ServiceError::BadRequest(format!("Argument 'limit' must be from 1 to {MAX_ROWS}")));
    }
    let page = crate::service::history::record_history(
        caller,
        &args.app_slug,
        &args.record_id,
        crate::service::history::HistoryQuery { filter: Default::default(), cursor: args.cursor, limit },
        state,
    )?;
    Ok(json!({ "entries": page.entries, "next_cursor": page.next_cursor }))
}

fn schema_propose_app_change() -> Value {
    json!({
        "type": "object",
        "properties": {
            "workspace_id": { "type": "string", "description": "Unique workspace identifier" },
            "app_slug": { "type": "string", "description": "The app. Required when there is no manifest" },
            "manifest": { "type": "object", "description": "The whole app manifest. Read scaffoldry://schema/app-manifest" },
            "rules": { "type": "array", "items": { "type": "object" }, "description": "Automation rules to add or change. A rule is saved as a new version when approved. No webhook steps" },
            "summary": { "type": "string", "description": "What changes and why, in a sentence or two" },
            "page_sources": { "type": "array", "items": { "type": "string" }, "description": "The source of every new or changed custom page. Each is stored under its SHA-256, which the manifest names" }
        },
        "required": ["workspace_id", "summary"]
    })
}

fn schema_list_proposals() -> Value {
    json!({
        "type": "object",
        "properties": { "workspace_id": { "type": "string", "description": "Unique workspace identifier" } },
        "required": ["workspace_id"]
    })
}

fn schema_get_proposal() -> Value {
    json!({
        "type": "object",
        "properties": { "proposal_id": { "type": "string", "description": "UUID of the proposal" } },
        "required": ["proposal_id"]
    })
}

fn schema_decide_proposal() -> Value {
    json!({
        "type": "object",
        "properties": {
            "proposal_id": { "type": "string", "description": "UUID of the proposal" },
            "decision": { "type": "string", "description": "approve or reject" },
            "rationale": { "type": "string", "description": "Why. It is recorded in the ledger" },
            "digest": { "type": "string", "description": "To approve: the digest of the proposal you reviewed, from get_proposal. A different digest applies nothing" }
        },
        "required": ["proposal_id", "decision", "rationale"]
    })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposeArgs {
    workspace_id: String,
    app_slug: Option<String>,
    manifest: Option<Value>,
    #[serde(default)]
    rules: Vec<Value>,
    summary: String,
    #[serde(default)]
    page_sources: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposalIdArgs {
    proposal_id: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DecideArgs {
    proposal_id: String,
    decision: String,
    rationale: String,
    digest: Option<String>,
}

fn run_propose_app_change(caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let a: ProposeArgs = parse(args)?;
    crate::service::proposals::propose(
        caller,
        crate::service::proposals::ProposeInput {
            workspace_id: a.workspace_id,
            app_slug: a.app_slug,
            manifest: a.manifest,
            rules: a.rules,
            summary: a.summary,
            page_sources: a.page_sources,
        },
        state,
    )
}

fn run_list_proposals(_caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let a: ListAppsArgs = parse(args)?;
    crate::service::proposals::list(&a.workspace_id, state)
}

fn run_get_proposal(_caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let a: ProposalIdArgs = parse(args)?;
    crate::service::proposals::get(&a.proposal_id, state)
}

fn run_decide_proposal(caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let a: DecideArgs = parse(args)?;
    crate::service::proposals::decide(caller, &a.proposal_id, &a.decision, &a.rationale, a.digest.as_deref(), state)
}

fn run_preview_proposal(caller: &AuthUser, origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let a: ProposalIdArgs = parse(args)?;
    let mut opened = crate::service::preview::start(caller, &a.proposal_id, state)?;
    let shadow = opened["app_slug"].as_str().unwrap_or_default().to_string();
    opened["app"] = call(caller, origin, "describe_app", json!({ "app_slug": shadow }), state)?;
    let custom: Vec<String> = state
        .engine
        .read()
        .map_err(|e| ServiceError::Internal(e.to_string()))?
        .resolve_by_slug(&shadow)
        .map(|m| m.pages.iter().filter(|p| matches!(p.kind, scaffoldry_engine::PageKind::Custom { .. })).map(|p| p.id.clone()).collect())
        .unwrap_or_default();
    let mut pages = Vec::new();
    for id in custom {
        pages.push(json!({ "id": id, "html": crate::service::pages::get_page(&shadow, &id, state)?["html"] }));
    }
    opened["pages"] = Value::Array(pages);
    Ok(opened)
}

fn run_get_agent_kit(_caller: &AuthUser, _origin: &Origin, _args: Value, _state: &SharedState) -> Result<Value, ServiceError> {
    Ok(crate::service::agent_kit::kit())
}

fn schema_explain_access() -> Value {
    json!({
        "type": "object",
        "properties": {
            "object": { "type": "string", "description": "workspace:<id>, app:<slug> or table:<slug>/<table id>" },
            "person": { "type": "string", "description": "Your own address, or leave out" }
        },
        "required": ["object"]
    })
}

#[derive(serde::Deserialize)]
struct ExplainAccessArgs {
    object: String,
    person: Option<String>,
}

fn run_explain_access(caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let a: ExplainAccessArgs = parse(args)?;
    if a.person.as_deref().is_some_and(|p| !p.trim().eq_ignore_ascii_case(&caller.eppn)) {
        return Err(ServiceError::forbidden("explain_access explains your own access. Ask a Platform Admin or compliance to explain someone else's"));
    }
    crate::service::access_review::explain(state, caller, &caller.eppn, &a.object)
}

fn schema_decide_process() -> Value {
    json!({
        "type": "object",
        "properties": {
            "app_slug": { "type": "string" },
            "instance_id": { "type": "string", "description": "The id of the waiting process instance" },
            "decision": { "type": "string", "description": "approve or reject" }
        },
        "required": ["app_slug", "instance_id", "decision"]
    })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DecideProcessArgs {
    app_slug: String,
    instance_id: String,
    decision: String,
}

fn run_list_waiting_decisions(caller: &AuthUser, _origin: &Origin, _args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    Ok(Value::Array(crate::service::processes::list_waiting_decisions(caller, state)?))
}

fn run_decide_process(caller: &AuthUser, origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let a: DecideProcessArgs = parse(args)?;
    to_value(&crate::service::processes::decide_process(caller, origin, &a.app_slug, &a.instance_id, &a.decision, state)?)
}


fn schema_get_view_data() -> Value {
    json!({
        "type": "object",
        "properties": {
            "app_slug": { "type": "string" },
            "view_id": { "type": "string" },
        },
        "required": ["app_slug", "view_id"]
    })
}

#[derive(serde::Deserialize)]
struct GetViewDataArgs {
    app_slug: String,
    view_id: String,
}

fn run_get_view_data(caller: &AuthUser, origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let args: GetViewDataArgs = parse(args)?;
    let manifest = state
        .engine
        .read()
        .map_err(|e| ServiceError::Internal(e.to_string()))?
        .resolve_by_slug(&args.app_slug)
        .cloned()
        .or_else(|| state.get_app_manifest(&args.app_slug))
        .ok_or_else(|| ServiceError::NotFound(format!("App '{}' not found", args.app_slug)))?;

    let view = manifest
        .views
        .iter()
        .find(|v| v.id == args.view_id)
        .ok_or_else(|| ServiceError::NotFound(format!("View '{}' not found", args.view_id)))?;

    crate::service::views::check_runnable(caller, &args.app_slug, view, state)?;
    let all_records = records::list_records(caller, &args.app_slug, state)?;
    let raw_records: Vec<Value> = all_records.iter().map(|r| r.data.clone()).collect();

    let view_data = scaffoldry_engine::group_by_select(&manifest, view, &raw_records)
        .ok_or_else(|| ServiceError::BadRequest(format!("View '{}' is not over a select field", args.view_id)))?;

    // The stages hold the records themselves, so this is a release of records. Log the ones that came back.
    let returned: std::collections::HashSet<String> = view_data.stages.iter().flat_map(|s| s.records.iter().map(Value::to_string)).collect();
    let released: Vec<_> = all_records.iter().filter(|r| returned.contains(&r.data.to_string())).cloned().collect();
    crate::service::disclosure::record_agent_read(caller, origin, &args.app_slug, view.table_id.as_deref(), &released, state)?;

    to_value(&view_data)
}

fn schema_aggregate_records() -> Value {
    json!({
        "type": "object",
        "properties": {
            "app_slug": { "type": "string" },
            "table_id": { "type": "string", "description": "Table id or slug from describe_app" },
            "function": { "type": "string", "enum": ["count", "sum", "avg", "min", "max", "count_distinct", "filled", "empty", "unique"], "description": "Aggregate function. count_distinct (alias unique), filled and empty work on a field of any type" },
            "field": { "type": "string", "description": "Numeric field to aggregate (required unless function is count)" },
            "filter": { "type": "object", "description": "Optional filter on indexed fields" },
            "group_by": { "type": "string", "description": "Optional indexed field to group by" },
            "group_by2": { "type": "string", "description": "Optional second indexed field to group by, inside the first" },
            "bucket": { "type": "string", "enum": ["day", "week", "month", "quarter", "year"], "description": "Groups a date group_by into days, weeks (from Monday), months, quarters or years in the institution's time zone" }
        },
        "required": ["app_slug", "table_id", "function"]
    })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AggregateRecordsArgs {
    app_slug: String,
    table_id: String,
    function: String,
    field: Option<String>,
    filter: Option<scaffoldry_engine::CompoundFilter>,
    group_by: Option<String>,
    group_by2: Option<String>,
    bucket: Option<String>,
}

fn run_aggregate_records(caller: &AuthUser, _origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let args: AggregateRecordsArgs = parse(args)?;
    let func = match args.function.to_lowercase().as_str() {
        "count" => records::AggregateFunction::Count,
        "sum" => records::AggregateFunction::Sum,
        "avg" => records::AggregateFunction::Avg,
        "min" => records::AggregateFunction::Min,
        "max" => records::AggregateFunction::Max,
        "filled" => records::AggregateFunction::Filled,
        "empty" => records::AggregateFunction::Empty,
        "unique" | "count_distinct" => records::AggregateFunction::Unique,
        other => return Err(ServiceError::BadRequest(format!("Unknown aggregate function '{other}'"))),
    };
    let res = records::aggregate_records(
        caller,
        &args.app_slug,
        &args.table_id,
        records::AggregateQuery {
            function: func,
            field: args.field,
            filter: args.filter,
            group_by: args.group_by,
            group_by2: args.group_by2,
            bucket: args.bucket,
            order_by_key: false,
        },
        state,
    )?;
    serde_json::to_value(res).map_err(|e| ServiceError::Internal(e.to_string()))
}


fn schema_export_records() -> Value {
    json!({
        "type": "object",
        "properties": {
            "app_slug": { "type": "string" },
            "table_id": { "type": "string", "description": "Table id or slug to export" },
            "format": { "type": "string", "description": "Export format: csv, xlsx, json, or jsonl (default csv)" },
            "fields": { "type": "array", "items": { "type": "string" }, "description": "Field names to include" }
        },
        "required": ["app_slug", "table_id"]
    })
}

#[derive(serde::Deserialize)]
struct ExportRecordsArgs {
    app_slug: String,
    table_id: String,
    format: Option<String>,
    fields: Option<Vec<String>>,
}

fn run_export_records(
    caller: &AuthUser,
    _origin: &Origin,
    args: Value,
    state: &SharedState,
) -> Result<Value, ServiceError> {
    let args: ExportRecordsArgs = parse(args)?;
    let app_slug = args.app_slug;
    let table_id = args.table_id;
    let format_str = args.format.unwrap_or_else(|| "csv".to_string()).to_lowercase();
    if !["csv", "xlsx", "json", "jsonl"].contains(&format_str.as_str()) {
        return Err(ServiceError::BadRequest(format!("Export format '{format_str}' does not exist. Use csv, xlsx, json, or jsonl")));
    }

    let manifest = state
        .engine
        .read()
        .map_err(|e| ServiceError::Internal(e.to_string()))?
        .resolve_by_slug(&app_slug)
        .cloned()
        .ok_or_else(|| ServiceError::NotFound(format!("App '{app_slug}' not found")))?;

    let table = manifest
        .tables
        .iter()
        .find(|t| t.id == table_id || t.slug == table_id)
        .ok_or_else(|| ServiceError::NotFound(format!("Table '{table_id}' not found")))?;

    let field_vis = crate::service::scope::field_visibility_of(caller, &app_slug, &table.id, state);
    let disallowed_fields = field_vis.hidden_names();
    let sensitive_fields = crate::service::records::sensitive_fields_of(&app_slug, &table.id, state);

    let rules = crate::service::scope::rules_of(&app_slug, state);
    let person = crate::service::scope::person_of(caller, state);

    let export_id = uuid::Uuid::new_v4();

    let job_id = crate::jobs::enqueue(
        &state.repository,
        "export_run",
        json!({
            "export_id": export_id,
            "app_slug": app_slug,
            "table_id": table.id,
            "fields": args.fields,
            "format": format_str,
            "caller_eppn": caller.eppn,
            "caller_affiliation": caller.affiliation,
            "caller_person": person,
            "row_rules": rules,
            "disallowed_fields": disallowed_fields,
            "sensitive_fields": sensitive_fields,
        }),
        &caller.eppn,
        None,
    ).map_err(|e| ServiceError::Internal(e.to_string()))?;

    Ok(json!({
        "job_id": job_id.to_string(),
        "export_id": export_id.to_string(),
        "download_url": format!("/api/v1/apps/{}/exports/{}/download", app_slug, export_id),
    }))
}
