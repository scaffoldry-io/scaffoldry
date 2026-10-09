# Flows and My Work — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. This brief needs `process-v2.md` phase 1, `mcp-apps.md` phase 4a, `approvers.md` phase 3, `access-rules.md` phase 3, `record-history.md` phase 1, `notifications.md` phase 1, and `jobs.md` phase 2. It changes `process-v2.md` phase 2, which runs after this brief's phase 2.

Two things are missing from the plans.

First, a flow cannot cross apps. A rule belongs to one app. It triggers on that app's records and writes only to that app's tables. A real process does not stay inside one app. An approved admission opens a record in the financial aid app and a task in housing. A grant award creates a budget in finance and tells the department.

Second, a person has no single list of what is waiting on them. The workspace panel shows approvals for the open workspace. A chair in four workspaces must open four. Only an agent can ask for all of them at once. And a task today can only mean "approve or reject". It cannot mean "provide the missing document".

This brief adds flows that span apps and workspaces under approved grants, a visual editor for them, tasks of more than one kind, and **My Work**: one list, tied to the person, across the whole institution.

## Standard

BPMN 2.0 names stay the vocabulary: a user task, a service task, an exclusive gateway, a timer. The editor draws a block-structured subset of BPMN. Scaffoldry does not implement BPMN and does not claim conformance.

## Decisions already made

| Question | Answer |
| --- | --- |
| What do flows run as? | A service identity for the flow, with grants a reviewer reads as sentences. Never the triggering person's rights |
| Can a flow cross workspaces? | Yes. A flow that touches more than one workspace needs approval from an owner or admin of each. A setting can switch cross-workspace flows off |
| Can a flow scan another app's records? | No. A flow reaches records by address: the triggering record, a record linked to it, a record it created, or a record it finds by an exact key, with at most 100 results |
| Can a flow delete? | Never |
| Can a flow loop? | No. Flows may start other flows, and a cycle across flows fails validation. A chain is at most 5 deep |
| Does the editor allow free-form diagrams? | No. It draws a block-structured flow with branches that rejoin. There are no loops and no parallel paths, so the layout is deterministic |
| Is the canvas the only way to edit? | No. An outline view edits the same model with every operation the canvas has. Both dispatch the same commands |
| Where is the person's list? | A top-level page, My Work. It lists decisions, tasks, reviews, and what the person is waiting on, across every app they may act in |
| Can an assignee who is not a workspace member act? | Only on the one record and only on the fields the step lists, while the step is open |

## What exists today

| Fact | Where |
| --- | --- |
| A rule belongs to one app, with `app_slug` | `workflow.rs`, `workflow_automations` |
| Rules are versioned and changed by proposal | `mcp-apps.md` phase 4a |
| Cross-table effects inside one app run as the triggering person under `run_automation` | `process-v2.md` phase 2 |
| A user task approves or rejects | `workflow.rs` |
| The Decisions panel lists waiting items for the open workspace | `business-process.md` phase 5 |
| `list_waiting_decisions` is the only cross-app list, for agents | `mcp-apps.md` phase 5 |
| The builder is a form plus an outline. A graph is forbidden | `business-process.md`, `process-v2.md` |
| Proposals have one workspace and one approver set | `mcp-apps.md` phase 4 |

## Out of scope

- Parallel branches that run at once and rejoin.
- Loops, and a flow that waits for another flow to finish.
- Arbitrary code in a step.
- Deleting records from a flow.
- Importing a diagram from another tool.
- A mobile layout for the canvas. The outline works on narrow screens.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency. The canvas is hand-written SVG.
3. A flow's writes go through `write_record` with the flow as the actor and the triggering person recorded as `on_behalf_of`.
4. An effect that is not inside an approved grant is refused at run time, and the run fails with the reason. A test proves it.
5. Every list shown to a person is computed by the server. The browser never decides who may act.
6. Use the kit for every screen.

## Phase 1 — flows, grants, and addresses

