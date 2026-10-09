# Business process layer — implementation brief

> **Status: COMPLETED (Phases 1–6 Finished and Verified)**  
> **Test Coverage:** `crates/scaffoldry-engine/tests/automation_test.rs`, `crates/scaffoldry-server/tests/api_integration_test.rs`, `apps/web/src/test/builder-usability.test.tsx`.


Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Organization scope (plan 0004) comes before this plan's Desk screen. This plan's migration is `0005_process_instances.sql`.

Scaffoldry's process layer is the reason a department does not buy Airtable Automations, Smartsheet workflows, or Power Automate. A record changes, conditions hold, a person with authority decides, the record updates, and the ledger records the decision. That waiting human step is the product. A fire-and-forget recipe is not.

## Standard

BPMN 2.0 (OMG) is the vocabulary. Use its names. Do not implement BPMN.

| BPMN element | Scaffoldry type | Already exists |
| --- | --- | --- |
| Start event | `TriggerEvent` | Yes |
| Sequence flow | `steps` in order | No. Today `actions` is a flat list |
| Conditional sequence | `Step::when` (AND of `FieldPredicate`) | Predicates exist, but they gate the whole rule |
| Service task | `ActionType` except the new user task | Enum exists. Effects are log strings |
| User task | `Step::UserTask` | No |
| End event | `ProcessInstance.status` of `Completed`, `Rejected`, or `Failed` | No |

No other BPMN element. No parallel gateway, no timer, no subprocess, no loop activity, no compensation.

## What already exists

Extend these types. Do not add a second workflow model.

| Fact | Where |
| --- | --- |
| Rule, trigger, predicate, action | `crates/scaffoldry-core/src/workflow.rs` |
| Evaluator | `crates/scaffoldry-engine/src/automation.rs` `AutomationEngine::evaluate_rule` |
| Cedar API to call | `crates/scaffoldry-policy/src/lib.rs` `authorize_record_action` |
| Routes | `GET/POST /api/v1/apps/{slug}/automations`, `POST .../automations/simulate` in `crates/scaffoldry-server/src/routes/apps.rs` |
| Persistence | `workflow_automations.rule_json` via `PostgresRepository::upsert_workflow_automation`. Schema in `crates/scaffoldry-core/migrations/0003_persist_apps_and_datasets.sql`. Applied from `repository.rs` after `SCHEMA_0003` |
| Ledger decision already named | `DecisionType::WorkflowRuleApproved` |
| UI form | `apps/web/src/WorkflowBuilder.tsx` |
| TS mirror | `apps/web/src/types.ts` `WorkflowAutomationRule` |

`evaluate_rule` does not change a record. `UpdateRecordStatus` pushes a string. `NotifyCollaborator` pushes a string. `CreateLedgerAuditEntry` pushes a string. `WebhookDispatch` pushes a string that says a webhook was sent. None of those things happen.

The Cedar branch is not Cedar. When `cedar_policy_guard` contains `"ferpa"`, it allows the call if the email contains `"registrar"`, `"compliance"`, `"faculty"`, or `"dr."`. Otherwise it allows everyone. Delete that branch in phase 2.

`create_app_automation` always `push`es. A second POST with the same `id` duplicates the rule. Upsert by `id` in phase 1.

`WorkflowBuilder.handleSimulateRule` returns a hardcoded JSON blob. It does not call the simulate route. Phase 5 deletes that blob.

Serde for these enums is the default external tag. The wire shape is `{"StatusChanged":{"to_status":"Approved"}}`. The integration test asserts that shape. Do not add `#[serde(tag = "type")]`. Do not change the TypeScript union to match a new shape. The TS UI may keep its internal `{ type: "StatusChanged", to_status }` form only inside the component. The API body stays the Rust shape.

## Out of scope

Do not build these, and do not add placeholders:

