//! Sovereign Governance and MCP App UI Resource Rendering Service
//! Implements Model Context Protocol Apps extension (SEP-1865) rendering
//! sovereign HTML components under `ui://` URIs with `text/html;profile=mcp-app`.

use crate::service::workspaces::get_workspace;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use serde_json::json;

pub fn verify_decision_ledger(state: &SharedState) -> Result<serde_json::Value, ServiceError> {
    let is_valid = state
        .verify_ledger()
        .map_err(|e| ServiceError::Internal(e.to_string()))?;
    let ledger = state
        .ledger
        .read()
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    Ok(json!({
        "verified": is_valid,
        "total_blocks": ledger.len(),
        "head_hash": ledger.last().map(|e| e.entry_hash.as_str()).unwrap_or(""),
        "integrity_audit": if is_valid { "ALL_BLOCKS_VALID" } else { "TAMPER_DETECTED" }
    }))
}

pub fn render_ui_workspace_settings(
    caller: &AuthUser,
    workspace_id: &str,
    state: &SharedState,
) -> Result<String, ServiceError> {
    let ws_res = get_workspace(caller, workspace_id, state)?;
    let ws = &ws_res.workspace;

    let collab_rows: Vec<String> = ws_res
        .collaborators
        .iter()
        .map(|c| {
            format!(
                r#"<tr>
                    <td style="padding: 8px; border-bottom: 1px solid #334155;">{}</td>
                    <td style="padding: 8px; border-bottom: 1px solid #334155; font-family: monospace;">{}</td>
                    <td style="padding: 8px; border-bottom: 1px solid #334155;"><span style="background: #1e293b; color: #38bdf8; padding: 2px 6px; border-radius: 4px;">{}</span></td>
                </tr>"#,
                html_escape(&c.name),
                html_escape(&c.eppn),
                html_escape(&c.role)
            )
        })
        .collect();

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8"/>
  <meta name="viewport" content="width=device-width, initial-scale=1.0"/>
  <title>Workspace Security Settings - {}</title>
  <style>
    body {{
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
      background: #0f172a;
      color: #f8fafc;
      margin: 0;
      padding: 24px;
    }}
    .card {{
      background: #1e293b;
      border: 1px solid #334155;
      border-radius: 8px;
      padding: 20px;
      margin-bottom: 20px;
    }}
    h1, h2 {{ margin-top: 0; color: #38bdf8; }}
    table {{ width: 100%; border-collapse: collapse; text-align: left; }}
    th {{ background: #0f172a; padding: 10px 8px; color: #94a3b8; font-size: 13px; text-transform: uppercase; }}
    .badge {{
      display: inline-block;
      padding: 4px 8px;
      font-size: 12px;
      font-weight: 600;
      border-radius: 4px;
      background: #0284c7;
      color: white;
    }}
  </style>
</head>
<body>
  <div class="card">
    <h1>Workspace Security &amp; Access Controls</h1>
    <p><strong>Workspace:</strong> {} ({})</p>
    <p><strong>Department:</strong> {} | <strong>Visibility:</strong> <span class="badge">{}</span> | <strong>Classification:</strong> {}</p>
    <p><strong>Governance Lattice:</strong> NIST OSCAL AC-02, AC-03, SC-07 | Sovereign Cedar ABAC Enforced</p>
  </div>

  <div class="card">
    <h2>Authorized Collaborators</h2>
    <table>
      <thead>
        <tr>
          <th>Name</th>
          <th>Identity (EPPN)</th>
          <th>Assigned Role</th>
        </tr>
      </thead>
      <tbody>
        {}
      </tbody>
    </table>
  </div>
</body>
</html>"#,
        html_escape(&ws.name),
        html_escape(&ws.name),
        html_escape(&ws.id),
        html_escape(&ws.department),
        html_escape(&ws.visibility),
        html_escape(&ws.data_classification),
        collab_rows.join("\n")
    );

    Ok(html)
}

pub fn render_ui_decision_ledger(
    _caller: &AuthUser,
    state: &SharedState,
) -> Result<String, ServiceError> {
    let ledger = state
        .ledger
        .read()
        .map_err(|e| ServiceError::Internal(e.to_string()))?;
    let is_valid = state
        .verify_ledger()
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    let rows: Vec<String> = ledger
        .iter()
        .map(|entry| {
            format!(
                r#"<tr>
                    <td style="padding: 8px; border-bottom: 1px solid #334155; font-family: monospace;">#{}</td>
                    <td style="padding: 8px; border-bottom: 1px solid #334155; font-family: monospace;">{}</td>
                    <td style="padding: 8px; border-bottom: 1px solid #334155;"><span style="background: #0369a1; color: white; padding: 2px 6px; border-radius: 4px;">{}</span></td>
                    <td style="padding: 8px; border-bottom: 1px solid #334155;">{}</td>
                    <td style="padding: 8px; border-bottom: 1px solid #334155; font-family: monospace; font-size: 11px;">{}</td>
                </tr>"#,
                entry.sequence,
                html_escape(&entry.principal),
                html_escape(&entry.oscal_control_id),
                html_escape(&entry.rationale),
                &entry.entry_hash[..16]
            )
        })
        .collect();

    let status_badge = if is_valid {
        r#"<span style="background: #15803d; color: white; padding: 4px 8px; border-radius: 4px; font-weight: 600;">ALL BLOCKS VALID (SHA-256)</span>"#
    } else {
        r#"<span style="background: #b91c1c; color: white; padding: 4px 8px; border-radius: 4px; font-weight: 600;">TAMPER DETECTED</span>"#
    };

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8"/>
  <meta name="viewport" content="width=device-width, initial-scale=1.0"/>
  <title>Governance Decision Ledger</title>
  <style>
    body {{
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
      background: #0f172a;
      color: #f8fafc;
      margin: 0;
      padding: 24px;
    }}
    .card {{
      background: #1e293b;
      border: 1px solid #334155;
      border-radius: 8px;
      padding: 20px;
      margin-bottom: 20px;
    }}
    h1, h2 {{ margin-top: 0; color: #38bdf8; }}
    table {{ width: 100%; border-collapse: collapse; text-align: left; }}
    th {{ background: #0f172a; padding: 10px 8px; color: #94a3b8; font-size: 13px; text-transform: uppercase; }}
  </style>
</head>
<body>
  <div class="card">
    <h1>Governance Decision Ledger</h1>
    <p>Cryptographic Immutable Audit Chain: {}</p>
    <p><strong>Total Recorded Blocks:</strong> {} | <strong>Standard:</strong> NIST OSCAL 1.1.2 AU-02 Cryptographic Audit</p>
  </div>

  <div class="card">
    <h2>Recorded Decision Blocks</h2>
    <table>
      <thead>
        <tr>
          <th>Seq</th>
          <th>Principal</th>
          <th>Control</th>
          <th>Rationale</th>
          <th>Block Hash</th>
        </tr>
      </thead>
      <tbody>
        {}
      </tbody>
    </table>
  </div>
</body>
</html>"#,
        status_badge,
        ledger.len(),
        rows.join("\n")
    );

    Ok(html)
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
