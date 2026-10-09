# Business process, second stage — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `approvers.md` phase 3 and `mcp-apps.md` phase 4a. Phases 3 and 4 need `notifications.md` phase 4 and `jobs.md` phase 2.

The first stage of the process engine runs a straight line of steps, with one place that waits for one person. Real institutional processes are not like that. A grant over a threshold needs a dean. A committee votes. A request that sits for two weeks goes to the next level up. A reviewer must be told. A manager wants to know where requests are stuck.

This brief adds timers and escalation, branches, committee votes, manual and scheduled triggers, effects that reach other tables, and the measurements that show where time goes.

## Standard

BPMN 2.0 names remain the vocabulary: a timer event, an exclusive gateway, and a multi-instance task. Scaffoldry does not implement BPMN, and the documents do not claim conformance.

## Decisions already made

| Question | Answer |
| --- | --- |
| Can a timer approve something? | Never. A timer can remind, escalate, or fail a step. It cannot decide |
| How is a committee decided? | By a rule on the step: all must approve, any one, or at least N. Reject policy is stated too. Voters are distinct and exclude the submitter and starter |
| Where do timers run? | In the scheduler from `jobs.md`, accurate to about a minute |
| Can a rule write to another table? | Yes, inside the same app, as the `run_automation` action, with every write in history as an automation batch |
| What does branching look like? | An exclusive choice: the first branch whose conditions hold runs, otherwise the default. Nesting is limited to two levels |

## What exists today

| Fact | Where |
| --- | --- |
| Steps run in order. A `UserTask` stops the run and waits. A step has `when` conditions | `workflow.rs` |
| `TriggerEvent` covers record created, updated, and status changed | `workflow.rs` |
| Effects: set fields, notify, ledger note, rejected | `ActionEffect` |
| Approvers are resolved from positions with delegation | `approvers.md` |
| A rule is a versioned definition changed by proposal | `mcp-apps.md` phase 4a |

## Out of scope

- Parallel branches that run at once and join.
- Loops and sub-processes.
- Message events from outside.
- Compensation.
- A graphical diagram editor.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. A timer never decides. A test enumerates every timer action and asserts none sets approved or rejected.
4. Every state change of an instance is an event in the instance's own log, so the history can be measured.
5. Use the kit for every screen.

## Phase 1 — measuring the process

Migration `0040_process_v2.sql`:

```sql
CREATE TABLE IF NOT EXISTS process_events (
    id BIGSERIAL PRIMARY KEY,
    instance_id VARCHAR(128) NOT NULL,
    app_slug VARCHAR(64) NOT NULL,
    rule_id VARCHAR(64) NOT NULL,
    rule_version INTEGER NOT NULL,
    step_id VARCHAR(64),
    kind VARCHAR(20) NOT NULL,
    actor VARCHAR(255),
    detail JSONB NOT NULL DEFAULT '{}',
    at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_process_events_instance ON process_events (instance_id, id);
CREATE INDEX IF NOT EXISTS idx_process_events_rule ON process_events (rule_id, kind, at);
```

`kind` is `started`, `step_entered`, `step_left`, `waiting`, `decided`, `reassigned`, `escalated`, `reminded`, `voted`, `cancelled`, `completed`, `failed`. Write one event for each transition, in the same transaction as the change. An event holds ids and names of steps, never record values. `detail` holds the decision and the approver count, for example.

Analytics, from this table. `GET /apps/{slug}/process-analytics?rule=&from=&to=` (app owner or admin, and Platform Admin) returns, per rule and step: the number of instances that entered it, the median and 90th percentile time in the step, the share that ended `Completed`, `Rejected`, `Failed`, and `Cancelled`, and the number currently waiting with an age histogram in buckets of 1, 3, 7, 14, 30 days. A person's own queue stays in the desk.

Screen. A `Process insights` tab in the app and a panel in the console: a table by step with the figures in words and numbers, a bar of how many are waiting by age, and a note on the slowest step (`Dean review takes a median of 6 days.`). The charts are from `views.md` phase 4 once they exist, and tables until then.

Tests.

1. A three-step process run end to end writes `started`, `step_entered`, `step_left`, `decided`, and `completed` events in order.
2. An event never contains a record value. A test checks every `detail` produced by the fixture.
3. The analytics for 50 fixture instances with known timings give the expected median and the 90th percentile.
4. The waiting histogram places instances in the right buckets.
5. A person who is not an owner or admin of the app is 403 on the analytics.