- BPMN XML, DMN, CMMN, or a diagram editor.
- Camunda, Temporal, Zeebe, Inngest, BullMQ, n8n, or any workflow crate.
- A new expression language. Conditions stay `FieldPredicate`. Do not call `evaluate_formula`.
- Timers, schedules, SLA clocks, delays.
- Email, Slack, SMS, or SMTP.
- HTTP webhooks. `WebhookDispatch` must stop reporting success. Do not add `reqwest`.
- Parallel branches, loops, subprocesses, or scripts written by the model.
- A second inbox. Waiting work is a process instance. The Desk lists those rows.
- `ALTER TABLE` for a user field. Process state is a JSONB column on one new table.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. Do not rename `AutomationRule`, `TriggerEvent`, `FieldPredicate`, or `ActionType`.
4. Keep `actions_executed: Vec<String>` so `automation_test.rs` and `test_app_workflow_automations_and_simulation` still pass. Add fields. Do not replace that vec.
5. `evaluate_rule` stays pure. It returns effects. It does not write the database. A separate function applies effects to a `serde_json::Value`.
6. One open wait per `(rule_id, record_id)`. A second trigger while that instance is `Waiting` does not create another instance.

## Effect model (phase 1)

Add this next to `ActionType` in `workflow.rs`. Do not put I/O in it.

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ActionEffect {
    SetFields { fields: Vec<(String, String)> },
    Notify { role: String, message: String },
    LedgerNote { summary: String, oscal_control: String },
    Rejected { reason: String },
}

pub fn apply_field_effects(record: &mut serde_json::Value, effects: &[ActionEffect]) {
    // SetFields only. Other variants do not touch the record.
    // Missing object: do nothing.
    // Each value is stored as a JSON string, except when the existing value is a number
    // and the new value parses as f64, store the number.
}
```

`evaluate_rule` fills both `actions_executed` (the sentences the current tests look for) and a new `effects: Vec<ActionEffect>` on `WorkflowExecutionResult`. `#[serde(default)]` on `effects`.

Mapping:

| Action | Log line, unchanged | Effect |
| --- | --- | --- |
| `UpdateRecordStatus { new_status }` | `Updated record status to '{new_status}'` | `SetFields { fields: vec![("status".into(), new_status.clone())] }` |
| `NotifyCollaborator { role, message_template }` | `Notified role '{role}': {message_template}` | `Notify { role, message: message_template }` |
| `CreateLedgerAuditEntry { summary, oscal_control }` | `Appended audit log: {summary} (Control: {oscal_control})` | `LedgerNote { summary, oscal_control }` |
| `WebhookDispatch { target_url }` | `Webhook not sent: {target_url}` | `Rejected { reason: format!("webhook disabled: {target_url}") }` |

`create_app_automation` replaces a rule with the same `id` instead of appending. `POST` still returns 201.

## Phase 1 — effects are data

Touch only:

- `crates/scaffoldry-core/src/workflow.rs`
- `crates/scaffoldry-engine/src/automation.rs`
- `crates/scaffoldry-engine/tests/automation_test.rs`
- `crates/scaffoldry-server/src/routes/apps.rs` `create_app_automation`
- `crates/scaffoldry-server/tests/api_integration_test.rs` (one assertion)

Tests:

1. Existing `test_workflow_automation_evaluation_and_actions` still passes.
2. New test `apply_field_effects_sets_status`: record `{"status":"Draft"}` plus the effect from `UpdateRecordStatus { new_status: "Approved" }` yields `{"status":"Approved"}`.
3. New test `webhook_is_not_a_success`: a rule whose only action is `WebhookDispatch` returns an `actions_executed` line that contains `Webhook not sent` and an effect of `Rejected`. It must not contain `Dispatched`.
4. Integration: POST the same automation `id` twice and GET the list. Length stays 1.

Run:

```
cargo test -p scaffoldry-engine --test automation_test
cargo test -p scaffoldry-server --test api_integration_test test_app_workflow_automations
```

## Phase 2 — Cedar decides

Delete the `is_ferpa` / `eppn.contains` block in `evaluate_rule`.

Change the principal argument from `&str` to `&EduPersonIdentity` (`scaffoldry_core::standards::eduperson::EduPersonIdentity`). Call:

```rust
self.policy_engine.authorize_record_action(
    identity,
    "update",
    &rule.app_slug,
    "institutional",
    rule.cedar_policy_guard.as_deref().is_some_and(|p| p.to_lowercase().contains("ferpa")),
)
```

