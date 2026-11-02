//! Notifications phase 1: the inbox.
//!
//! A notice carries names and links, never a record value. It is built only by `render`, from a `Notice` whose every
//! argument is a reference the server resolves (an app, a table, a person, a step, a record id, a job), a count, or a
//! yes or no. No argument is free text, so a caller has nowhere to put a record's content. A reference that names
//! nothing the server holds renders as a plain word ("an app", "Someone"), never as what was passed.
//!
//! `notify` resolves who is told, removes the actor and the submitter, removes anyone inactive or on hold, de-duplicates,
//! and creates nothing for someone who may not read the app the notice is about, so a notice cannot reveal an app.

use crate::repository::{PostgresRepository, RepositoryError};
use crate::state::SharedState;
use scaffoldry_core::approver::Person;
use scaffoldry_engine::HostRouter;
use serde_json::{json, Value};

/// An app, by slug. Rendered as the app's title, or "an app" when the server holds no such app.
#[derive(Debug, Clone)]
pub struct AppRef(pub String);
/// A table of the notice's app, by id. Rendered as its name, or "a table".
#[derive(Debug, Clone)]
pub struct TableRef(pub String);
/// A person, by address. Rendered as their display name, or "Someone".
#[derive(Debug, Clone)]
pub struct PersonRef(pub String);
/// A record, by id. It goes in the link, never in the text.
#[derive(Debug, Clone)]
pub struct RecordRef(pub String);
/// A process instance's waiting step, by instance id. Rendered as the step's prompt from its definition.
#[derive(Debug, Clone)]
pub struct StepRef(pub String);
/// A background job, by id. Rendered as its kind's title.
#[derive(Debug, Clone)]
pub struct JobRef(pub uuid::Uuid);
/// A number.
#[derive(Debug, Clone, Copy)]
pub struct Count(pub u32);
/// A yes or no.
#[derive(Debug, Clone, Copy)]
pub struct Flag(pub bool);

/// Defines the closed list of kinds, each with its typed arguments, and the list of those arguments that a test reads.
macro_rules! notices {
    ($( $variant:ident => $kind:literal { $( $field:ident : $ty:ident ),* $(,)? } ),* $(,)?) => {
        /// A notice to send: its kind and its arguments. Every argument is a reference, a count, or a yes or no.
        #[derive(Debug, Clone)]
        pub enum Notice { $( $variant { $( $field: $ty ),* } ),* }

        impl Notice {
            pub fn kind(&self) -> &'static str {
                match self { $( Notice::$variant { .. } => $kind ),* }
            }
        }

        /// The closed list of kinds.
        pub const KINDS: &[&str] = &[ $( $kind ),* ];

        /// Each kind's arguments, as `(name, type)`, from the same definition as `Notice`.
        pub const FIELDS: &[(&str, &[(&str, &str)])] = &[ $( ($kind, &[ $( (stringify!($field), stringify!($ty)) ),* ]) ),* ];
    };
}

notices! {
    Mention => "mention" { by: PersonRef, app: AppRef, table: TableRef, record: RecordRef },
    DecisionWaiting => "decision_waiting" { app: AppRef, table: TableRef, step: StepRef, waiting: Count },
    DecisionMade => "decision_made" { by: PersonRef, app: AppRef, table: TableRef, step: StepRef, approved: Flag },
    ProcessEscalated => "process_escalated" { app: AppRef, table: TableRef, step: StepRef },
    ProcessOverdue => "process_overdue" { app: AppRef, table: TableRef, step: StepRef, days: Count },
    ProposalPending => "proposal_pending" { by: PersonRef, app: AppRef },
    ProposalDecided => "proposal_decided" { by: PersonRef, app: AppRef, approved: Flag },
    JobFinished => "job_finished" { job: JobRef, failed: Flag },
    ImportFinished => "import_finished" { app: AppRef, table: TableRef, rows: Count, refused: Count },
    SyncFailed => "sync_failed" { app: AppRef, table: TableRef },
    WebhookDisabled => "webhook_disabled" { app: AppRef },
    FileInfected => "file_infected" { app: AppRef, table: TableRef, record: RecordRef },
    Assigned => "assigned" { by: PersonRef, app: AppRef, table: TableRef, step: StepRef },
    DelegationReceived => "delegation_received" { by: PersonRef },
    AccessChanged => "access_changed" { by: PersonRef, app: AppRef },
    ViewShared => "view_shared" { by: PersonRef, app: AppRef, table: TableRef },
}

/// Who is told.
#[derive(Debug, Clone)]
pub enum Recipients {
    Person(String),
    /// The approvers of the step a process instance waits on, by instance id.
    Approvers(String),
    Position { key: String, unit: uuid::Uuid },
    Workspace { workspace: String, roles: Vec<String> },
}

