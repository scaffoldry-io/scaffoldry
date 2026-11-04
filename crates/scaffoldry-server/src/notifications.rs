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
/// A workspace, by id. Rendered as its name, or "a workspace".
#[derive(Debug, Clone)]
pub struct WorkspaceRef(pub String);

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
    Assigned => "assigned" { by: PersonRef, app: AppRef, table: TableRef, record: RecordRef },
    DelegationReceived => "delegation_received" { by: PersonRef },
    AccessChanged => "access_changed" { by: PersonRef, workspace: WorkspaceRef },
    ViewShared => "view_shared" { by: PersonRef, app: AppRef, table: TableRef },
    // A rule's Notify effect (notifications phase 4). The rule's message text is not shown: a notice holds names only.
    ProcessNotice => "process_notice" { app: AppRef, table: TableRef },
}

/// Who is told.
#[derive(Debug, Clone)]
pub enum Recipients {
    Person(String),
    /// Named people, such as a record's creator and the person who started its process.
    People(Vec<String>),
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
            | Notice::ViewShared { app, .. }
            | Notice::ProcessNotice { app, .. } => Some(app.0.as_str()),
            Notice::JobFinished { .. } | Notice::DelegationReceived { .. } | Notice::AccessChanged { .. } => None,
        }
    }
}

/// Notifications phase 3: where a kind goes for a person.
pub const CHANNELS: &[&str] = &["in_app", "email", "both", "digest", "off"];

/// Kinds a person can never set below `in_app`: a decision waiting on them, a change to their own access, an
/// infected file. Each with the reason the settings pane shows.
pub const FORCED: &[(&str, &str)] = &[
    ("decision_waiting", "A decision waiting on you always reaches your inbox, so nothing stalls unseen."),
    ("access_changed", "A change to your own access always reaches your inbox."),
    ("file_infected", "An infected file always reaches your inbox."),
];

/// The channel when neither the person nor the institution chose one.
pub const DEFAULT_CHANNEL: &str = "both";

pub fn is_forced(kind: &str) -> bool {
    FORCED.iter().any(|(k, _)| *k == kind)
}

/// Each kind in plain words, for the settings pane.
pub fn label(kind: &str) -> &'static str {
    match kind {
        "mention" => "Someone mentions you in a comment",
        "decision_waiting" => "A decision is waiting on you",
        "decision_made" => "A decision is made on something you started",
        "process_escalated" => "A step is escalated",
        "process_overdue" => "A step is overdue",
        "proposal_pending" => "A proposal needs deciding",
        "proposal_decided" => "Your proposal is decided",
        "job_finished" => "Your background work finishes",
        "import_finished" => "Your import finishes",
        "sync_failed" => "A sync fails",
        "webhook_disabled" => "A webhook is turned off",
        "file_infected" => "A file is found infected",
        "assigned" => "You are assigned to something",
        "delegation_received" => "Someone asks you to decide for them",
        "access_changed" => "Your access changes",
        "view_shared" => "Someone shares a view with you",
        "process_notice" => "A rule notifies you",
        _ => "Other notices",
    }
}

/// Why a channel cannot be used for a kind, or nothing when it can.
pub fn channel_problem(kind: &str, channel: &str) -> Option<String> {
    if !KINDS.contains(&kind) {
        return Some(format!("There is no kind of notice called {kind}"));
    }
    if !CHANNELS.contains(&channel) {
        return Some(format!("A channel is one of {}", CHANNELS.join(", ")));
    }
    if channel == "off" && is_forced(kind) {
        let reason = FORCED.iter().find(|(k, _)| *k == kind).map(|(_, r)| *r).unwrap_or("");
        return Some(format!("You cannot turn this off. {reason}"));
    }
    None
}

/// Checks the setting `notifications.defaults`: an object from kind to channel, with no forced kind off.
pub fn valid_defaults(value: &Value) -> bool {
    value.as_object().is_some_and(|o| o.iter().all(|(k, v)| v.as_str().is_some_and(|c| channel_problem(k, c).is_none())))
}