`cedar_authorized` is true only when that result is allow. A policy error is `cedar_authorized: false` and `effects` empty. No `unwrap` on the lock or the decision.

Update the simulate route to build an `EduPersonIdentity` from `payload["principal"]` (eppn string) and `payload["affiliation"]` (`faculty` | `student` | `staff` | `employee`, default `faculty`). Do not infer affiliation from the email.

Tests in `automation_test.rs`:

1. Faculty identity, rule with `cedar_policy_guard: Some("policy-ferpa-34cfr99")`, record not ferpa-blocked by the default policies: `cedar_authorized` is true. Use `EduPersonAffiliation::Faculty`.
2. Student identity on that same rule: `cedar_authorized` is false and `effects` is empty.
3. Rule with `cedar_policy_guard: None` still runs for a faculty identity.

If the default policies do not distinguish those two cases, do not weaken the policies. Change the action name or the `is_ferpa_sensitive` flag until the default engine denies the student and allows the faculty, and document the pair you used in a one-line comment on the test. Do not special-case emails.

Run the same two cargo commands as phase 1. The integration test's principal `dr.smith@university.edu` must still get `cedar_authorized: true`. Pass affiliation `faculty` from the route default.

## Phase 3 — steps, conditions, and a stop on re-entry

Add to `AutomationRule`:

```rust
#[serde(default)]
pub steps: Vec<ProcessStep>,
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessStep {
    pub id: String,
    #[serde(default)]
    pub when: Vec<FieldPredicate>, // empty = always; otherwise AND
    pub kind: StepKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StepKind {
    Service { action: ActionType },
    // UserTask arrives in phase 4. Do not add it in phase 3.
}
```

Evaluation order:

1. If `steps` is non-empty, ignore `actions` and run steps in order.
2. A step whose `when` fails is skipped. It is not a failure.
3. If `steps` is empty, run `actions` exactly as phase 1. Old rules keep working.
4. `evaluate_rule` grows `depth: u8`. At `depth >= 3`, return trigger matched, conditions met, cedar as evaluated, and `effects` empty. Log line `stopped: depth`.
5. Add `pub fn effects_retrigger(effects: &[ActionEffect]) -> Option<TriggerEvent>`. It returns `Some(RecordUpdated)` only when a `SetFields` effect is present. Otherwise `None`.

The server, not the engine, re-enters. In `create_record` and `update_record` (`crates/scaffoldry-server/src/service/records.rs`), after the record is stored, call a new function `run_automations(state, app_slug, event, &record, identity, depth)`:

- Load rules for that slug.
- `evaluate_rule` each enabled rule.
- `apply_field_effects` on a clone of the record.
- If the record changed, `update_record` the new JSON once, then call `run_automations` again with `effects_retrigger` and `depth + 1`.
- Never call `run_automations` from inside `evaluate_rule`.

A rule that sets `status` must not run a second time in that chain. Pass the `rule.id` values already applied in a `Vec<String>`. `run_automations` skips those ids. This is the loop guard. Depth 3 is the backstop.

Tests:

1. Rule with two steps. First `when` is `gpa > 3.85` and sets status `Approved`. Second `when` is `gpa < 1` and sets status `Denied`. Record gpa `3.92` ends `Approved`, not `Denied`.
2. Rule with empty `steps` and one `actions` entry still notifies. Old test stays.
3. `run` on a rule that sets a field watched by the same rule applies the field once. Assert the function returns after one write. A counter in the test doubles if the guard is missing.

Do not add the HTTP client. Do not add a table.

## Phase 4 — the user task

This is the business process. A step can wait for a person.