Migration `crates/scaffoldry-core/migrations/0055_flows.sql`:

```sql
ALTER TABLE workflow_automations ALTER COLUMN app_slug DROP NOT NULL;
ALTER TABLE workflow_automations ADD COLUMN IF NOT EXISTS workspace_id VARCHAR(64);
ALTER TABLE workflow_rule_versions ALTER COLUMN app_slug DROP NOT NULL;
ALTER TABLE record_history ADD COLUMN IF NOT EXISTS on_behalf_of VARCHAR(255);
ALTER TABLE app_proposals ADD COLUMN IF NOT EXISTS parties JSONB;

CREATE TABLE IF NOT EXISTS proposal_approvals (
    proposal_id UUID NOT NULL,
    workspace_id VARCHAR(64) NOT NULL,
    decided_by VARCHAR(255),
    decision VARCHAR(8),
    rationale TEXT,
    at TIMESTAMPTZ,
    PRIMARY KEY (proposal_id, workspace_id)
);

CREATE TABLE IF NOT EXISTS flow_effects (
    dedupe_key VARCHAR(200) PRIMARY KEY,
    instance_id VARCHAR(128) NOT NULL,
    at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

A flow is the stored `AutomationRule`, extended. Keep the type name. Add, all with `#[serde(default)]`:

```rust
pub home_workspace: Option<String>,        // where it is owned and listed
pub grants: Vec<FlowGrant>,                // derived, stored with the version
pub struct FlowGrant {
    pub app_slug: String, pub table_id: String,
    pub read: Vec<String>, pub write: Vec<String>,
    pub create: bool, pub link: Vec<String>,
}
```

`app_slug` now names the trigger's source app and may be empty for a flow that starts on a schedule or by being called.

Addresses. Effects and reads name a target, never a query:

```rust
pub enum Target {
    Trigger,
    Linked { link_field: String },            // from the trigger record
    Created { step: String },                  // a record this run created
    Found { step: String },                    // see FindRecord
}
pub enum ActionType {
    // existing variants, plus:
    FindRecord { id: String, app: String, table: String, field: String, equals: Expr, limit: u8 },
    CreateRecord { id: String, app: String, table: String, fields: Vec<(String, Expr)> },
    UpdateRecord { target: Target, fields: Vec<(String, Expr)> },
    SetLink { target: Target, link_field: String, to: Target },
}
```

`FindRecord` matches one indexed field by exact value (case-insensitive, as the index is) and returns at most `limit` records, at most 100. It never scans. `UpdateRecord` on a `Found` target applies to each result. There is no delete variant.

Expressions. The formula language from `calc-graph.md` gains qualified references: `{trigger.field}`, `{step_id.field}`, `{run.id}`, `{run.started_at}`. The validator resolves each against the flow's steps and the target tables, and fails on an unknown or later reference.

Grants are derived, not written. `derive_grants(flow, manifests) -> Vec<FlowGrant>` walks the steps and collects: every field read in a condition, an expression, or a find, every field written, every table created in, and every link changed. It is stored with the flow version and shown in the change list as sentences:

`This flow may read Applicants: status, program. It may create records in Financial Aid / Awards. It may set Awards: amount, student. It may not change anything else.`

At run time every read and write is checked against the stored grants before it happens. An attempt outside them fails the run with `flow_grant_exceeded`. Add that code to the closed list in `ux-standards.md`.

Identity. A flow acts as the service identity `flow:{rule_id}`. `write_record` receives that as the actor, `actor_kind` `automation`, and the triggering person as `on_behalf_of`. History shows `Flow "Open aid file", started by Dr. Rivera`. Row rules and column rules from `access-rules.md` do not apply to a flow's addressed records, because the grants and the addressing are the limit. They still apply to anyone who reads what the flow wrote.

Idempotency. Every effect has a dedupe key of run id, step id, target id, and field set. The effect is recorded in `flow_effects` in the same transaction as its write, and an effect whose key exists is skipped. A retry never writes twice.

