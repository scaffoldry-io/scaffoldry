//! Flows (workflows phase 1): what a flow may touch, said as grants and as a sentence, and what makes a flow invalid.
//!
//! A flow names records by address: the trigger record, the records it links to, and records that earlier steps of the
//! same run created or found. Its expressions name `{trigger.field}`, `{step_id.field}`, `{run.id}` and
//! `{run.started_at}`. Grants are derived from the steps, never written by hand.

use crate::{AppManifest, AppTable, FieldType};
use scaffoldry_core::workflow::{ActionType, AutomationRule, Expr, FlowGrant, StepKind, Target, TriggerEvent, MAX_FIND};

/// The fields a run exposes under `{run.…}`.
pub const RUN_FIELDS: &[&str] = &["id", "started_at"];

/// The `(scope, field)` pairs an expression names. A reference without a scope has the scope `""`.
pub fn expr_refs(expr: &str) -> Vec<(String, String)> {
    let mut refs = crate::formula::formula_field_refs(expr).unwrap_or_default();
    refs.sort();
    refs.into_iter()
        .map(|r| match r.split_once('.') {
            Some((scope, field)) => (scope.to_string(), field.to_string()),
            None => (String::new(), r),
        })
        .collect()
}

fn app<'a>(manifests: &'a [AppManifest], slug: &str) -> Option<&'a AppManifest> {
    manifests.iter().find(|m| m.slug == slug)
}

fn table<'a>(manifests: &'a [AppManifest], slug: &str, table_id: &str) -> Option<&'a AppTable> {
    app(manifests, slug)?.tables.iter().find(|t| t.id == table_id)
}

/// Every action a flow runs, in order, with the id of the step that holds it.
fn flow_steps(flow: &AutomationRule) -> Vec<(&str, &ActionType)> {
    scaffoldry_core::workflow::all_steps(&flow.steps)
        .into_iter()
        .filter_map(|s| match &s.kind {
            StepKind::Service { action } => Some((s.id.as_str(), action)),
            StepKind::UserTask { .. } | StepKind::Branch { .. } => None,
        })
        .collect()
}

/// The id under which later steps name what an action found or created.
fn produces(action: &ActionType) -> Option<(&str, &str, &str)> {
    match action {
        ActionType::FindRecord { id, app, table, .. } | ActionType::CreateRecord { id, app, table, .. } => Some((id, app, table)),
        _ => None,
    }
}

/// The source app's table the trigger record belongs to: the only table, or the first that holds every trigger field
/// the flow names, or the first table.
pub fn trigger_table(flow: &AutomationRule, manifests: &[AppManifest]) -> Option<String> {
    let m = app(manifests, &flow.app_slug)?;
    if m.tables.len() == 1 {
        return Some(m.tables[0].id.clone());
    }
    let mut names: Vec<String> = flow.predicates.iter().map(|p| p.field_name.clone()).collect();
    for s in scaffoldry_core::workflow::all_steps(&flow.steps) {
        names.extend(s.when.iter().map(|p| p.field_name.clone()));
    }
    for (_, action) in flow_steps(flow) {
        for e in exprs(action) {
            names.extend(expr_refs(e).into_iter().filter(|(s, f)| s == "trigger" && f != "id").map(|(_, f)| f));
        }
    }
    m.tables
        .iter()
        .find(|t| names.iter().all(|n| t.fields.iter().any(|f| &f.name == n)))
        .or(m.tables.first())
        .map(|t| t.id.clone())
}

fn exprs(action: &ActionType) -> Vec<&Expr> {
    match action {
        ActionType::FindRecord { equals, .. } => vec![equals],
        ActionType::CreateRecord { fields, .. } | ActionType::UpdateRecord { fields, .. } => fields.iter().map(|(_, e)| e).collect(),
        ActionType::StartFlow { inputs, .. } => inputs.iter().map(|(_, e)| e).collect(),
        _ => vec![],
    }
}

/// Grants in the order the flow first touches each table, each list in the order the flow first names each field.
struct Grants(Vec<FlowGrant>);

