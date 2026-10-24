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
                "Call list_waiting_decisions to see the steps waiting for you, then retry."
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
                "Call list_workspaces to see where you have access, and ask a workspace owner for a role if you need more."
            }
        }
        "version_conflict" => "Call get_record to read the current version, then call the tool again with it.",
        "constraint_violated" => "Call describe_app to see which fields are required and their limits, then send them.",
        "field_hidden" | "field_read_only" => "Call describe_app to see which fields you may read and change, then retry without the others.",
        "index_building" | "calculating" => "Call get_job to watch progress, then retry when it is done.",
        "storage_unavailable" => "Retry in a minute, and call list_workspaces to check that the server is answering.",
        "quota_exceeded" => "Wait for your queued work to finish, and call get_job to check on it.",
        "too_large" => "Send less in one call, and call list_records with a limit to read a page at a time.",
        "file_too_large" | "file_type_blocked" | "scanner_unavailable" => "Choose a smaller or plainer file, and call get_job if an upload was queued.",
        "undo_conflict" => "Call get_record_history to see what changed since, then decide what to keep.",
        "position_full" => "Call list_workspaces to find the unit, then ask a Platform Admin to replace the holder.",
        "view_as_read_only" => "Exit the view to make changes. Call list_records to keep reading in the meantime.",
        "client_sensitive_field" => "Remove the sensitive field from the client's ceiling. Sensitive data stays inside Scaffoldry's own pages.",
        "client_scope" => "The app's ceiling does not cover this. Ask its owners to widen the ceiling, or do it in Scaffoldry yourself.",
        "token_scope" => "This token is limited. Ask the person who made it for a token that covers this app, table and operation.",
        "preview_read_only" => "Stop the preview to make a change. A preview only reads. Approve the proposal to make it live.",
        "proposal_changed" => "Call get_proposal, read what it now holds, and approve with the digest it returns.",
        "grant_outside_scope" | "grant_too_broad" => "Ask for access inside your own units, and call list_workspaces to see what you administer.",
        "unauthorized" => "Sign in again, then call list_workspaces to confirm access.",
        _ => "Retry once, and call list_workspaces to check that the server is answering.",
    };
    h.to_string()
}