Limits, in settings (`flows.max_steps` 50, `flows.max_writes_per_run` 1,000, both in the closed list): fan-out of at most 100 per step, at most 1,000 writes per run. A run that would exceed them fails with the count.

This phase supersedes the cross-table effect text in `process-v2.md` phase 2. Existing single-app rules get grants derived once, when their next version is saved.

Tests.

1. A flow triggered by an application creates an award in another app, sets two fields, and links the award back. The history of the award shows the flow as actor and the triggering person as `on_behalf_of`.
2. `derive_grants` for the flow above returns exactly the fields it reads and writes, and its sentence is the expected text.
3. A flow step edited after approval to write a field outside the stored grants fails at run time with `flow_grant_exceeded`, and the instance shows the reason.
4. `FindRecord` with `limit` of 101 fails validation. It matches by an indexed field only. It never reads another field of non-matching records.
5. A flow with no delete variant cannot delete. A scan test proves no delete effect exists.
6. A reference to `{step_9.field}` where step 9 comes later fails validation naming it.
7. Run the same flow twice with the same run id (simulate a retry): the second writes nothing because the dedupe keys exist.
8. A person with no rights on the target app can still trigger the flow by editing a record they can edit, and cannot cause any write outside the grants.
9. A run that would write 1,001 records fails with the count and leaves the first writes in place, recorded, and the instance `Failed`.

## Phase 2 — approval from every workspace it touches

A proposal that carries a flow now has parties: the home workspace, and every other workspace in `grants`. `app_proposals.parties` lists them. `proposal_approvals` holds one row per party.

Checks, added to `mcp-apps.md` phase 4a's list:

| Check | Outcome |
| --- | --- |
| `flow_grants` | Info. The grants as sentences, per workspace |
| `cross_workspace` | Flagged when more than one workspace is touched |
| `data_movement` | Flagged when an effectively sensitive field read in one app is written into another app |
| `classification_downgrade` | `fail` when a sensitive field moves to a workspace whose `data_classification` ranks lower than its source's. The order is `Public`, `Internal`, `Restricted`, `FERPA Sensitive` |
| `flow_target_exists` | `fail` when a target table, field, or link does not exist |

Approval. `decide_proposal` now takes the workspace the caller is deciding for. A person who is an owner or admin of a party workspace approves that party. One person may approve several parties if they hold the role in each. When a flagged check is raised, the approver of the party where the data comes from must not be the proposer, except that a proposer who is the only person with the role in that workspace is refused and the proposal waits for a Platform Admin to approve that party. A rejection by any party rejects the proposal.

The proposal becomes `Approved` only when every party has approved, and the stale check runs at that moment against the then-live versions of every app and flow it touches. The final transaction writes one ledger entry per party (`AccessRoleGranted` is not reused: add `FlowPartyApproved` to `DecisionType::ALL`) and one entry for the publication with the list of parties, then writes the version. Until then the flow is not live, and the proposer sees which parties are still waiting.

Each party's owners and admins are notified when a proposal needs them (`notifications.md`, kind `proposal_pending`). The change list shows each party only what concerns it, in sentences: the Financial Aid owner sees what the flow will create and set in their app, and what it reads from elsewhere, and does not need the whole flow.

Settings, in the closed list: `flows.cross_workspace_enabled` (boolean, default true) and `flows.disabled` (array of flow ids). The first, when false, makes any proposal with more than one party fail the `cross_workspace` check. The second stops a flow at once. Both only restrict.

Tests.

1. A flow touching two workspaces creates a proposal with two parties. Approving one leaves it pending and not live. Approving both makes it live and writes two party entries and one publication entry.
2. Any one party rejecting rejects the proposal.
3. A flow that moves a sensitive field into a lower-classification workspace fails `classification_downgrade` at proposal time and stores nothing.
4. A flagged `data_movement` is refused to the proposer as source-side approver. Another owner of the source workspace can approve it.
5. A flow version changes in another party's app before the final approval: the final check is stale, the proposal is `Superseded`, and nothing is live.
6. With `flows.cross_workspace_enabled` false, a two-party proposal fails.
7. Adding a flow id to `flows.disabled` stops its next run, and a ledger entry records the setting change.
8. The Financial Aid owner's view of the change list contains only the sentences about their app.