impl Grants {
    fn at(&mut self, app: &str, table: &str) -> &mut FlowGrant {
        if let Some(i) = self.0.iter().position(|g| g.app_slug == app && g.table_id == table) {
            return &mut self.0[i];
        }
        self.0.push(FlowGrant { app_slug: app.to_string(), table_id: table.to_string(), ..Default::default() });
        self.0.last_mut().expect("pushed")
    }
    fn add(list: &mut Vec<String>, field: &str) {
        if !list.iter().any(|f| f == field) {
            list.push(field.to_string());
        }
    }
    fn read(&mut self, at: &(String, String), field: &str) {
        Self::add(&mut self.at(&at.0, &at.1).read, field);
    }
    fn write(&mut self, at: &(String, String), field: &str) {
        Self::add(&mut self.at(&at.0, &at.1).write, field);
    }
    fn link(&mut self, at: &(String, String), field: &str) {
        Self::add(&mut self.at(&at.0, &at.1).link, field);
    }
}

/// Where a flow's names point: the trigger table, and each finding or creating step's table.
struct Places {
    trigger: (String, String),
    steps: Vec<(String, (String, String))>,
    manifests: Vec<AppManifest>,
}

impl Places {
    fn new(flow: &AutomationRule, manifests: &[AppManifest]) -> Self {
        let steps = flow_steps(flow).into_iter().filter_map(|(_, a)| produces(a)).map(|(id, app, t)| (id.to_string(), (app.to_string(), t.to_string()))).collect();
        Places { trigger: (flow.app_slug.clone(), trigger_table(flow, manifests).unwrap_or_default()), steps, manifests: manifests.to_vec() }
    }
    fn step(&self, id: &str) -> Option<&(String, String)> {
        self.steps.iter().find(|(s, _)| s == id).map(|(_, p)| p)
    }
    /// The table a target's records are in.
    fn target(&self, t: &Target) -> Option<(String, String)> {
        match t {
            Target::Trigger => Some(self.trigger.clone()),
            Target::Created { step } | Target::Found { step } => self.step(step).cloned(),
            Target::Linked { link_field } => {
                let f = table(&self.manifests, &self.trigger.0, &self.trigger.1)?.fields.iter().find(|f| &f.name == link_field)?;
                Some((self.trigger.0.clone(), f.link_target_table.clone()?))
            }
        }
    }
}

/// Every field a flow reads, writes, creates in or links, by table. Stored with the flow version; the run checks every
/// read and write against it.
pub fn derive_grants(flow: &AutomationRule, manifests: &[AppManifest]) -> Vec<FlowGrant> {
    let places = Places::new(flow, manifests);
    let mut g = Grants(Vec::new());
    let trigger = places.trigger.clone();
    if let TriggerEvent::FieldChanged { field_name } = &flow.trigger {
        g.read(&trigger, field_name);
    }
    for p in &flow.predicates {
        g.read(&trigger, &p.field_name);
    }
    let refs = |g: &mut Grants, e: &Expr| {
        for (scope, field) in expr_refs(e) {
            if scope == "run" || scope == "input" || field == "id" {
                continue;
            }
            let at = if scope == "trigger" { Some(trigger.clone()) } else { places.step(&scope).cloned() };
            if let Some(at) = at {
                g.read(&at, &field);
            }
        }
    };
    let target_reads = |g: &mut Grants, t: &Target| {
        if let Target::Linked { link_field } = t {
            g.read(&trigger, link_field);
        }
    };
    for s in scaffoldry_core::workflow::all_steps(&flow.steps) {
        for p in &s.when {
            g.read(&trigger, &p.field_name);
        }
        let StepKind::Service { action } = &s.kind else { continue };
        match action {
            ActionType::FindRecord { app, table, field, equals, .. } => {
                refs(&mut g, equals);
                g.read(&(app.clone(), table.clone()), field);
            }
            ActionType::CreateRecord { app, table, fields, .. } => {
                for (_, e) in fields {
                    refs(&mut g, e);
                }
                let at = (app.clone(), table.clone());
                g.at(app, table).create = true;
                for (f, _) in fields {
                    g.write(&at, f);
                }
            }
            ActionType::UpdateRecord { target, fields } => {
                for (_, e) in fields {
                    refs(&mut g, e);
                }
                target_reads(&mut g, target);
                if let Some(at) = places.target(target) {
                    for (f, _) in fields {
                        g.write(&at, f);
                    }
                }
            }
            ActionType::SetLink { target, link_field, to } => {
                target_reads(&mut g, target);
                target_reads(&mut g, to);
                if let Some(at) = places.target(target) {
                    g.link(&at, link_field);
                }
            }
            ActionType::UpdateRecordStatus { .. } => g.write(&trigger, "status"),
            // What a call passes on is read here, inside this flow's grants.
            ActionType::StartFlow { inputs, .. } => {
                for (_, e) in inputs {
                    refs(&mut g, e);
                }
            }
            _ => {}
        }
    }
    g.0
}