/// Each recipient's channel for `kind`: their own choice, else the institution's, else `both`. A forced kind is never
/// below `in_app`, whatever is stored.
fn channels_for(state: &SharedState, kind: &str, people: &[String]) -> Result<Vec<String>, RepositoryError> {
    let default = state
        .settings
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .get("notifications.defaults")
        .and_then(|d| d.get(kind))
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_CHANNEL)
        .to_string();
    let lower: Vec<String> = people.iter().map(|p| p.to_lowercase()).collect();
    let kind_s = kind.to_string();
    let chosen: Vec<(String, String)> = state.repository.with_client(move |c| {
        Ok(c.query("SELECT lower(eppn), channel FROM notification_prefs WHERE lower(eppn) = ANY($1) AND kind = $2", &[&lower, &kind_s])?
            .iter()
            .map(|r| (r.get(0), r.get(1)))
            .collect())
    })?;
    Ok(people
        .iter()
        .map(|p| {
            let c = chosen.iter().find(|(e, _)| e.eq_ignore_ascii_case(p)).map(|(_, c)| c.clone()).unwrap_or_else(|| default.clone());
            if c == "off" && is_forced(kind) {
                "in_app".to_string()
            } else {
                c
            }
        })
        .collect())
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
    let step = scaffoldry_core::workflow::find_step(&rule.steps, &step_id)?;
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
        Notice::Assigned { by, app: a, table: t, record: r } => (
            format!("Assigned to you: {}", app(a)),
            format!("{} added you to a record in {}.", who(by), table(a, t)),
            link(format!("{}?record={}", app_path(a), urlencode(&r.0)), json!({ "app": a.0, "record": r.0 })),
        ),
        Notice::DelegationReceived { by } => (
            format!("{} delegated to you", who(by)),
            format!("{} asked you to decide in their place for a while. Their decisions now come to you too.", who(by)),
            link("/settings".to_string(), json!({})),
        ),
        Notice::AccessChanged { by, workspace } => {
            let name = state.workspaces.read().ok().and_then(|w| w.get(&workspace.0).map(|x| x.name.clone())).unwrap_or_else(|| "a workspace".to_string());
            (
                format!("Your access changed: {name}"),
                format!("{} changed what you may do in {name}.", who(by)),
                link(format!("/workspace/{}", urlencode(&workspace.0)), json!({ "workspace": workspace.0 })),
            )
        }
        Notice::ProcessNotice { app: a, table: t } => (
            format!("A rule in {} notified you", app(a)),
            format!("A rule in {} asked that you be told about a change in {}.", app(a), table(a, t)),
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
        Recipients::People(list) => (list.clone(), None),
        Recipients::Approvers(instance) => {
            let Some(inst) = state.repository.get_process_instance(instance).ok().flatten() else { return (Vec::new(), None) };
            crate::service::approvers::current_approvers(state, &inst)
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
    let users = state.repository.users_by_names(&resolved).unwrap_or_default();
    let people: Vec<Person> = users
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
    // Notifications phase 3: each person's channel for this kind. `off` creates nothing.
    let channels = channels_for(state, notice.kind(), &told)?;
    let (told, channels): (Vec<String>, Vec<String>) = told.into_iter().zip(channels).filter(|(_, c)| c != "off").unzip();
    if told.is_empty() {
        return Ok(0);
    }
    let (title, body, link) = render(state, notice);
    let (kind, app) = (notice.kind().to_string(), notice.app().map(str::to_string));
    let ids: Vec<uuid::Uuid> = told.iter().map(|_| uuid::Uuid::new_v4()).collect();
    // Mail goes only when it is on and complete. Then each notice's mail is queued in the same transaction.
    let base_url = crate::mail::mailing(&state.settings.read().unwrap_or_else(|p| p.into_inner())).ok();
    // With mail off, a mail-only choice still reaches the inbox, so nothing is lost.
    let mail_on = base_url.is_some();
    let read: Vec<bool> = channels.iter().map(|c| c == "email" && mail_on).collect();
    let states: Vec<String> = channels.iter().map(|c| if c == "digest" { "digest".to_string() } else { "none".to_string() }).collect();
    let directory: Vec<(String, Option<String>)> =
        told.iter().map(|e| (e.clone(), users.iter().find(|u| u.user_name.eq_ignore_ascii_case(e)).and_then(crate::service::people::primary_email))).collect();
    state.repository.with_client(move |c| {
        let mut tx = c.transaction()?;
        let n = tx.execute(
            "INSERT INTO notifications (id, recipient, kind, app_slug, title, body, link, read_at, email_state) \
             SELECT t.id, t.recipient, v.kind, v.app, v.title, v.body, v.link, CASE WHEN t.read THEN NOW() END, t.state \
             FROM unnest($1::uuid[], $2::varchar[], $8::bool[], $9::varchar[]) AS t(id, recipient, read, state) \
             CROSS JOIN (SELECT $3::varchar AS kind, $4::varchar AS app, $5::varchar AS title, $6::varchar AS body, $7::jsonb AS link) AS v",
            &[&ids, &told, &kind, &app, &title, &body, &link, &read, &states],
        )?;
        if let Some(base) = base_url {
            for ((id, (eppn, email)), channel) in ids.iter().zip(&directory).zip(&channels) {
                // Only email and both are mailed now. A digest notice waits for the morning.
                if channel != "email" && channel != "both" {
                    continue;
                }
                if let Some(to) = crate::mail::address_of(&mut tx, eppn, email.as_deref())? {
                    crate::mail::queue(&mut tx, Some(*id), &to, &title, &crate::mail::compose(&title, &body, &base, *id))?;
                }
            }
        }
        tx.commit()?;
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

/// Notifications phase 4: who a rule's Notify effect names. A position key reaches its holders from the workspace's
/// unit up, and their delegates. Anything else is a workspace role, as before.
pub fn notify_role(state: &SharedState, app: &str, table: &str, role: &str, actor: &str) {
    let is_position = state.position_types.read().map(|p| p.contains_key(role)).unwrap_or(false);
    let to = if is_position {
        let spec = scaffoldry_core::approver::ApproverSpec::Position { key: role.to_string(), from: scaffoldry_core::approver::Origin::Workspace, walk_up: true };
        let ctx = crate::service::approvers::context_for(state, app, None, None, &[]);
        Recipients::People(scaffoldry_core::approver::resolve_approvers(&spec, &ctx).approvers.into_iter().map(|a| a.eppn).collect())
    } else {
        use scaffoldry_engine::HostRouter;
        let ws = state.engine.read().ok().and_then(|e| e.resolve_by_slug(app).and_then(|m| m.workspace_id.clone()));
        match ws {
            Some(workspace) => Recipients::Workspace { workspace, roles: vec![role.to_string()] },
            None => return,
        }
    };
    let _ = notify(state, &to, Some(actor), &Notice::ProcessNotice { app: AppRef(app.to_string()), table: TableRef(table.to_string()) });
}

/// What a finished job tells its owner (notifications phase 4): `job_finished`, or `import_finished` for an import,
/// or `file_infected` to the uploader and the workspace's admins when a scan finds a virus.
pub fn on_job_finished(state: &SharedState, job: &crate::jobs::FinishedJob) {
    if job.kind == "scan_file" {
        if job.result.as_ref().and_then(|r| r.get("scan_state")).and_then(Value::as_str) == Some("infected") {
            let file = job.payload.get("ref_id").and_then(Value::as_str).and_then(|r| uuid::Uuid::parse_str(r).ok()).and_then(|id| state.repository.file_ref_any(id).ok().flatten());
            if let Some(f) = file {
                let notice = Notice::FileInfected { app: AppRef(f.app_slug.clone()), table: TableRef(f.table_id.clone().unwrap_or_default()), record: RecordRef(f.record_id.clone().unwrap_or_default()) };
                let _ = notify(state, &Recipients::Person(f.uploaded_by.clone()), None, &notice);
                use scaffoldry_engine::HostRouter;
                let ws = state.engine.read().ok().and_then(|e| e.resolve_by_slug(&f.app_slug).and_then(|m| m.workspace_id.clone()));
                if let Some(workspace) = ws {
                    let _ = notify(state, &Recipients::Workspace { workspace, roles: vec!["owner".into(), "admin".into()] }, Some(&f.uploaded_by), &notice);
                }
            }
        }
        return;
    }
    // Work the platform started for itself has nobody to tell.
    if !job.created_by.contains('@') {
        return;
    }
    let to = Recipients::Person(job.created_by.clone());
    let notice = match (job.kind.as_str(), &job.result) {
        ("import_run", Some(r)) if !job.failed => {
            let n = |k: &str| r.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
            Notice::ImportFinished {
                app: AppRef(job.payload.get("app_slug").and_then(Value::as_str).unwrap_or_default().to_string()),
                table: TableRef(job.payload.get("table_id").and_then(Value::as_str).unwrap_or_default().to_string()),
                rows: Count(n("created_count") + n("updated_count")),
                refused: Count(n("rejected_count")),
            }
        }
        _ => Notice::JobFinished { job: JobRef(job.id), failed: Flag(job.failed) },
    };
    let _ = notify(state, &to, None, &notice);
}

/// The job runner's hook, for `JobConfig::on_finish`.
pub fn job_hook(state: SharedState) -> crate::jobs::OnFinish {
    crate::jobs::OnFinish(std::sync::Arc::new(move |job: &crate::jobs::FinishedJob| on_job_finished(&state, job)))
}
