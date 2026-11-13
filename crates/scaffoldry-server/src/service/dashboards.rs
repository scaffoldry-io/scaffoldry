//! Views phase 5: dashboards and linked widgets.
//!
//! A dashboard is a view of kind `dashboard`: a 12-column grid of widgets. It is presentation and needs no proposal. A
//! widget shows a view, a chart over a summary view, one number, some text, or a custom page the manifest already approved.
//!
//! A widget may be linked to another: selecting a row in the source widget makes this widget's request add the filter
//! `target_field = <selected value>`. The server runs it, through the same row scope and field rules as any read. Nothing is
//! filtered in the browser, and a selection reaches no row the person could not already read.

use crate::repository_views::StoredView;
use crate::service::{records, views, ServiceError};
use crate::state::{AuthUser, SharedState};
use scaffoldry_engine::{PageKind, SUMMARY_FUNCTIONS};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

/// The grid's width. A widget lies inside it.
pub const COLUMNS: u64 = 12;
/// The most widgets a dashboard holds.
pub const MAX_WIDGETS: usize = 30;
/// The grid's depth, in rows.
pub const MAX_ROWS: u64 = 200;
const MAX_TEXT: usize = 20_000;

fn bad(m: impl Into<String>) -> ServiceError {
    ServiceError::BadRequest(m.into())
}

fn text_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// A widget's place on the grid: `(x, y, w, h)`.
type Spot = (u64, u64, u64, u64);

fn place(widget: &Value, id: &str) -> Result<Spot, ServiceError> {
    let n = |k: &str| widget.get(k).and_then(Value::as_u64).ok_or_else(|| bad(format!("The widget '{id}' needs a whole number for {k}")));
    let (x, y, w, h) = (n("x")?, n("y")?, n("w")?, n("h")?);
    if w == 0 || h == 0 {
        return Err(bad(format!("The widget '{id}' needs a width and a height of at least 1")));
    }
    if x + w > COLUMNS {
        return Err(bad(format!("The widget '{id}' runs past the right edge. The grid is {COLUMNS} columns wide")));
    }
    if y + h > MAX_ROWS {
        return Err(bad(format!("The widget '{id}' runs past the bottom. The grid is {MAX_ROWS} rows deep")));
    }
    Ok((x, y, w, h))
}

/// Checks a dashboard's config before it is saved. `shared` is true when anyone but the owner will open it: then it may
/// not lean on a personal view, which they cannot see.
pub fn validate(caller: &AuthUser, app_slug: &str, config: &Value, shared: bool, state: &SharedState) -> Result<(), ServiceError> {
    match config.get("columns") {
        None => {}
        Some(c) if c.as_u64() == Some(COLUMNS) => {}
        Some(_) => return Err(bad(format!("A dashboard has {COLUMNS} columns"))),
    }
    let widgets = config.get("widgets").and_then(Value::as_array).ok_or_else(|| bad("A dashboard needs a list of widgets"))?;
    if widgets.len() > MAX_WIDGETS {
        return Err(bad(format!("A dashboard holds {MAX_WIDGETS} widgets at most")));
    }
    let manifest = views::runtime_manifest(state, app_slug)?;
    let stored: Vec<StoredView> = state.repository.list_app_views(app_slug).map_err(|e| ServiceError::Internal(e.to_string()))?;

    let mut kinds: HashMap<&str, &str> = HashMap::new();
    let mut spots: Vec<(&str, Spot)> = Vec::new();
    for w in widgets {
        let id = text_of(w, "id").filter(|i| i.chars().count() <= 64).ok_or_else(|| bad("Every widget needs an id of 1 to 64 characters"))?;
        if kinds.contains_key(id) {
            return Err(bad(format!("Two widgets are called '{id}'")));
        }
        text_of(w, "title").filter(|t| t.chars().count() <= 120).ok_or_else(|| bad(format!("The widget '{id}' needs a title of 1 to 120 characters")))?;
        let spot = place(w, id)?;
        for (other, o) in &spots {
            let apart = spot.0 + spot.2 <= o.0 || o.0 + o.2 <= spot.0 || spot.1 + spot.3 <= o.1 || o.1 + o.3 <= spot.1;
            if !apart {
                return Err(bad(format!("The widgets '{other}' and '{id}' overlap")));
            }
        }
        spots.push((id, spot));

        let body = w.get("body").ok_or_else(|| bad(format!("The widget '{id}' needs a body")))?;
        let kind = body.get("type").and_then(Value::as_str).unwrap_or("");
        kinds.insert(id, kind);
        match kind {
            "view" | "chart" => {
                let vid = text_of(body, "view_id").ok_or_else(|| bad(format!("The widget '{id}' needs a view_id")))?;
                let view = stored
                    .iter()
                    .find(|v| v.id.to_string() == vid && (v.ownership != "personal" || v.owner.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(&caller.eppn))))
                    .ok_or_else(|| bad(format!("The widget '{id}' names a view this app does not have: {vid}")))?;
                if shared && view.ownership == "personal" {
                    return Err(bad(format!("The widget '{id}' shows a personal view, and a shared dashboard cannot")));
                }
                if view.kind == "dashboard" {
                    return Err(bad(format!("The widget '{id}' shows a dashboard. A dashboard holds no dashboard")));
                }
                if kind == "chart" && view.kind != "summary" {
                    return Err(bad(format!("The widget '{id}' is a chart, and a chart draws a summary view")));
                }
            }
            "stat" => {
                let table = text_of(body, "table_id").ok_or_else(|| bad(format!("The widget '{id}' needs a table_id")))?;
                let t = manifest.tables.iter().find(|t| t.id == table).ok_or_else(|| bad(format!("The widget '{id}' names a table this app does not have: {table}")))?;
                let function = text_of(body, "fn").unwrap_or("");
                if !SUMMARY_FUNCTIONS.contains(&function) {
                    return Err(bad(format!("The widget '{id}' uses '{function}', which is not an aggregation")));
                }
                if function != "count" {
                    let field = text_of(body, "field").ok_or_else(|| bad(format!("The widget '{id}' needs a field for {function}")))?;
                    if !t.fields.iter().any(|f| f.name == field) {
                        return Err(bad(format!("The widget '{id}' names a field the table does not have: {field}")));
                    }
                }
            }
            "text" => {
                let md = body.get("markdown").and_then(Value::as_str).ok_or_else(|| bad(format!("The widget '{id}' needs markdown")))?;
                if md.chars().count() > MAX_TEXT {
                    return Err(bad(format!("The widget '{id}' holds {MAX_TEXT} characters of text at most")));
                }
            }
            "page" => {
                let page = text_of(body, "page_id").ok_or_else(|| bad(format!("The widget '{id}' needs a page_id")))?;
                let approved = manifest.pages.iter().any(|p| p.id == page && matches!(p.kind, PageKind::Custom { .. }));
                if !approved {
                    return Err(bad(format!("The widget '{id}' names the page '{page}', and this app has no approved custom page by that name")));
                }
            }
            other => return Err(bad(format!("The widget '{id}' has the body type '{other}'. Use view, chart, stat, text or page"))),
        }
    }

    for w in widgets {
        let Some(link) = w.get("select_by") else { continue };
        let id = text_of(w, "id").unwrap_or("");
        let source = text_of(link, "source_widget").ok_or_else(|| bad(format!("The link of '{id}' needs a source_widget")))?;
        if source == id {
            return Err(bad(format!("The widget '{id}' cannot be linked to itself")));
        }
        match kinds.get(source) {
            None => return Err(bad(format!("The link of '{id}' names the widget '{source}', which this dashboard does not have"))),
            Some(&"view") => {}
            Some(_) => return Err(bad(format!("The link of '{id}' names '{source}', and only a view widget has rows to select"))),
        }
        text_of(link, "source_field").ok_or_else(|| bad(format!("The link of '{id}' needs a source_field")))?;
        text_of(link, "target_field").ok_or_else(|| bad(format!("The link of '{id}' needs a target_field")))?;
    }
    Ok(())
}