## Phase 3 — triggers, chaining, and safety

Triggers added to `TriggerEvent`:

| Trigger | Starts when |
| --- | --- |
| `Schedule { every, at }` | A daily, weekly (a day), or monthly (a day) time in `platform.timezone`. No source app |
| `Called { inputs }` | Another flow starts it with `StartFlow`. `inputs` is a typed list of names and types |
| `FlowCompleted { flow_id, status }` | A flow ends with that status |
| `ProcessDecided { app, rule, decision }` | A user task in that rule is decided |

Steps added: `StartFlow { flow_id, inputs: Vec<(String, Expr)> }`. It starts a new instance of a `Called` flow and does not wait. Inputs are values and ids, and are typed against the called flow's declaration.

Validation across flows. A static graph of every active flow's `StartFlow` and `FlowCompleted` and `ProcessDecided` edges is checked when a version is saved. A cycle fails with the path in words: `Open aid file starts Notify housing, which starts Open aid file.` A chain at run time carries `chain_depth`. A flow started at depth 5 fails with the chain. Scheduled triggers use a job, `fire_schedules`, with a dedupe key of flow and the scheduled time, so a missed or doubled tick starts one run.

Safety. A circuit breaker: a flow that fails 20 consecutive runs is disabled in the table `flow_state` (migration `0058_flow_state.sql`, with `flow_id`, `consecutive_failures`, and `disabled_at`) and its owners are notified. An owner re-enables after fixing it. A rate cap of `flows.max_runs_per_minute` (default 600, closed list) per flow starts no new runs past it and records `rate_limited` in the run.

Tests.

1. A daily 07:00 schedule in the institution's zone starts one run per day, once, with two scheduler threads.
2. A flow that starts a `Called` flow passes typed inputs. A wrong type fails validation.
3. A cycle across three flows fails validation with the path in words.
4. A chain of 6 fails the sixth with the chain recorded.
5. Twenty consecutive failures disable the flow and notify its owners. Re-enabling clears the count.
6. `FlowCompleted` and `ProcessDecided` start the right flows with the right context.
7. A burst past the rate cap starts no more runs and logs it.

## Phase 4 — tasks of more than one kind

`UserTask.kind: Approve | Complete | Review`.

| Kind | The person does | Outcomes |
| --- | --- | --- |
| `Approve` | Decide on the record | `approve`, `reject` |
| `Complete` | Fill the listed fields and press Done | `done`, `decline` |
| `Review` | Vote and comment (committees from `process-v2.md` phase 4) | the vote outcomes |

A step lists `show` (fields the assignee may read) and, for `Complete`, `fill` (fields the assignee may write) and `require` (fields that must be non-blank to finish). `Complete` may also list a `decline_reason` requirement.

Task grants. While a step is open, each assignee holds a task grant on that one record: read of `show` and `fill`, write of `fill`. Migration `0057_task_grants.sql`:

```sql
CREATE TABLE IF NOT EXISTS task_grants (
    instance_id VARCHAR(128) NOT NULL,
    step_id VARCHAR(64) NOT NULL,
    person VARCHAR(255) NOT NULL,
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    record_id VARCHAR(64) NOT NULL,
    read_fields TEXT[] NOT NULL,
    write_fields TEXT[] NOT NULL,
    state VARCHAR(8) NOT NULL DEFAULT 'open',
    closed_at TIMESTAMPTZ,
    PRIMARY KEY (instance_id, step_id, person)
);
```

