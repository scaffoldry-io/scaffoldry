//! Screens that render inside an AI tool (mcp-apps phase 8), by the MCP Apps extension, specification 2026-01-26.
//!
//! A screen is a `ui://` resource with the MIME type `text/html;profile=mcp-app`. Two built-in screens are static files that
//! hold no data: the host gives them the tool result and they place it as text. A custom page is the document `get_page`
//! returns, under `ui://scaffoldry/apps/{slug}/pages/{id}`. A tool links to its screen with `_meta.ui.resourceUri` on its
//! definition.

use crate::service::access::{authorize_app, AppAction};
use crate::state::{AuthUser, SharedState};
use scaffoldry_engine::PageKind;
use serde_json::{json, Value};

pub const MIME: &str = "text/html;profile=mcp-app";
pub const REVIEW_URI: &str = "ui://scaffoldry/proposal-review";
pub const DECISION_URI: &str = "ui://scaffoldry/decision";

const REVIEW_HTML: &str = include_str!("../ui/proposal-review.html");
const DECISION_HTML: &str = include_str!("../ui/decision.html");

/// A page cannot be listed forever: more than this many custom pages are left off the list. Each is still readable.
const MAX_LISTED_PAGES: usize = 500;

pub fn page_uri(slug: &str, page_id: &str) -> String {
    format!("ui://scaffoldry/apps/{slug}/pages/{page_id}")
}

/// The app and page named by `ui://scaffoldry/apps/{slug}/pages/{id}`.
fn parse_page_uri(uri: &str) -> Option<(&str, &str)> {
    let rest = uri.strip_prefix("ui://scaffoldry/apps/")?;
    let (slug, page_id) = rest.split_once("/pages/")?;
    (!slug.is_empty() && !page_id.is_empty() && !slug.contains('/') && !page_id.contains('/')).then_some((slug, page_id))
}

/// The screen a tool links to, for `_meta.ui.resourceUri` on its definition.
pub fn tool_screen(tool: &str) -> Option<&'static str> {
    match tool {
        "get_proposal" => Some(REVIEW_URI),
        "list_waiting_decisions" => Some(DECISION_URI),
        _ => None,
    }
}

/// The `resources/list` entries for screens: the two built-in ones, and one per custom page the caller can read.
pub fn listed(caller: &AuthUser, state: &SharedState) -> Vec<Value> {
    let mut out = vec![
        json!({ "uri": REVIEW_URI, "name": "Review a proposed change", "description": "Shows a proposal, its changes and checks, with Approve and Reject", "mimeType": MIME }),
        json!({ "uri": DECISION_URI, "name": "Decisions waiting for you", "description": "Shows each waiting process step with its record, with Approve and Reject", "mimeType": MIME }),
    ];
    let mut pages: Vec<(String, String, String)> = {
        let engine = state.engine.read().unwrap_or_else(|p| p.into_inner());
        engine
            .list_manifests()
            .into_iter()
            .filter(|m| !crate::service::preview::is_preview(&m.slug))
            .flat_map(|m| {
                m.pages
                    .iter()
                    .filter(|p| matches!(p.kind, PageKind::Custom { .. }))
                    .map(|p| (m.slug.clone(), p.id.clone(), p.title.clone()))
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    pages.sort();
    let mut allowed: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    for (slug, page_id, title) in pages {
        if out.len() >= MAX_LISTED_PAGES + 2 {
            break;
        }
        let ok = *allowed.entry(slug.clone()).or_insert_with(|| authorize_app(caller, &slug, AppAction::Read, state).is_ok());
        if ok {
            out.push(json!({ "uri": page_uri(&slug, &page_id), "name": title, "description": format!("A page of the app {slug}"), "mimeType": MIME }));
        }
    }
    out
}

/// The document at a screen URI. None when the URI is not one of ours. `Some(None)` is a screen the caller may not have,
/// or one that does not exist: the two look the same, so a person cannot find out what they may not see.
pub fn read(caller: &AuthUser, uri: &str, state: &SharedState) -> Option<Option<String>> {
    match uri {
        REVIEW_URI => return Some(Some(REVIEW_HTML.to_string())),
        DECISION_URI => return Some(Some(DECISION_HTML.to_string())),
        _ => {}
    }
    let (slug, page_id) = parse_page_uri(uri)?;
    if crate::service::preview::is_preview(slug) || authorize_app(caller, slug, AppAction::Read, state).is_err() {
        return Some(None);
    }
    Some(crate::service::pages::render_page(slug, page_id, state).ok())
}