fn table_name(manifests: &[AppManifest], g: &FlowGrant) -> String {
    table(manifests, &g.app_slug, &g.table_id).map(|t| t.name.clone()).unwrap_or_else(|| g.table_id.clone())
}

fn app_title(manifests: &[AppManifest], slug: &str) -> String {
    app(manifests, slug).map(|m| m.title.clone()).unwrap_or_else(|| slug.to_string())
}

/// A flow's grants in plain sentences, for the change list. A reviewer approves what the flow may touch.
pub fn grants_sentence(grants: &[FlowGrant], _source: &str, manifests: &[AppManifest]) -> String {
    let listed = |pick: &dyn Fn(&FlowGrant) -> Vec<String>| {
        grants
            .iter()
            .filter_map(|g| {
                let fields = pick(g);
                (!fields.is_empty()).then(|| format!("{}: {}", table_name(manifests, g), fields.join(", ")))
            })
            .collect::<Vec<_>>()
            .join("; ")
    };
    let mut out = Vec::new();
    let read = listed(&|g| g.read.clone());
    if !read.is_empty() {
        out.push(format!("This flow may read {read}."));
    }
    let created: Vec<String> = grants.iter().filter(|g| g.create).map(|g| format!("{} / {}", app_title(manifests, &g.app_slug), table_name(manifests, g))).collect();
    if !created.is_empty() {
        out.push(format!("It may create records in {}.", created.join(" and ")));
    }
    let set = listed(&|g| g.write.iter().chain(g.link.iter()).cloned().collect());
    if !set.is_empty() {
        out.push(format!("It may set {set}."));
    }
    if out.is_empty() {
        return "This flow may not read or change anything.".to_string();
    }
    out.push("It may not change anything else.".to_string());
    out.join(" ")
}