`row_scope` and `field_visibility` consult task grants: for exactly that record and those fields, an open grant counts as access, with sensitive fields still masked unless the grant lists them. A grant closes when the step closes, is decided, escalated away, reassigned, or cancelled, in the same transaction. A person's task grant never gives them the app, a list, a search, a link picker, or any other record. Writes through a task grant go through `write_record` with the person as the actor and `actor_kind` `user`, `on_behalf_of` empty, and a history batch that names the flow and step.

The validator requires `show` and `fill` to exist and `fill` to be writable fields (not computed, not read-only). A proposal change list reads: `Asks the Financial Aid officer to provide Award letter on the application. They can see Program and Status of that application only, for as long as the task is open.` A task that shows a sensitive field raises `page_sensitivity`'s sibling, `task_sensitivity`, flagged for a second person.

Tests.

1. A `Complete` task assigned to a person with no rights to the app lets them read the `show` fields and write the `fill` fields of that one record, and nothing else. A list, a search, another record, and an unlisted field are refused.
2. `Done` with a `require` field blank is refused with the field named. With it filled, the step completes and the grant closes in the same transaction.
3. `Decline` takes the decline outcome and the flow continues on that branch.
4. A reassign or an escalation closes the old grant and opens the new one, and the old person can no longer read or write.
5. A task showing a sensitive field raises `task_sensitivity`. The proposer cannot approve alone.
6. A write through a task grant is in history with the flow and step named.
7. Every path that closes a step closes its grants. A scan test lists the closing paths.

## Phase 5 — My Work: data and API

Migration `0056_work_items.sql`:

```sql
CREATE TABLE IF NOT EXISTS work_items (
    id UUID PRIMARY KEY,
    person VARCHAR(255) NOT NULL,
    kind VARCHAR(10) NOT NULL,
    state VARCHAR(8) NOT NULL DEFAULT 'open',
    app_slug VARCHAR(64),
    workspace_id VARCHAR(64),
    flow_id VARCHAR(64),
    instance_id VARCHAR(128),
    step_id VARCHAR(64),
    record_id VARCHAR(64),
    proposal_id UUID,
    title VARCHAR(200) NOT NULL,
    summary VARCHAR(300) NOT NULL,
    via VARCHAR(30) NOT NULL DEFAULT 'holder',
    due_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    closed_at TIMESTAMPTZ,
    closed_reason VARCHAR(20)
);
CREATE INDEX IF NOT EXISTS idx_work_open ON work_items (person, state, due_at) WHERE state = 'open';
CREATE INDEX IF NOT EXISTS idx_work_instance ON work_items (instance_id);
```

`kind` is `decision`, `task`, `review`, or `waiting`. `via` is `holder`, `delegate`, `assigned`, or `escalated`. A title and a summary hold names, never record values.

Maintenance, in the same transaction as the change that causes it:

- A step entering `Waiting` inserts an item for each resolved person (`Approve` and committee votes are `decision`, `Complete` is `task`), including delegates, and an item of kind `waiting` for the person who started the process and the record's creator.
- A decision, a task completion, a cancel, an escalation, a reassign, or a timeout closes the items with the reason.
- A proposal needing a party inserts a `review` item for each owner and admin of that party, except the proposer. A decision closes them. The proposer gets a `waiting` item.

Drift is repaired by jobs. Changes to positions, delegations, holds, group membership, and people (`approvers.md`, `admin-console.md`) enqueue `reresolve_work` for the affected position or unit, which recomputes the people for open instances and adds or closes items. A nightly job, `reconcile_work`, resolves every open instance live and repairs any difference, recording the count of repairs, which the Overview shows. The decision route never trusts an item: it re-checks `can_decide` live.

Routes, signed in, of the caller only.

| Method | Path | Behavior |
| --- | --- | --- |
| GET | `/me/work` | Query `kind`, `state`, `app`, `workspace`, `overdue`, `acting_for`, `cursor`, `limit`. Open items by due date then age. A person only sees items where they still pass the live read check for the app or hold a task grant |
| GET | `/me/work/summary` | Counts by kind, the overdue count, and the oldest age |
| GET | `/me/work/{id}` | The item and what is needed to act: the instance, the step's prompt, the `show` fields with values the person may see, the `fill` fields, the history of the instance in sentences, and, for a committee, the tally without names |
| POST | `/me/work/{id}/act` | Body depends on the kind: `approve` or `reject` with a comment, `done` with field values, `decline` with a reason, `vote`. It runs the same service functions as the other routes |