## Phase 2 — branches, cross-table effects, and triggers

Branches. `StepKind::Branch { branches: Vec<{ when: Vec<FieldPredicate>, steps: Vec<ProcessStep> }>, otherwise: Vec<ProcessStep> }`. The first branch whose conditions hold runs its steps. Otherwise `otherwise` runs. Nesting is at most two levels, and the validator counts steps (at most 50 per rule, at most 20 per branch). Branch decisions are events.

Effects across tables. `ActionType::CreateRecord { table_id, fields: Vec<(String, Expr)> }` and `ActionType::UpdateLinked { link_field, fields: Vec<(String, Expr)> }`. `Expr` is a formula from `calc-graph.md`, evaluated with the triggering record. Authorization is the Cedar action `run_automation` on the target table, through `decide`, with the triggering person as principal, and each write goes through `write_record` with `actor_kind` of `automation` and a batch shared by the run. The loop guard and depth cap from stage one apply, and a rule cannot create a record that triggers itself on the same table.

Triggers. `TriggerEvent::Manual` for the button field (`field-types.md` phase 5), and `TriggerEvent::DateField { field, offset_days, at }`, for example thirty days before a deadline. A scheduled job, `scan_date_triggers`, runs hourly. For each rule with a date trigger it finds records whose date plus offset falls in the window since the last scan, using the index on the date field, and starts an instance for each with `dedupe_key` of rule, record, and the date, so a record is started once per date even if the job runs twice or is resumed. The validator requires the date field to be indexed and adds it.

Validation additions: a branch's conditions use fields that exist, a `CreateRecord` names a table of the same app and every required field there is supplied, and a `Manual` trigger has a button.

Tests.

1. A grant over the threshold takes the dean branch. One under it takes the default. The events show which.
2. A nested branch three levels deep fails validation.
3. `CreateRecord` writes a record in another table as the triggering person under `run_automation`, in history as an automation batch. A person without that action starts the rule and the write is refused, and the instance fails with the reason.
4. A rule that would create a record triggering itself on the same table fails validation.
5. A date trigger fires once for a record even if the job runs twice in the window, and not again the next scan.
6. A `Manual` trigger with no button fails validation.
7. A branch's conditions on a missing field fail validation with the field named.

## Phase 3 — timers and escalation

A `UserTask` gains `due: Option<DurationSpec>` and `on_due: Vec<DueAction>`, where `DueAction` is one of:

| Action | Effect |
| --- | --- |
| `Remind` | Notify the current approvers again with `process_overdue` |
| `Escalate { approver: ApproverSpec }` | Add the resolved approvers to who may decide, notify them with `process_escalated`, and tell the old approvers and the record's owner |
| `EscalateUp` | Resolve the same position one unit higher, as `walk_up` does |
| `Fail { reason }` | End the instance as `Failed` with the reason, applying the step's `on_fail` actions |

There is no action that approves or rejects. `due` is counted from entering the step, in whole days, in `platform.timezone`. `remind_every` repeats `Remind` at most 5 times.

Timers. Migration `0053_process_timers.sql`:

```sql
CREATE TABLE IF NOT EXISTS process_timers (
    id UUID PRIMARY KEY,
    instance_id VARCHAR(128) NOT NULL,
    step_id VARCHAR(64) NOT NULL,
    action JSONB NOT NULL,
    fire_at TIMESTAMPTZ NOT NULL,
    state VARCHAR(8) NOT NULL DEFAULT 'pending',
    fired_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_process_timers_due ON process_timers (fire_at) WHERE state = 'pending';
```

Entering a step inserts its timers in the same transaction. A scheduled job `fire_timers`, every minute, claims due timers with `FOR UPDATE SKIP LOCKED`, applies the action, and marks them fired. A decision, a cancel, or a reassign cancels the instance's pending timers in the same transaction, so a decided step never escalates. A fired `Escalate` adds a second timer if the step defines one.

Stale in the console now means a step with a fired or due timer. The Instances panel from `admin-console.md` phase 6 shows the next timer and what it will do, in words: `On 14 Oct, this will be sent to the Dean of Sciences.`

Tests.