impl Notice {
    /// The app a notice is about, when it is about one.
    pub fn app(&self) -> Option<&str> {
        match self {
            Notice::Mention { app, .. }
            | Notice::DecisionWaiting { app, .. }
            | Notice::DecisionMade { app, .. }
            | Notice::ProcessEscalated { app, .. }
            | Notice::ProcessOverdue { app, .. }
            | Notice::ProposalPending { app, .. }
            | Notice::ProposalDecided { app, .. }
            | Notice::ImportFinished { app, .. }
            | Notice::SyncFailed { app, .. }
            | Notice::WebhookDisabled { app }
            | Notice::FileInfected { app, .. }
            | Notice::Assigned { app, .. }
            | Notice::AccessChanged { app, .. }
            | Notice::ViewShared { app, .. } => Some(app.0.as_str()),
            Notice::JobFinished { .. } | Notice::DelegationReceived { .. } => None,
        }
    }
}

pub const MAX_TITLE: usize = 200;
pub const MAX_BODY: usize = 500;

/// Who is told, from what the resolver found: never the actor or the submitter, never anyone inactive or on hold, and
/// each person once. A person the platform holds no record of counts as active.
pub fn recipients_of(resolved: Vec<String>, actor: Option<&str>, submitter: Option<&str>, people: &[Person]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in resolved {
        let same = |o: Option<&str>| o.is_some_and(|o| o.eq_ignore_ascii_case(&e));
        if same(actor) || same(submitter) || out.iter().any(|x| x.eq_ignore_ascii_case(&e)) {
            continue;
        }
        if people.iter().any(|p| p.eppn.eq_ignore_ascii_case(&e) && (!p.active || p.on_hold)) {
            continue;
        }
        out.push(e);
    }
    out
}

/// `text` in at most `max` characters, cut at a word with an ellipsis when it is longer.
pub fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max.saturating_sub(1)).collect();
    let at_word = match head.rfind(char::is_whitespace) {
        Some(i) if i > 0 => &head[..i],
        _ => head.as_str(),
    };
    format!("{}…", at_word.trim_end())
}

fn app_title(state: &SharedState, slug: &str) -> Option<String> {
    let engine = state.engine.read().ok()?;
    engine.resolve_by_slug(slug).map(|m| m.title.clone())
}

fn table_name(state: &SharedState, slug: &str, table: &str) -> Option<String> {
    let engine = state.engine.read().ok()?;
    let m = engine.resolve_by_slug(slug)?;
    m.tables.iter().find(|t| t.id == table).map(|t| t.name.clone())
}

fn person_name(state: &SharedState, eppn: &str) -> String {
    state
        .repository
        .users_by_names(&[eppn.to_string()])
        .ok()
        .and_then(|u| u.first().map(crate::service::people::display_name))
        .unwrap_or_else(|| "Someone".to_string())
}

/// The prompt of the step an instance waits on, from its process definition.
fn step_prompt(state: &SharedState, instance: &str) -> Option<(String, String)> {
    let inst = state.repository.get_process_instance(instance).ok().flatten()?;
    let rule = crate::service::processes::rule_for(state, &inst)?;
    let step_id = inst.waiting_step_id.clone().or_else(|| rule.steps.first().map(|s| s.id.clone()))?;
    let step = rule.steps.iter().find(|s| s.id == step_id)?;
    match &step.kind {
        scaffoldry_core::workflow::StepKind::UserTask { prompt, .. } => Some((prompt.clone(), inst.record_id.clone())),
        _ => None,
    }
}

fn job_title(state: &SharedState, job: uuid::Uuid) -> String {
    crate::jobs::get_job(&state.repository, job)
        .ok()
        .flatten()
        .and_then(|j| crate::jobs::JOB_KINDS.iter().find(|k| k.name == j.kind).map(|k| k.title.to_string()))
        .unwrap_or_else(|| "Background work".to_string())
}