/// Every problem that stops a flow from being saved. Empty means it can be saved.
pub fn validate_flow(flow: &AutomationRule, manifests: &[AppManifest]) -> Vec<String> {
    let mut problems = Vec::new();
    let places = Places::new(flow, manifests);
    let ids: Vec<&str> = flow_steps(flow).into_iter().filter_map(|(_, a)| produces(a).map(|p| p.0)).collect();
    for s in scaffoldry_core::workflow::all_steps(&flow.steps) {
        if let StepKind::UserTask { approve, reject, .. } = &s.kind {
            if approve.iter().chain(reject.iter()).any(ActionType::is_flow) {
                problems.push(format!("step '{}': a flow action must be a step of its own, not the result of a decision", s.id));
            }
        }
    }
    let field_of = |at: &(String, String), name: &str| table(manifests, &at.0, &at.1).and_then(|t| t.fields.iter().find(|f| f.name == name)).cloned();
    let writable = |problems: &mut Vec<String>, step: &str, at: &(String, String), fields: &[(String, Expr)]| {
        for (f, _) in fields {
            match field_of(at, f) {
                None => problems.push(format!("step '{step}': the table '{}' has no field '{f}'", at.1)),
                Some(spec) if spec.field_type.is_computed() => problems.push(format!("step '{step}': '{f}' is computed, so nothing writes it")),
                Some(_) => {}
            }
        }
    };
    let target_ok = |problems: &mut Vec<String>, step: &str, t: &Target, position: usize| -> Option<(String, String)> {
        match t {
            Target::Created { step: s } | Target::Found { step: s } => match ids.iter().position(|i| i == s) {
                None => {
                    problems.push(format!("step '{step}' names '{s}', which is not a step of this flow"));
                    return None;
                }
                Some(p) if p >= position => {
                    problems.push(format!("step '{step}' names step '{s}', which comes later"));
                    return None;
                }
                Some(_) => {}
            },
            Target::Linked { link_field } => {
                if field_of(&places.trigger, link_field).is_none_or(|f| f.field_type != FieldType::Link) {
                    problems.push(format!("step '{step}': '{link_field}' is not a link field of the trigger's table"));
                    return None;
                }
            }
            Target::Trigger => {}
        }
        places.target(t)
    };
    let mut produced = 0usize;
    for s in scaffoldry_core::workflow::all_steps(&flow.steps) {
        let StepKind::Service { action } = &s.kind else { continue };
        let step = s.id.as_str();
        // References: qualified, known, and earlier.
        for e in exprs(action) {
            if let Err(err) = crate::formula::validate_formula(e) {
                problems.push(format!("step '{step}': the expression {e} is not valid: {}", err.message));
                continue;
            }
            for (scope, field) in expr_refs(e) {
                let reference = format!("{{{scope}.{field}}}");
                match scope.as_str() {
                    "" => problems.push(format!("step '{step}': write {{trigger.{field}}} or {{step_id.{field}}}, not {{{field}}}")),
                    "run" if !RUN_FIELDS.contains(&field.as_str()) => problems.push(format!("step '{step}': {reference} is not a field of the run")),
                    "run" => {}
                    "input" => {
                        let declared = matches!(&flow.trigger, TriggerEvent::Called { inputs } if inputs.iter().any(|i| i.name == field));
                        if !declared {
                            problems.push(format!("step '{step}': {reference} names an input this flow does not declare. Only a flow started by another declares inputs"));
                        }
                    }
                    "trigger" => {
                        if field != "id" && field_of(&places.trigger, &field).is_none() {
                            problems.push(format!("step '{step}': {reference} names a field the trigger's table does not have"));
                        }
                    }
                    other => match ids.iter().position(|i| *i == other) {
                        None => problems.push(format!("step '{step}': {reference} names '{other}', which is not a step of this flow")),
                        Some(p) if p >= produced => problems.push(format!("step '{step}': {reference} names step '{other}', which comes later")),
                        Some(_) => {
                            if let Some(at) = places.step(other) {
                                if field != "id" && field_of(at, &field).is_none() {
                                    problems.push(format!("step '{step}': {reference} names a field '{}' does not have", at.1));
                                }
                            }
                        }
                    },
                }
            }
        }
        match action {
            ActionType::FindRecord { app: a, table: t, field, limit, .. } => {
                if *limit == 0 || *limit > MAX_FIND {
                    problems.push(format!("step '{step}': FindRecord asks for {limit} records, and the limit is 1 to {MAX_FIND}"));
                }
                match table(manifests, a, t) {
                    None => problems.push(format!("step '{step}': there is no table '{t}' in the app '{a}'")),
                    Some(tb) => match tb.fields.iter().find(|f| &f.name == field) {
                        None => problems.push(format!("step '{step}': the table '{t}' has no field '{field}'")),
                        Some(f) if !f.indexed => problems.push(format!("step '{step}': the field '{field}' of '{t}' is not indexed. FindRecord matches an indexed field only")),
                        Some(_) => {}
                    },
                }
            }
            ActionType::CreateRecord { app: a, table: t, fields, .. } => match table(manifests, a, t) {
                None => problems.push(format!("step '{step}': there is no table '{t}' in the app '{a}'")),
                Some(_) => writable(&mut problems, step, &(a.clone(), t.clone()), fields),
            },
            ActionType::UpdateRecord { target, fields } => {
                if let Some(at) = target_ok(&mut problems, step, target, produced) {
                    writable(&mut problems, step, &at, fields);
                }
            }
            ActionType::SetLink { target, link_field, to } => {
                let from = target_ok(&mut problems, step, target, produced);
                let to_at = target_ok(&mut problems, step, to, produced);
                if let (Some(from), Some(to_at)) = (from, to_at) {
                    match field_of(&from, link_field) {
                        Some(f) if f.field_type == FieldType::Link => {
                            if from.0 != to_at.0 || f.link_target_table.as_deref() != Some(to_at.1.as_str()) {
                                problems.push(format!("step '{step}': '{link_field}' links to '{}', not to the records it names", f.link_target_table.unwrap_or_default()));
                            }
                        }
                        _ => problems.push(format!("step '{step}': '{link_field}' is not a link field of '{}'", from.1)),
                    }
                }
            }
            _ => {}
        }
        if produces(action).is_some() {
            produced += 1;
        }
    }
    problems
}