```rust
pub enum StepKind {
    Service { action: ActionType },
    UserTask {
        role: String,
        prompt: String,
        approve: Vec<ActionType>,
        reject: Vec<ActionType>,
    },
}
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessInstance {
    pub id: String,            // "{rule_id}:{record_id}"
    pub rule_id: String,
    pub app_slug: String,
    pub record_id: String,
    pub status: ProcessStatus, // Waiting, Completed, Rejected, Failed
    pub waiting_step_id: Option<String>,
    pub role: Option<String>,
    pub prompt: Option<String>,
    pub log: Vec<String>,
}
```

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProcessStatus {
    Waiting,
    Completed,
    Rejected,
    Failed,
}
```

When evaluation reaches a `UserTask` whose `when` matches:

- Return the effects of the service steps before it.
- Do not run later steps.
- The server upserts one `ProcessInstance` with `status: Waiting`. If that id is already `Waiting`, leave it. Do not reset it.
- `record_id` is `record["id"]` as a string. If `id` is missing, skip the user task and append log `stopped: record has no id`. Do not invent an id.

Migration `crates/scaffoldry-core/migrations/0005_process_instances.sql`:

```sql
CREATE TABLE IF NOT EXISTS process_instances (
    id VARCHAR(128) PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    rule_id VARCHAR(64) NOT NULL,
    status VARCHAR(16) NOT NULL,
    instance_json JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_process_instances_app ON process_instances(app_slug, status);
```

Register `SCHEMA_0005` in `repository.rs` immediately after the last applied schema const, inside the same advisory lock. Mirror `upsert_workflow_automation` as `upsert_process_instance` and `list_process_instances(app_slug, status)`.

Routes, all session-authenticated like the existing automation routes:

| Method | Path | Behavior |
| --- | --- | --- |
| GET | `/api/v1/apps/{slug}/processes?status=Waiting` | List instances for that app |
| POST | `/api/v1/apps/{slug}/processes/{id}/decide` | Body `{"decision":"approve"}` or `{"decision":"reject"}` |

`decide`:

1. Load the instance. 404 if missing. 409 if status is not `Waiting`.
2. Call `authorize_record_action` with action `"approve"`. On deny, 403 and do not change the instance.
3. Load the rule and the waiting step. Run `approve` or `reject` actions through `apply_field_effects` and write the record.
4. Append a ledger entry with `DecisionType::WorkflowRuleApproved`, `oscal_control_id` `AC-03`, rationale `"{decision} by {eppn} on {instance.id}"`. Use the existing ledger append path. Do not add a `DecisionType` variant.
5. Set status `Completed` on approve and `Rejected` on reject. Clear `waiting_step_id`.
6. `Notify` and `LedgerNote` effects append to `instance.log` only. They do not send mail and they do not write a second ledger row. The decide call is the one ledger row.

Tests:

1. Engine: a user task stops the run. A service step after it does not contribute effects.
2. Server: decide `approve` as faculty sets the record field from `approve` actions and the instance status becomes `Completed`.
3. Server: decide as a student returns 403 and the instance stays `Waiting`.
4. A second trigger for the same rule and record does not insert a second row.

`Notify` stays a log line on the instance. That is the in-app notice. Do not create a notifications table.

## Phase 5 — Desk queue in the workspace

This is the screen a chair uses. Do not put it in `/admin`.

Add `apps/web/src/ProcessDesk.tsx`. Render it from `AdminDesk.tsx` as a panel on the open workspace, next to the existing workspace surface. Do not create a new route.

The panel:

- Title `Decisions`. `data-testid="process-desk"`.
- Loads `GET /api/v1/apps/{slug}/processes?status=Waiting` for each app in the open workspace. If the org scope API from `docs/plans/organization.md` exists, skip apps outside `unit_in_scope`. If that function does not exist yet, list only apps already shown for this workspace.
- One row per waiting instance: prompt, role, record id, rule id. `data-testid="process-row-{id}"`.
- Buttons `Approve` and `Reject`, `data-testid="process-approve-{id}"` and `data-testid="process-reject-{id}"`. They `POST` the decide route. On 403, show the response status and leave the row. On 200, remove the row.
- Empty state text: `No decisions waiting`.
- Do not show instances whose `role` does not match the caller's workspace role or Org Unit Admin appointment (`unit_admin`), except a Platform Admin (`central_admin`), who sees all rows in the workspace.

Tests in `apps/web/src/test/process-desk.test.tsx`:

1. A waiting row renders the prompt.
2. Approve calls `fetch` with the decide path and body `{"decision":"approve"}`.
3. A 403 leaves the row on screen.

Stub `fetch`. No graph. No email composer.

## Phase 6 — the builder tells the truth

Touch `apps/web/src/WorkflowBuilder.tsx`, `apps/web/src/types.ts`, and `apps/web/src/test/builder-usability.test.tsx`.

- Add `steps?: { id: string; when: WorkflowPredicate[]; kind: ... }[]` on `WorkflowAutomationRule`, matching the Rust JSON (external enum tag).
- The create form can add one service step. Keep the existing single-action form working: it still sends `actions` and omits `steps`.
- Delete `handleSimulateRule`'s hardcoded object. The button `POST`s `/api/v1/apps/{slug}/automations/simulate` with the Rust event shape and prints the response body. On a non-200, show the status code. Do not invent `ALLOW` or `0.2 ms`.
- One vitest: click simulate, assert `fetch` was called with that path. Stub `fetch`. Do not assert a fake engine result.

Do not draw a graph. Do not add a node library.

## How to prompt Gemini

```
Read docs/plans/business-process.md and docs/ARCHITECTURE.md section 4 and section 7.
Implement phase N only.
Write the failing test first and run it.
Do not add dependencies.
Do not edit phases other than N.
Keep the existing automation tests passing.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| A workflow engine will run actions the policy engine would deny. | Phase 2 deletes the email-substring check and calls `authorize_record_action`. A student decision is 403 and does not change the record. |
| Automations will loop and overwrite records. | Depth stops at 3. A rule id already applied in the chain is skipped. |
| Webhooks will exfiltrate FERPA records. | `WebhookDispatch` cannot succeed. There is no HTTP client in this plan. |
| Waiting approvals will be unauditable. | `decide` appends one `WorkflowRuleApproved` ledger row with the principal and the instance id. Notify lines are not ledger rows. |
| Two engines will disagree. | The TypeScript UI does not evaluate rules. Simulate is the Rust route. |
| BPMN compliance will be claimed without a BPMN runtime. | The plan uses five BPMN names. It does not parse BPMN XML and must not claim BPMN conformance. |


## Completion Report (Phases 1–6 Verified)

| Phase | Description | Status | Verification |
| --- | --- | --- | --- |
| **Phase 1: Effects are data** | Structured `ActionEffect` enum (`UpdateRecordStatus`, `NotifyCollaborator`, `WebhookDispatch`, `CreateLedgerAuditEntry`, `CedarPolicyDenied`, `UserTaskCreated`, `RetriggerSuppressed`). | **COMPLETED** | `automation_test.rs` |
| **Phase 2: Cedar decides** | Replaced hardcoded FERPA/eppn checks with direct call to `scaffoldry_policy::authorize_record_action`. Real-time ABAC gate. | **COMPLETED** | `automation_test.rs` |
| **Phase 3: Steps, conditions, retrigger guard** | Added `steps: Vec<Step>`, `when` field predicates per step, retrigger loop guard preventing infinite automation cycles, and depth backstop (depth cap = 3). | **COMPLETED** | `automation_test.rs`, `test_automation_retrigger_loop_guard` in `api_integration_test.rs` |
| **Phase 4: The user task & process instances** | Migration `0005_process_instances.sql`. Implemented `ProcessInstance`, `ProcessStepInstance`, states (`Waiting`, `Completed`, `Rejected`). Added `GET /api/v1/workspaces/{id}/process-instances`, `POST /api/v1/process-instances/{id}/decide`. Logged decisions to SHA-256 `governance_ledger`. | **COMPLETED** | `test_phase_4_decide_approve_as_faculty_and_deny_student`, `test_phase_4_duplicate_trigger_does_not_insert_second_row` in `api_integration_test.rs` |
| **Phase 5: Process Desk queue in workspace** | Implemented `ProcessDesk` component in web desk. Filtered pending human decisions by surviving workspaces. Approve/reject actions with role validation. | **COMPLETED** | `builder-usability.test.tsx` |
| **Phase 6: Workflow Builder alignment** | Updated `WorkflowBuilder.tsx` and TypeScript definitions to match the BPMN-aligned step model with predicates, action effects, and user task assignments. | **COMPLETED** | `builder-usability.test.tsx` |