/// Builds a notice's title, body and link from its references. The only way a notice gets words.
pub fn render(state: &SharedState, notice: &Notice) -> (String, String, Value) {
    let app = |a: &AppRef| app_title(state, &a.0).unwrap_or_else(|| "an app".to_string());
    let table = |a: &AppRef, t: &TableRef| table_name(state, &a.0, &t.0).unwrap_or_else(|| "a table".to_string());
    let who = |p: &PersonRef| person_name(state, &p.0);
    let step = |s: &StepRef| step_prompt(state, &s.0);
    let app_path = |a: &AppRef| format!("/app/{}", urlencode(&a.0));
    let link = |path: String, extra: Value| {
        let mut l = json!({ "path": path });
        if let (Some(o), Some(e)) = (l.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        l
    };
    let plural = |n: u32, one: &str, many: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
    let (title, body, link) = match notice {
        Notice::Mention { by, app: a, table: t, record } => (
            format!("{} mentioned you in {}", who(by), app(a)),
            format!("{} mentioned you in a comment on a record in {}.", who(by), table(a, t)),
            link(format!("{}?record={}", app_path(a), urlencode(&record.0)), json!({ "app": a.0, "record": record.0 })),
        ),
        Notice::DecisionWaiting { app: a, table: t, step: s, waiting } => {
            let prompt = step(s).map(|(p, _)| format!("Review \"{p}\" for {}.", table(a, t))).unwrap_or_else(|| format!("A decision waits for you in {}.", table(a, t)));
            (
                format!("Decision needed: {}", app(a)),
                format!("{prompt} {} waiting.", if waiting.0 == 1 { "1 is".to_string() } else { format!("{} are", waiting.0) }),
                link(format!("{}?instance={}", app_path(a), urlencode(&s.0)), json!({ "app": a.0, "instance": s.0 })),
            )
        }
        Notice::DecisionMade { by, app: a, table: t, step: s, approved } => (
            format!("{} {}: {}", who(by), if approved.0 { "approved" } else { "declined" }, app(a)),
            format!("{} decided a step in {}: {}.", who(by), table(a, t), if approved.0 { "approved" } else { "declined" }),
            link(format!("{}?instance={}", app_path(a), urlencode(&s.0)), json!({ "app": a.0, "instance": s.0 })),
        ),
        Notice::ProcessEscalated { app: a, table: t, step: s } => (
            format!("Escalated to you: {}", app(a)),
            format!("A step in {} waited too long and now comes to you.", table(a, t)),
            link(format!("{}?instance={}", app_path(a), urlencode(&s.0)), json!({ "app": a.0, "instance": s.0 })),
        ),
        Notice::ProcessOverdue { app: a, table: t, step: s, days } => (
            format!("Overdue: {}", app(a)),
            format!("A step in {} has waited {}.", table(a, t), plural(days.0, "day", "days")),
            link(format!("{}?instance={}", app_path(a), urlencode(&s.0)), json!({ "app": a.0, "instance": s.0 })),
        ),
        Notice::ProposalPending { by, app: a } => (
            format!("Proposal waiting: {}", app(a)),
            format!("{} proposed a change to {}. It waits for a decision.", who(by), app(a)),
            link("/proposals".to_string(), json!({ "app": a.0 })),
        ),
        Notice::ProposalDecided { by, app: a, approved } => (
            format!("Proposal {}: {}", if approved.0 { "approved" } else { "rejected" }, app(a)),
            format!("{} {} the proposed change to {}.", who(by), if approved.0 { "approved" } else { "rejected" }, app(a)),
            link(app_path(a), json!({ "app": a.0 })),
        ),
        Notice::JobFinished { job, failed } => (
            format!("{} {}", job_title(state, job.0), if failed.0 { "failed" } else { "finished" }),
            if failed.0 { "The background work did not finish. Open it to see what happened.".to_string() } else { "The background work you started has finished.".to_string() },
            link(format!("/me/jobs/{}", job.0), json!({ "job": job.0 })),
        ),
        Notice::ImportFinished { app: a, table: t, rows, refused } => (
            format!("Import finished: {}", app(a)),
            format!("{} added to {}. {} refused.", plural(rows.0, "row", "rows"), table(a, t), plural(refused.0, "row", "rows")),
            link(app_path(a), json!({ "app": a.0 })),
        ),
        Notice::SyncFailed { app: a, table: t } => (
            format!("Sync failed: {}", app(a)),
            format!("The connection that fills {} could not run. Its last good data stays.", table(a, t)),
            link(app_path(a), json!({ "app": a.0 })),
        ),
        Notice::WebhookDisabled { app: a } => (
            format!("Webhook turned off: {}", app(a)),
            format!("A webhook of {} failed too many times in a row and was turned off.", app(a)),
            link(app_path(a), json!({ "app": a.0 })),
        ),
        Notice::FileInfected { app: a, table: t, record } => (
            format!("A file was blocked: {}", app(a)),
            format!("A file attached to a record in {} failed the virus scan and cannot be opened.", table(a, t)),
            link(format!("{}?record={}", app_path(a), urlencode(&record.0)), json!({ "app": a.0, "record": record.0 })),
        ),
        Notice::Assigned { by, app: a, table: t, step: s } => (
            format!("Assigned to you: {}", app(a)),
            format!("{} assigned you a step in {}.", who(by), table(a, t)),
            link(format!("{}?instance={}", app_path(a), urlencode(&s.0)), json!({ "app": a.0, "instance": s.0 })),
        ),
        Notice::DelegationReceived { by } => (
            format!("{} delegated to you", who(by)),
            format!("{} asked you to decide in their place for a while. Their decisions now come to you too.", who(by)),
            link("/settings".to_string(), json!({})),
        ),
        Notice::AccessChanged { by, app: a } => (
            format!("Your access changed: {}", app(a)),
            format!("{} changed what you may do in {}.", who(by), app(a)),
            link(app_path(a), json!({ "app": a.0 })),
        ),
        Notice::ViewShared { by, app: a, table: t } => (
            format!("{} shared a view with you", who(by)),
            format!("{} shared a view of {} in {}.", who(by), table(a, t), app(a)),
            link(app_path(a), json!({ "app": a.0 })),
        ),
    };
    (cut(&title, MAX_TITLE), cut(&body, MAX_BODY), link)
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The addresses a specification names, before any filtering, and the submitter to leave out.
fn resolve(state: &SharedState, to: &Recipients) -> (Vec<String>, Option<String>) {
    match to {
        Recipients::Person(e) => (vec![e.clone()], None),
        Recipients::Approvers(instance) => {
            let Some(inst) = state.repository.get_process_instance(instance).ok().flatten() else { return (Vec::new(), None) };
            match crate::service::approvers::resolution_for(state, &inst) {
                Some((_, ctx, resolution)) => (resolution.approvers.into_iter().map(|a| a.eppn).collect(), ctx.submitter.clone()),
                None => (Vec::new(), None),
            }
        }
        Recipients::Position { key, unit } => {
            let spec = scaffoldry_core::approver::ApproverSpec::Position { key: key.clone(), from: scaffoldry_core::approver::Origin::Workspace, walk_up: false };
            (crate::service::approvers::resolved_approvers(state, &spec, *unit, &[], None), None)
        }
        Recipients::Workspace { workspace, roles } => (
            crate::service::members::people_records(workspace, state)
                .into_iter()
                .filter(|c| roles.iter().any(|r| r.eq_ignore_ascii_case(&c.role)))
                .map(|c| c.eppn)
                .collect(),
            None,
        ),
    }
}

/// Tells the people `to` names about `notice`. Returns how many notices were created.
pub fn notify(state: &SharedState, to: &Recipients, actor: Option<&str>, notice: &Notice) -> Result<usize, RepositoryError> {
    let (resolved, submitter) = resolve(state, to);
    if resolved.is_empty() {
        return Ok(0);
    }
    let people: Vec<Person> = state
        .repository
        .users_by_names(&resolved)
        .unwrap_or_default()
        .iter()
        .map(|u| Person { eppn: u.user_name.clone(), display_name: crate::service::people::display_name(u), active: u.active, on_hold: u.admin_hold })
        .collect();
    let mut told = recipients_of(resolved, actor, submitter.as_deref(), &people);
    // A notice about an app goes only to people who may read it, so it cannot reveal the app.
    if let Some(slug) = notice.app() {
        told.retain(|e| {
            crate::service::identity::resolve_user(e, state)
                .is_some_and(|u| crate::service::access::authorize_app(&u, slug, crate::service::access::AppAction::Read, state).is_ok())
        });
    }
    if told.is_empty() {
        return Ok(0);
    }
    let (title, body, link) = render(state, notice);
    let (kind, app) = (notice.kind().to_string(), notice.app().map(str::to_string));
    let ids: Vec<uuid::Uuid> = told.iter().map(|_| uuid::Uuid::new_v4()).collect();
    state.repository.with_client(move |c| {
        let n = c.execute(
            "INSERT INTO notifications (id, recipient, kind, app_slug, title, body, link) SELECT * FROM unnest($1::uuid[], $2::varchar[]) AS t(id, recipient) \
             CROSS JOIN (SELECT $3::varchar, $4::varchar, $5::varchar, $6::varchar, $7::jsonb) AS v",
            &[&ids, &told, &kind, &app, &title, &body, &link],
        )?;
        Ok(n as usize)
    })
}

/// Deletes read notices older than 90 days and any older than 365. Returns how many.
pub fn purge(repo: &PostgresRepository) -> Result<u64, RepositoryError> {
    repo.with_client(|c| {
        Ok(c.execute(
            "DELETE FROM notifications WHERE (read_at IS NOT NULL AND created_at < NOW() - INTERVAL '90 days') OR created_at < NOW() - INTERVAL '365 days'",
            &[],
        )?)
    })
}

/// The `purge_notifications` job, run daily.
pub fn purge_job(ctx: &crate::jobs::JobCtx, _payload: Value) -> Result<Value, crate::jobs::JobError> {
    let n = purge(ctx.repository()).map_err(|e| crate::jobs::JobError::Retry(e.to_string()))?;
    Ok(json!({ "removed": n }))
}
