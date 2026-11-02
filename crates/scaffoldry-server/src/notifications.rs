//! Notifications phase 1: the inbox. (Stub bodies: the tests come first.)

use crate::repository::{PostgresRepository, RepositoryError};
use crate::state::ServerState;
use scaffoldry_core::approver::Person;
use serde_json::Value;

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

pub fn recipients_of(_resolved: Vec<String>, _actor: Option<&str>, _submitter: Option<&str>, _people: &[Person]) -> Vec<String> {
    Vec::new()
}

pub fn cut(text: &str, _max: usize) -> String {
    text.to_string()
}

pub fn render(_state: &ServerState, _notice: &Notice) -> (String, String, Value) {
    (String::new(), String::new(), Value::Null)
}

pub fn notify(_state: &ServerState, _to: &Recipients, _actor: Option<&str>, _notice: &Notice) -> Result<usize, RepositoryError> {
    Ok(0)
}

pub fn purge(_repo: &PostgresRepository) -> Result<u64, RepositoryError> {
    Ok(0)
}