MCP. `list_my_work` replaces `list_waiting_decisions` (the tool budget in `agent-coverage.md` stays). Add `complete_task`. `decide_process` remains. An agent acts with the person's own rights and scope.

The realtime channel `me/events` (`realtime.md` phase 3) sends `work` with the new counts and an item id, no text.

Tests.

1. A person who holds a position in three workspaces sees an item from each in one list.
2. A delegate sees the delegator's items marked `delegate` and `acting_for`. When the delegation ends, the items go on the next re-resolve and the decide route refuses at once.
3. A person removed from a position loses their items after `reresolve_work`, and an attempt to decide before it runs is refused.
4. Deliberately corrupt an item (delete one, add a wrong one): `reconcile_work` repairs both and reports two repairs.
5. An item for an app the person can no longer read and holds no grant for does not appear.
6. A title or summary built with a record value in the fixture contains none. Every producer goes through the template builder.
7. 10,000 open items for one person page through with the index and no sequential scan.
8. The proposal `review` items exclude the proposer and close on a decision.
9. `list_my_work` and the route return the same ids for the same person.

## Phase 6 — My Work: screens

From the kit. A top-level item in the navigation, `My Work`, with the count of open decisions and tasks in its accessible name. The page `/work`:

- Tabs: `Decisions`, `Tasks`, `Reviews`, `Waiting on others`. Each has a table with title, app and workspace names, who started it, due date in words (`Due tomorrow`, `3 days overdue`), and a marker `Acting for Dr. Rivera` when it is a delegate item. Overdue is shown with text and an icon, not color alone.
- Filters: workspace, app, overdue, acting for. A search by title.
- A row opens a **review drawer** with the step's prompt, the `show` fields, the instance history in sentences, and the actions for its kind. `Approve` and `Reject` take a comment, required on reject when the setting from `process-v2.md` says so. `Done` shows the `fill` fields as a form with the required ones marked. After acting, the drawer moves to the next item in the list, with `Skip`, and says how many remain. This is the review queue for a chair with forty items.
- `Waiting on others` shows who each item is waiting on, since when, and what will happen if it is late.
- Empty states say plainly: `Nothing is waiting on you.`
- Live: `me/events` updates the counts and adds items without a reload, and keeps the person's place in the list.

The Decisions panel from `business-process.md` phase 5 is replaced by a link to `/work` filtered to the workspace. Notification links for `decision_waiting` open the review drawer.

Tests (vitest, `fetch` stubbed).

1. The four tabs show their items. Counts match the summary.
2. A delegate item shows `Acting for` text.
3. Approving moves to the next item and the remaining count goes down. A rejection without a required comment is not sent.
4. A `Complete` task form blocks `Done` until the required fields are filled and sends the values.
5. An overdue item has the text `overdue` in its accessible name.
6. A `work` event adds an item without moving the focus or the scroll position.
7. The drawer is operable by keyboard: open with Enter, act, move on, Escape returns focus to the row.

## Phase 7 — the editor: commands, outline, validation

Web, in `apps/web/src/flows/`, and one server route.

Command model. All edits go through a pure reducer: `flowReducer(state, command) -> state`. Commands: `AddStep(path, kind)`, `RemoveStep(path)`, `MoveStep(from, to)`, `UpdateStep(path, patch)`, `AddBranch(path)`, `RemoveBranch(path)`, `ReorderBranches(path, order)`, `SetTrigger(trigger)`, and `Undo` and `Redo`, with a stack of 100. Every view dispatches these and nothing else. The state is the flow's JSON, with no layout.