/// What a widget's request asks for.
#[derive(Debug, Default, Deserialize)]
pub struct WidgetQuery {
    /// The selected row's value of the source field. Needs a widget that is linked.
    #[serde(default)]
    pub select: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// `GET /apps/{slug}/views/{id}/widgets/{widget_id}`: one widget's data, run on the server.
pub fn run_widget(
    caller: &AuthUser,
    origin: &crate::service::history::Origin,
    app_slug: &str,
    dashboard: Uuid,
    widget_id: &str,
    query: WidgetQuery,
    state: &SharedState,
) -> Result<Value, ServiceError> {
    let dash = views::existing(caller, app_slug, dashboard, state)?;
    if dash.kind != "dashboard" {
        return Err(ServiceError::NotFound("Dashboard not found".to_string()));
    }
    let widget = dash
        .config
        .get("widgets")
        .and_then(Value::as_array)
        .and_then(|w| w.iter().find(|w| w.get("id").and_then(Value::as_str) == Some(widget_id)))
        .ok_or_else(|| ServiceError::NotFound(format!("The dashboard has no widget '{widget_id}'")))?;
    let body = &widget["body"];
    let kind = body.get("type").and_then(Value::as_str).unwrap_or("");

    // The link turns a selected value into one more filter on this widget's own request.
    let also = match (widget.get("select_by"), query.select) {
        (Some(link), Some(value)) => Some((text_of(link, "target_field").unwrap_or("").to_string(), value)),
        (None, Some(_)) => return Err(bad(format!("The widget '{widget_id}' is not linked to another, so a selection means nothing to it"))),
        (_, None) => None,
    };

    let data = match kind {
        "view" | "chart" => {
            let vid = text_of(body, "view_id").and_then(|v| Uuid::parse_str(v).ok()).ok_or_else(|| bad("The widget's view_id is not a view"))?;
            views::run(caller, origin, app_slug, vid, views::RunQuery { cursor: query.cursor, limit: query.limit, also: also.clone(), ..Default::default() }, state)?
        }
        "stat" => {
            use records::AggregateFunction as F;
            let function = match text_of(body, "fn").unwrap_or("") {
                "count" => F::Count,
                "sum" => F::Sum,
                "avg" => F::Avg,
                "min" => F::Min,
                "max" => F::Max,
                "count_distinct" => F::Unique,
                other => return Err(bad(format!("'{other}' is not an aggregation"))),
            };
            let table = text_of(body, "table_id").unwrap_or("");
            let filter = also.as_ref().map(|(f, v)| views::and_clause(None, f, v));
            let result = records::aggregate_records(
                caller,
                app_slug,
                table,
                records::AggregateQuery { function, field: text_of(body, "field").map(str::to_string), filter, ..Default::default() },
                state,
            )?;
            json!({ "stat": { "fn": text_of(body, "fn"), "field": text_of(body, "field"), "value": result.value.unwrap_or(0.0) } })
        }
        _ => json!({}),
    };
    Ok(json!({ "widget_id": widget_id, "kind": kind, "selected": also.map(|(field, value)| json!({ "field": field, "value": value })), "data": data }))
}