1. A step due in 3 days with `EscalateUp` adds the dean at day 3, notifies old and new approvers and the owner, and the dean can decide.
2. A decision before the timer fires cancels it. Nothing is escalated afterwards.
3. `Fail` ends the instance with the reason and runs its `on_fail` actions.
4. `Remind` with `remind_every` of 1 day sends at most 5 reminders.
5. A timer is applied once even if two job threads run `fire_timers`.
6. No `DueAction` variant approves or rejects. A test enumerates the enum.
7. The console shows the next timer in words.
8. Changing the platform time zone changes when new timers fire and not ones already set.

## Phase 4 — committees

`UserTask.decision: Single | AllOf | AtLeast(n)` and `reject: FirstReject | MajorityReject`. The approver specification resolves to a set. A position with `max_holders` above one is a committee. Votes are stored by migration `0054_process_votes.sql`:

```sql
CREATE TABLE IF NOT EXISTS process_votes (
    instance_id VARCHAR(128) NOT NULL,
    step_id VARCHAR(64) NOT NULL,
    voter VARCHAR(255) NOT NULL,
    vote VARCHAR(8) NOT NULL,
    comment TEXT,
    at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (instance_id, step_id, voter)
);
```

A vote is `approve` or `reject`. A person votes once and may change their vote until the step closes. The step closes when the rule says it can: `AllOf` when everyone approved or `FirstReject` applies, `AtLeast(n)` when `n` approved or too many rejected for `n` to remain possible, `MajorityReject` when more than half have rejected. The voters set is fixed when the step is entered, from the resolution then, plus delegates. Someone appointed later is not added mid-vote. A step needs at least `n` resolvable voters, or the instance shows `no_approver` as before. A comment is required on `reject` when the setting `process.require_reject_comment` is true (default true, in the closed list).

The decision is recorded as one ledger entry, `WorkflowRuleApproved`, with the tally and each voter's address, vote, and time, so the audit shows who decided. Individual votes are visible to the committee and to app owners and admins, not to the submitter.

Tests.

1. `AllOf` with three voters stays waiting after two approvals and completes on the third. One rejection under `FirstReject` closes it rejected.
2. `AtLeast(2)` of 3 closes approved on the second approval and closes rejected once two reject.
3. A voter cannot vote twice, can change a vote, and the submitter and starter are not voters.
4. A person added to the position after the step started is not a voter.
5. A reject without a comment is refused when required.
6. One ledger entry holds the tally and every voter.
7. The submitter cannot see individual votes.

## Phase 5 — the screens

From the kit.

- Builder. In the user-task editor: `Who decides` from `approvers.md` phase 5, then `How many must agree` (one, all, or at least N), `Due after` days with `If it is late` choices (remind, ask the next level, fail), and a preview sentence for the whole step: `Asked of the Chair of Physics. If no one decides in 7 days, it goes to the Dean of Sciences.` Branch steps show as a list of conditions with their steps nested, and the whole rule reads back as the sentences from `describe_rule`.
- Desk. A waiting decision shows its due date, and in a committee step the tally in words (`2 of 3 have approved`) without individual names for the submitter, and the person's own vote with `Change my vote`.
- Insights, from phase 1.

Tests (vitest, `fetch` stubbed).

1. The step preview sentence matches the chosen options.
2. A committee step shows the tally and the person's own vote, and `Change my vote` posts the new vote.
3. A branch shows its nested steps and the saved rule's JSON has the branch shape.
4. The due date is shown as a date and in words.

## How to prompt Gemini

```
Read docs/plans/process-v2.md, docs/plans/approvers.md, and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
A timer never approves or rejects.
Every state change writes a process event in the same transaction.
Do not put a record value in an event.
Do not read, copy, or adapt another product's source code.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| A timer will approve something nobody read. | No timer action approves or rejects. A test enumerates them. |
| An escalation will go to the wrong person. | It resolves through the same positions and delegation as any step, tells the old and new approvers and the owner, and the console shows what will happen and when. |
| A committee vote will be unprovable. | One ledger entry holds the tally and every voter with their vote and time. |
| Rules that create records will run away. | The loop guard and depth cap remain, a rule cannot trigger itself on its table, and each write is authorized and in history. |
| We cannot see where requests stall. | Phase 1 measures time by step and the age of what is waiting, for owners and administrators. |
| A date trigger will fire twice. | A dedupe key of rule, record, and date makes it once, even if the job runs twice. |