Validation route. `POST /flows/validate`, body a flow, authorized as app `Manage` or workspace owner or admin. It returns each problem with a path to the step, the derived grants as sentences, the parties, the checks that would be raised, and the `describe_flow` sentences. It writes nothing. The editor calls it debounced at 400 ms after an edit. The client never decides validity.

Outline editor. A nested list of steps with an inline form for each, and the operations as buttons and keyboard commands: add after, move up and down, indent into a branch, remove. It replaces the form-based builder and keeps its entry points and existing tests. It is the accessible, complete editor.

Tests.

1. Each command changes the state as expected, and `Undo` and `Redo` restore it. A fuzz of 200 random commands never produces a state the server's validator reports as structurally broken (dangling reference, unknown step kind).
2. The validate route returns a problem with the right path for a missing field, an unknown step reference, and a cycle across flows.
3. The outline can perform every command. A table in the test lists the commands and the outline control that issues each.
4. Editing a step shows the validation result within the debounce, and a problem is announced.
5. The existing builder tests pass against the outline.

## Phase 8 — the editor: canvas

A second view of the same state. Hand-written SVG in `apps/web/src/flows/Canvas.tsx`, no library.

- Layout. A pure function `layout(flow) -> Boxes and Edges`. Steps stack top to bottom. A branch is drawn as a decision node with its branches in columns side by side and a join where they rejoin. Block structure means the layout needs no crossing edges and no manual positions, so it is deterministic and the same flow always draws the same.
- Nodes. Trigger, service step (with an icon and one sentence), user task (approver sentence and due date), decision, wait, called flow, end. Text in every node. Color shows kind and is never the only signal.
- Editing. An insertion point `+` between steps opens a menu grouped Data, People, Logic, Time, and Connect. Selecting a node opens an inspector panel with the same typed form the outline uses. Drag a node to move it by mouse. Alt with arrow keys moves it by keyboard. Delete removes it. Every action dispatches a command.
- Navigation. Zoom with `+` and `-` and `Fit`. Pan by arrow keys when nothing is selected. A minimap is not built.
- Problems. A node with a problem shows an icon and the message in its accessible name, and the first problem is announced.
- The canvas and outline are switched by a tab and stay in sync, because both read the one state.

Tests.

1. `layout` is deterministic: the same flow gives the same boxes twice, and a branch of three columns places its steps without overlap.
2. Dragging a node to a new place dispatches `MoveStep`, and the outline shows the same order.
3. Adding a step through `+` yields the same state as adding it through the outline.
4. A branch with a problem shows the icon and the message in its accessible name.
5. Every canvas operation has a keyboard equivalent. A test performs a full edit (add, change, move, remove) with the keyboard alone.
6. At 1,000 pixels wide, a flow of 40 steps renders in under 200 ms in the test environment (record the figure).

## Phase 9 — test run

`POST /flows/{id}/test-run`, body a record id or sample values, and an optional `as_person` that only the author may use for resolving approvers. It runs the flow in dry mode: conditions and expressions are evaluated, `FindRecord` reads run, effects are not written, and user tasks are assumed decided by a chosen outcome (a parameter for each task, defaulting to the first). It returns a trace: for each step, `ran`, `skipped (condition was false)`, or `would write`, and the targets in plain words (`would create 1 record in Financial Aid / Awards and set Amount and Student`), the approvers each task would go to, and the first failure. It writes nothing, sends nothing, and creates no work items or notifications.

Values in the trace are shown only for records and fields the tester may read. A foreign app's values appear as counts and ids unless the tester can read them.

The canvas shows the trace on the nodes: the path taken highlighted and marked in text, skipped steps dimmed and marked, and each node's outcome in its detail. The outline shows the same as a list. A `Run again with` choice lets the person try another sample record.

Tests.