/// Process v2 phase 3: when something `days` whole days after `from` is due, in the zone `tz`. It is the start of the
/// day that many days after `from`'s own day there. A zone the platform does not know counts as UTC.
pub fn due_at(from: chrono::DateTime<chrono::Utc>, days: u32, tz: &str) -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone;
    let zone: chrono_tz::Tz = tz.parse().unwrap_or(chrono_tz::UTC);
    let day = from.with_timezone(&zone).date_naive() + chrono::Duration::days(days as i64);
    let midnight = day.and_hms_opt(0, 0, 0).expect("midnight exists");
    match zone.from_local_datetime(&midnight) {
        chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) => t.with_timezone(&chrono::Utc),
        // A day that starts in a gap starts an hour later.
        chrono::LocalResult::None => zone.from_local_datetime(&(midnight + chrono::Duration::hours(1))).earliest().map(|t| t.with_timezone(&chrono::Utc)).unwrap_or(from),
    }
}

/// `at` written with `fmt` in the zone `tz`. A zone the platform does not know counts as UTC.
pub fn local_format(at: chrono::DateTime<chrono::Utc>, tz: &str, fmt: &str) -> String {
    let zone: chrono_tz::Tz = tz.parse().unwrap_or(chrono_tz::UTC);
    at.with_timezone(&zone).format(fmt).to_string()
}

/// The latest time at or before `now` that a schedule falls on, in the zone `tz`. `None` when `at` or the zone is not
/// valid, or a day is out of range.
pub fn last_occurrence(every: &scaffoldry_core::workflow::ScheduleEvery, at: &str, tz: &str, now: chrono::DateTime<chrono::Utc>) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::{Datelike, Duration, NaiveTime, TimeZone};
    use scaffoldry_core::workflow::ScheduleEvery;
    let zone: chrono_tz::Tz = tz.parse().ok()?;
    let time = NaiveTime::parse_from_str(at, "%H:%M").ok()?;
    let local = now.with_timezone(&zone).date_naive();
    // The candidate days, newest first, far enough back to hold one occurrence of any schedule.
    let days = (0..62).map(|back| local - Duration::days(back));
    let falls = |d: &chrono::NaiveDate| match every {
        ScheduleEvery::Daily => true,
        ScheduleEvery::Weekly { day } => d.weekday().number_from_monday() == u32::from(*day),
        ScheduleEvery::Monthly { day } => d.day() == u32::from(*day),
    };
    days.filter(falls)
        .filter_map(|d| zone.from_local_datetime(&d.and_time(time)).earliest())
        .map(|t| t.with_timezone(&chrono::Utc))
        .find(|t| *t <= now)
}

/// What is wrong with a schedule, if anything.
pub fn schedule_problems(every: &scaffoldry_core::workflow::ScheduleEvery, at: &str) -> Vec<String> {
    use scaffoldry_core::workflow::ScheduleEvery;
    let mut out = Vec::new();
    if chrono::NaiveTime::parse_from_str(at, "%H:%M").is_err() {
        out.push(format!("the schedule's time '{at}' is not a time like 07:00"));
    }
    match every {
        ScheduleEvery::Weekly { day } if !(1..=7).contains(day) => out.push(format!("the weekday {day} is not 1 (Monday) to 7 (Sunday)")),
        ScheduleEvery::Monthly { day } if !(1..=28).contains(day) => out.push(format!("the day of the month {day} is not 1 to 28, which every month has")),
        _ => {}
    }
    out
}
