//! One sentence for an agent that got a tool error: what to do next, naming a tool that exists. A hint is chosen from the
//! error's code and the shape of its message. It repeats nothing from the message that came from the caller, so it never
//! carries a record value.

use crate::service::ServiceError;

pub fn hint_for(err: &ServiceError) -> String {
    let msg = err.message();
    let has = |s: &str| msg.contains(s);
    let h = match err.code() {
        "not_found" => {
            if has("Tool '") {
                "Use one of the listed tools, and start with list_workspaces to see where you can work."
            } else if has("Workspace") {
                "Call list_workspaces to see the workspace ids you can use, then retry."
            } else if has("App ") || has("App not found") {
                "Call list_apps with a workspace_id to see the app slugs, then retry."
            } else if has("Table") {
                "Call describe_app to see the table ids, then retry."
            } else if has("Record") {
                "Call list_records to find the record id, then retry."
            } else if has("Proposal") {
                "Call list_proposals with a workspace_id to find the proposal id, then retry."
            } else if has("Job") {
                "Use the job_id that the tool returned, then call get_job again."
            } else if has("View") {
                "Call list_views to see the view ids, then retry."
            } else if has("Process") {
                "Call list_my_work to see the steps waiting for you, then retry."
            } else {
                "Call list_workspaces to find the right id, then retry."
            }
        }
        "bad_request" => {
            if has("Unknown argument") {
                "Use only the argument names the message lists, or call describe_app if the name is a field."
            } else if has("is required") {
                "Add the missing argument and call the tool again, and call describe_app if it is a field name."
            } else if has("must be of type") {
                "Send the argument in the type the message names, and call describe_app to see field types."
            } else if has("option id") {
                "Call describe_app to see the select options, then send the option id and not its label."
            } else if has("manifest") || has("schema:") {
                "Read scaffoldry://schema/app-manifest, fix what the message lists, then call propose_app_change again."
            } else if has("needs a manifest") {
                "Send a manifest or rules, or name an app_slug, and call propose_app_change again."
            } else if has("indexed set") {
                "Call describe_app and use a field marked indexed, then retry."
            } else if has("member action") {
                "Use add, update, or remove, then call manage_workspace_member again."
            } else if has("decision") {
                "Send the decision as approve or reject, then call decide_proposal or decide_process again."
            } else if has("unknown variant") || has("missing field") {
                "Check the argument against the tool's schema, and call describe_app for the names it needs."
            } else {
                "Call describe_app to check the names and types you sent, then retry."
            }
        }
        "forbidden" => {
            if has("Platform Admin") {
                "This tool is for a Platform Admin. Call list_workspaces to see what you can do instead."
            } else {
                "Call request_access with the request this result carries, or call list_workspaces to see where you have access."
            }
        }
        other => hint_for_code(other).unwrap_or("Retry once, and call list_workspaces to check that the server is answering."),
    };
    h.to_string()
}

/// The hint for a code when nothing in the message sharpens it. Every code in `ERROR_CODES` has one, and a test holds
/// that. Each is fixed text: it carries nothing from the error, so it carries no record value.
pub fn hint_for_code(code: &str) -> Option<&'static str> {
    Some(match code {
        "no_approver" => "Call list_workspaces to find the unit, then ask a Platform Admin to fill the approver position.",
        "job_failed" => "Call get_job to read why it stopped, fix that, then start the work again.",
        "sensitive_content_blocked" => "Remove the protected data from what you send. Sensitive data stays inside Scaffoldry's own pages, so ask the person to enter it there.",
        "rate_limited" => "Wait a minute, then retry. Send fewer calls, and call list_records with a limit to read a page at a time.",
        "master_key_missing" => "This server cannot keep secrets yet. Ask a Platform Admin to set the master key, then retry.",
        "share_not_allowed" => "Sharing is off here. Ask a workspace owner or a Platform Admin to allow it, or call request_access for the access you need.",
        "archived" => "The app or workspace only reads. Call list_records to keep reading, and ask its owner to unarchive it to change it.",
        "too_many_indexes" => "Remove an index that the message lists as unused, then call propose_app_change again.",
        "internal" => "Retry once. If it fails again, call list_workspaces to check that the server is answering, and tell the person.",
        "forbidden" => "Call list_workspaces to see where you have access, then call request_access for the role you need.",
        "not_found" => "Call list_workspaces to find the right id, then retry.",
        "bad_request" => "Call describe_app to check the names and types you sent, then retry.",
        "read_only_source" => "This is a copy of another system's data. Change it at its source, and the next sync brings the change here.",
        "version_conflict" => "Call get_record to read the current version, then call the tool again with it.",
        "constraint_violated" => "Call describe_app to see which fields are required and their limits, then send them.",
        "field_hidden" | "field_read_only" => "Call describe_app to see which fields you may read and change, then retry without the others.",
        "index_building" | "calculating" => "Call get_job to watch progress, then retry when it is done.",
        "storage_unavailable" => "Retry in a minute, and call list_workspaces to check that the server is answering.",
        "quota_exceeded" => "Wait for your queued work to finish, and call get_job to check on it.",
        "too_large" => "Send less in one call, and call list_records with a limit to read a page at a time.",
        "file_too_large" | "file_type_blocked" | "scanner_unavailable" => "Choose a smaller or plainer file, and call get_job if an upload was queued.",
        "undo_conflict" => "Call get_record with history: true to see what changed since, then decide what to keep.",
        "position_full" => "Call list_workspaces to find the unit, then ask a Platform Admin to replace the holder.",
        "impersonation_not_synthetic" => "Open the app and use view-as to see what that person sees. Only synthetic people can be acted as.",
        "authority_loosens" => "Ask for the same or more: a flagged check is at least Moderate, and a Moderate or High change needs at least a second person.",
        "citation_not_found" => "Copy the passage exactly from the snapshot, and read it again with read_library_snapshot if it has changed.",
        "grant_conflict" => "One person may not hold both. Give it to someone else, or ask a Platform Admin to change the setting access.conflicts.",
        "view_as_read_only" => "Exit the view to make changes. Call list_records to keep reading in the meantime.",
        "client_sensitive_field" => "Remove the sensitive field from the client's ceiling. Sensitive data stays inside Scaffoldry's own pages.",
        "client_scope" => "The app's ceiling does not cover this. Ask its owners to widen the ceiling, or do it in Scaffoldry yourself.",
        "flow_grant_exceeded" => "The flow touched something its approved grants do not cover. Call propose_app_change with a new version of the flow, and its grants are derived again for review.",
        "last_owner" => "Make someone else an owner first with manage_workspace_member, then change this one. Call list_workspaces to see who owns it.",
        "token_scope" => "This token is limited. Ask the person who made it for a token that covers this app, table and operation.",
        "preview_read_only" => "Stop the preview to make a change. A preview only reads. Approve the proposal to make it live.",
        "proposal_changed" => "Call get_proposal, read what it now holds, and approve with the digest it returns.",
        "separation_of_duties" => "Choose someone who did not make the record or start the process.",
        "grant_outside_scope" | "grant_too_broad" => "Ask for access inside your own units, and call list_workspaces to see what you administer.",
        "unauthorized" => "Sign in again, then call list_workspaces to confirm access.",
        _ => return None,
    })
}