1. A test run of a flow with a branch reports which branch ran and which were skipped, and writes nothing. Check the tables are unchanged.
2. `FindRecord` in a test run reads real data, and a foreign app's values are hidden from a tester who cannot read it.
3. A user task shows who it would go to, using the real resolver for the sample record.
4. A failure in a step stops the trace and shows the reason.
5. A test run creates no work item, notification, or history entry.
6. The canvas marks the path in text for a screen reader.

## Phase 10 — runs, retry, and version comparison

Run history. `GET /flows/{id}/runs?status=&from=&to=&cursor=`: one row per run with status, start, duration, trigger in words, the person who caused it, and the number of writes. `GET /flows/runs/{instance_id}` returns the timeline from `process_events`: each step with start, end, outcome, and the effects in words with the records written as links the viewer may open, and the first error. No field values appear in the timeline.

Retry. `POST /flows/runs/{instance_id}/retry`, flow owner or admin, with a reason. It resumes a `Failed` run from the failed step with the same run id, so the dedupe keys prevent double effects. A run that failed on a grant error cannot be retried until a new version is approved. Ledger entry on retry.

Version comparison. The version list shows each version with its author, approvers, and proposal. Choosing two shows them on the canvas with added, changed, and removed nodes outlined and marked in text, and the sentence changes beside it.

Screen. A `Runs` tab and a `Versions` tab in the editor, built from the kit, with the failure rate over the last 7 days in words and the circuit-breaker state.

Tests.

1. A failed run's timeline shows the failed step and its error. A retry resumes there and writes each effect once, proved by the dedupe table.
2. A run that failed on `flow_grant_exceeded` cannot be retried until a new version is live.
3. The timeline holds no record values. Check the fixture.
4. A comparison of two versions marks the changed step as changed in text.
5. A retry by an editor who is not an owner or admin is 403.

## Phase 11 — the console panel

A `Flows` section, from the kit, in `ADMIN_ROUTES`: every flow across workspaces with home workspace, whether it crosses workspaces, the parties and when each approved, its grants in sentences, last run and result, failures in the last 7 days, and state (`Active`, `Disabled`, `Tripped`). Actions with a reason: `Disable` and `Enable` through `flows.disabled`. A filter `Crosses workspaces` and a filter `Reads sensitive data`. The Overview gains the number of flows, those that cross workspaces, tripped flows, and the work-item drift repaired by the last reconcile.

Tests.

1. The panel lists a cross-workspace flow with both parties and their approval times.
2. `Disable` writes a settings ledger entry and stops the next run.
3. The filter `Reads sensitive data` lists only flows with a sensitive field in `read`.
4. A faculty caller is 403 on every route here.

## How to prompt Gemini

```
Read docs/plans/workflows.md and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency. The canvas is hand-written SVG.
A flow acts only within its approved grants. Never run an effect outside them.
A flow never deletes and never scans. It addresses records.
Every list a person sees is computed by the server.
Never put a record value in a work item title, a summary, a notice, or a run timeline.
Do not read, copy, or adapt another product's source code.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| A flow across workspaces will move data where it should not go. | It runs only inside grants a reviewer reads as sentences, every touched workspace's owner approves, moving a sensitive field is flagged, and moving it to a lower classification fails. |
| Automation will act with more rights than the person who triggered it. | It acts with its own approved grants and nothing else, so triggering cannot widen what it does. |
| A flow will run away. | It cannot loop, cannot delete, cannot scan, is capped in steps, writes, fan-out, rate, and chain depth, and trips a breaker after 20 failures. |
| A task assignee will see more than they should. | A task grant reaches one record and the listed fields only while the step is open, and closes in the same transaction as the step. |
| The list will show people things they cannot act on. | It is computed by the server and re-checked live when they act. A drift job repairs differences and reports them. |
| A diagram editor will be inaccessible. | The outline is a complete editor that shares one command model with the canvas, and a test performs a full edit by keyboard. |
| Retrying a failed run will duplicate work. | Every effect has a dedupe key written in the same transaction, so a retry cannot write twice. |
| Cross-workspace flows are risky for us. | A setting turns them off, and a second setting stops any flow at once. |
