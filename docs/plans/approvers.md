# Positions and approvers — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `organization.md` phases 1, 2, and 5 and `admin-console.md` phase 1. Phase 3 needs `business-process.md` phases 1 to 6 verified.

An admission goes to "the chair of the applicant's department". A grant goes to "the dean". A leave request goes to a person's supervisor. None of these is a role name typed into a text box. They are positions in the org tree, held by people, that change when people are appointed and when they go on leave.

Today a waiting step carries a `role` string. The server compares it with the caller's workspace role or affiliation. Nobody can say who a step is waiting for, a vacant position stops the process silently, and the person who submitted a record can approve it.

## Standard

The W3C Organization Ontology models this exactly: a `Post` exists in an organization, a person holds it through a `Membership`, and a `Role` names what the post is. This brief uses its meaning and keeps the platform's own words, **position** and **holder**. Record the search in the first commit message: W3C Organization Ontology (`org:Post`, `org:Role`, `org:Membership`, `org:heldBy`). CEDS and eduPerson define no position model.

Separation of duties is NIST SP 800-53 AC-5.

## Decisions already made

| Question | Answer |
| --- | --- |
| Where does a position live? | A `roles` row with a `position_key`. A person holds a position at one unit. It is the existing appointment table. No new holder table |
| Who defines position types? | A Platform Admin. A type has a key, a name, the unit types it applies to, and how many holders it may have |
| Is a position inferred from a job title? | No. Never. A title such as "Chair" in SCIM grants nothing. A position comes from an explicit appointment, or from an explicit SCIM role entry of type `scaffoldry-position` |
| Can the submitter approve their own record? | No. The person who created the record, and the person whose change started the process, are never approvers. This is not a setting |
| What if a position is vacant? | Optionally the next unit up is asked. If no one is found, the step stays `Waiting`, is flagged `no_approver`, and appears in the console as Unassigned. It does not fail and it does not skip |
| Is delegation real? | Yes. A holder delegates to another person for a date range. It is one hop. A delegate cannot delegate again |

## What exists today

| Fact | Where |
| --- | --- |
| The `roles` table has `person_id`, `organization_id`, `role_title`, `scoped_affiliation`, `is_primary`, `source` | `0001_initial_schema.sql`, `0006_role_source.sql` |
| `UserTask.role` is a string matched against the caller's workspace role or affiliation | `workflow.rs`, `routes/apps.rs` `decide_app_process` |
| A record stores no author. `dataset_records` has no `created_by` | `0002_workspaces_and_ledger.sql` |
| A `ProcessInstance` stores no starter | `workflow.rs` |
| `title` from SCIM is copied into `role_title` and nothing else | `routes/scim.rs` `sync_org_roles` |

## Out of scope

- Org-chart import from an HR system beyond the existing SCIM.
- Supervisor relationships between people ("my manager"). Positions are held at units.
- Delegation chains, or delegation of a single instance.
- Escalation timers and reminders. The console shows what is stuck.
- Approval by a group or a quorum ("two of five committee members"). A step has one decision.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. Resolution is a pure function over the org list, role rows, users, delegations, and a time. It does no I/O. Everything else calls it.
4. Every change to a position type, a holding, or a delegation is a ledger entry written before the change.
5. Use the kit in `apps/web/src/ui/` for every screen.

## Phase 1 — the model and the resolver

Migration `crates/scaffoldry-core/migrations/0021_positions.sql`:

```sql
CREATE TABLE IF NOT EXISTS position_types (
    key VARCHAR(64) PRIMARY KEY,
    name VARCHAR(128) NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    org_types JSONB NOT NULL,
    max_holders INTEGER NOT NULL DEFAULT 1,
    retired_at TIMESTAMPTZ,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
ALTER TABLE roles ADD COLUMN IF NOT EXISTS position_key VARCHAR(64) REFERENCES position_types(key);
CREATE INDEX IF NOT EXISTS idx_roles_position ON roles (position_key, organization_id) WHERE position_key IS NOT NULL;
```

Migration `0022_record_created_by.sql`:

```sql
ALTER TABLE dataset_records ADD COLUMN IF NOT EXISTS created_by VARCHAR(255);
```

`create_record` stores the caller's eppn in `created_by`, through every path (REST, table REST, MCP). Add `created_by: Option<String>` to `DatasetRecord`. `ProcessInstance` gains, all with `#[serde(default)]`: `started_by: Option<String>`, set to the principal whose write triggered it; `started_at: String` (RFC 3339), set when it is created; `assigned_to: Option<String>`, which only `admin-console.md` phase 6 sets; and `no_approver: Option<String>`, set in phase 3.

`position_key` and `org_types` are keys and unit types. `org_types` is a non-empty subset of `Institution`, `College`, `Department`, `Center`, `Program`. `max_holders` is 1 to 50.

Types, in `scaffoldry-core/src/workflow.rs` or a new `approver.rs`, with a schema `governance/schema/approver-spec.schema.json`:

```rust
pub enum ApproverSpec {
    Position { key: String, from: Origin, walk_up: bool },
    Person { eppn: String },
    Role { name: String },
}
pub enum Origin { Submitter, Workspace }
```

`UserTask` gains `#[serde(default)] approver: Option<ApproverSpec>`. When it is `None`, the step uses `Role { name: role }`. Old rules keep working. Wire shape is the default external tag, like every other enum here.

The resolver:

```rust
pub struct Resolution {
    pub approvers: Vec<Approver>,        // eppn, display name, via
    pub unit_id: Option<Uuid>,           // the unit a position was found at
    pub problem: Option<NoApprover>,     // why the list is empty
}
pub enum Via { Holder, DelegateOf(String), Named, Role }

pub fn resolve_approvers(spec: &ApproverSpec, ctx: &ResolveCtx) -> Resolution
```

`ResolveCtx` holds: the org list, role rows, users (active and hold state), delegations, the current time, the submitter, the starter, the workspace's unit, and, for `Role`, the workspace's collaborators.

Rules, in order.

1. `Position`. The start unit is the submitter's primary unit when `from` is `Submitter` and the submitter is known and has a primary role. Otherwise it is the workspace's unit.
2. Holders at that unit: role rows with that `position_key`, whose person is active and not on hold.
3. If there are none and `walk_up` is true, move to the parent and repeat. Stop at the root. Cap at 32 steps. A cycle returns no result and does not loop.
4. Remove the submitter and the starter from the result. If that empties the list and `walk_up` is true, continue upward. Otherwise the list is empty.
5. Add active delegates of each remaining holder whose delegation covers the current time, is not revoked, and whose delegate is not the submitter or the starter. One hop only.
6. `Person` resolves to that user when active and not on hold, then the same removal and delegation rules.
7. `Role` returns the workspace collaborators whose role or affiliation equals the name, minus the submitter and the starter.
8. An empty list sets `problem` to one of: `position_vacant`, `only_submitter`, `person_unavailable`, `role_empty`.

Add `validate_approver(spec, state) -> Result<(), String>`: a position key must exist and not be retired, a person must exist, a role name must be non-empty.

Tests (unit tests of the pure function in `scaffoldry-core`; no database).

1. A chair at the submitter's department is returned with `via: Holder` and that unit.
2. A vacant department with `walk_up` true returns the college's dean and the college unit. With `walk_up` false it returns nothing and `position_vacant`.
3. The only holder is the submitter. With `walk_up` true it moves up. With it false the result is empty with `only_submitter`.
4. A holder who is on hold, or inactive, is skipped.
5. A delegate inside the date range is added with `DelegateOf`. Outside it, revoked, or equal to the submitter, it is not. A delegate's own delegate is not followed.
6. The starter is excluded even when they hold the position.
7. A parent cycle returns empty and does not loop.
8. `Role` returns the same people the old role match did, minus the submitter.
9. An old rule JSON with only `role` deserializes and resolves as `Role`.
10. `created_by` is stored on create through REST and MCP.

## Phase 2 — positions in the console

Needs phase 1 and `admin-console.md` phases 1 and 2.

Routes. Platform Admin unless noted. Every route is in `ADMIN_ROUTES`.

| Method | Path | Behavior |
| --- | --- | --- |
| GET | `/admin/positions` | Position types with holder counts |
| POST | `/admin/positions` | Body `key`, `name`, `description`, `org_types`, `max_holders`, `reason`. Ledger `PositionChanged` |
| PATCH | `/admin/positions/{key}` | Name, description, `max_holders`, or `retired: true`. Not `key` and not `org_types` while holders exist. Retiring does not remove holders. Ledger `PositionChanged` |
| GET | `/admin/positions/vacancies` | One row for each unit whose type the position applies to and that has no holder |
| GET | `/orgs/{id}/positions` | Position types that apply to that unit and their current holders. Anyone in scope |
| POST | `/orgs/{id}/positions/{key}/holders` | Body `eppn`, `replace` (bool), `reason`. A Platform Admin, or an Org Unit Admin in scope. The unit's type must be in the position's `org_types`. The holder must resolve and be active. When the position is full, 409 `position_full` unless `replace` is true, which vacates the current holders in the same transaction. Ledger `AccessRoleGranted` |
| DELETE | `/orgs/{id}/positions/{key}/holders/{eppn}` | Body `reason`. Refused with 409 when the row's `source` is `scim`. Ledger `AccessRoleRevoked` |

Add `PositionChanged` and `DelegationChanged` to `DecisionType::ALL`. Use the one-definition mechanism from `admin-console.md` phase 1.

SCIM. In `sync_org_roles`, a `roles[]` entry `{ "value": "{key}", "type": "scaffoldry-position" }` inserts a holding at the user's deepest matched unit with `source = scim`, only if that key exists and applies to the unit's type. An unknown key inserts nothing and the write still returns 201. A job title never creates a holding. Re-sync deletes only the user's `scim` holdings.

Frontend, from the kit.

- A `Positions` section in the console navigation: a table of position types with holders, `New position type`, and a `Vacancies` tab listing each vacant unit and position with an `Assign` action.
- In the Organization panel's unit drawer, a `Positions` list: each applicable position, its holder or `Vacant`, and `Assign`, `Replace`, `Vacate`, each through `ConfirmAction` with a reason.
- In the People drawer, the person's positions.
- `PersonLabel` and `UnitLabel` everywhere. Never show a bare id.

Tests.

1. Create a position type, assign a holder at a department, and read it back from `/orgs/{id}/positions`.
2. A position that applies to `College` cannot be held at a `Department`: 400.
3. Assigning to a full position is 409 `position_full`. With `replace` the old holder is gone and the new one is in, in one ledger entry pair.
4. A `scim` holding cannot be vacated: 409. Re-syncing the user removes and re-adds it.
5. A SCIM user with title `Department Chair` and no `scaffoldry-position` role holds nothing.
6. `vacancies` lists exactly the units with no holder.
7. An Org Unit Admin in scope can assign inside their unit and is 403 outside it.
8. Web: `Vacate` without a reason is not sent. The unit drawer shows `Vacant` for an empty position.

## Phase 3 — the engine asks the resolver

Needs `business-process.md` phases 1 to 6 verified, and `admin-console.md` phase 6 not yet started.

1. `save_rule` validation calls `validate_approver` for every `UserTask`. A bad key, a missing person, or an empty role name is 400 with the step id.
2. Where a process instance is created, store `started_by`. Where the record's `created_by` is read for resolution, load it from the record.
3. One pure function replaces the role comparison everywhere: `can_decide(user, instance, step, ctx) -> bool`. A user may decide when they appear in `resolve_approvers` for the step. When the instance has an `assigned_to` from `admin-console.md` phase 6, only that person may decide, and they must still not be the submitter or starter. The Cedar `approve` check through `decide` still applies after this.
4. A step with no approver does not fail. The instance stays `Waiting`. Add `no_approver: Option<String>` (the `problem`) to the instance JSON, set when the instance is created and refreshed when it is listed. Add a log line when it is first set.
5. `GET /apps/{slug}/processes` gains `mine=true`, which returns only the instances the caller can decide now, computed with `can_decide`. Add `approvers` (names and `via`) to each instance returned to a user who can read the app.
6. The web `ProcessDesk` calls `mine=true`. It no longer compares roles in the browser.
7. A rule saved with a `Position` spec whose position currently resolves to nobody still saves. The check is a warning in `mcp-apps.md` phase 4a.

Tests.

1. A rule with `Position { chair, from: Submitter, walk_up: true }`. A record created by a faculty member in Physics. The Physics chair can decide. The Chemistry chair gets 403. The submitter gets 403.
2. The Physics chair is removed. The instance is still `Waiting` and now lists the dean as approver. With `walk_up` false it shows `position_vacant`.
3. The submitter is the chair. They cannot decide their own record. The dean can.
4. The chair changes while the instance waits. The old chair gets 403 and the new one can decide.
5. A `Person` approver who goes on hold cannot decide, and the instance shows `person_unavailable`.
6. An old `Role`-only rule behaves as before. `business-process.md` tests still pass.
7. `mine=true` returns the same set as running `can_decide` over all waiting instances.
8. Web: the desk shows only what `mine=true` returned.

## Phase 4 — delegation

Migration `0025_delegations.sql`:

```sql
CREATE TABLE IF NOT EXISTS delegations (
    id UUID PRIMARY KEY,
    delegator_eppn VARCHAR(255) NOT NULL,
    delegate_eppn VARCHAR(255) NOT NULL,
    position_key VARCHAR(64) REFERENCES position_types(key),
    starts_at TIMESTAMPTZ NOT NULL,
    ends_at TIMESTAMPTZ NOT NULL,
    reason TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_delegations_delegator ON delegations (delegator_eppn);
CREATE INDEX IF NOT EXISTS idx_delegations_delegate ON delegations (delegate_eppn);
```

`position_key` null means every position the delegator holds.

| Method | Path | Behavior |
| --- | --- | --- |
| GET | `/me/delegations` | Delegations the caller made and received |
| POST | `/me/delegations` | Body `delegate`, optional `position_key`, `starts_at`, `ends_at`, `reason`. The caller must hold the position, or any position when it is null. The delegate must resolve and be active. At most 365 days. A delegate cannot be the caller. Ledger `DelegationChanged` |
| DELETE | `/me/delegations/{id}` | Revokes. The delegator, or a Platform Admin |
| GET | `/admin/delegations` | Platform Admin. Query `delegator`, `delegate`, `state`, cursor. Every delegation |
| DELETE | `/admin/delegations/{id}` | Platform Admin. Body `reason`. Ledger `DelegationChanged` |

A delegation ends on `ends_at` without anyone acting. Resolution already reads the time.

Frontend. In the user settings pane from `foundation.md` phase 9, a `Delegation` section. It shows the positions the person holds. `Delegate while away` takes a person (searched by name, shown with address), dates, and a reason. Active and past delegations are listed with `Revoke`. A person who receives a delegation sees it in the same pane and on the desk: `You are deciding for Dr. Rivera until 14 Oct.` The admin People drawer lists a person's delegations.

Tests.

1. A delegate inside the window appears in a step's approvers with `via: DelegateOf(chair)` and can decide.
2. The day after `ends_at`, they cannot.
3. Revoking removes them at once.
4. A person who holds no position cannot create a position delegation: 400.
5. A delegation of 400 days is 400. Delegating to oneself is 400.
6. A delegate who is the record's submitter is not an approver for that record.
7. Every create and revoke writes a ledger entry.
8. Web: the form requires dates and a reason. A received delegation is shown on the desk.

## Phase 5 — the builder says who

Web, and one preview route. Touch `apps/web/src/WorkflowBuilder.tsx` and `types.ts`.

Route: `POST /apps/{slug}/automations/preview-approvers`. App `Manage` scope. Body `approver` (an `ApproverSpec`) and an optional `created_by` eppn, which defaults to the caller. Returns the `Resolution` with names, the unit it came from, `via`, and the `problem` if any. It never creates anything.

In the user-task editor, replace the free-text `role` box with a `Who decides` control:

- `A position`: a list of position types, then `Starting from: the person who submitted the record` or `the workspace's unit`, and a checkbox `If the position is vacant, ask the next unit up`, checked by default.
- `A specific person`: searched by name.
- `A workspace role`: the old behavior, labelled as such.
- A live preview from the route: `Today this would go to: Dr. Rivera, Chair of Physics.` or the problem in words, `This position is vacant at Physics and above.` as a `Banner` of tone `warning`.

Keep the single-action form working. Do not add a graph here. The canvas is `workflows.md` phase 8.

Tests (vitest, `fetch` stubbed).

1. Choosing a position and a starting point posts the preview request and shows the returned names.
2. A `position_vacant` response shows the warning in words.
3. The saved rule JSON holds `approver: { Position: { key, from, walk_up } }` and no `role` text typed by the user.
4. The role option still saves `approver: { Role: { name } }`.

## How to prompt Gemini

```
Read docs/plans/approvers.md, docs/plans/ux-standards.md, and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
Do not infer a position from a job title.
Do not let the submitter or the starter decide.
Do not write I/O in resolve_approvers.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| A job title in the directory will quietly make someone an approver. | Titles grant nothing. A position comes from a Platform Admin or Org Unit Admin appointment, or from an explicit `scaffoldry-position` entry in the registry feed. |
| People approve their own requests. | The submitter and the person who started the process are removed from every approver list. Test 3 in phase 3 proves it, and there is no setting to turn it off. |
| A vacant chair will stall a student's application. | The step can ask the next unit up. If no one is found it stays waiting and shows in the console as Unassigned with a reason, so an administrator can assign a person. It is never skipped. |
| A chair on leave will block everything. | They delegate by date. It ends by itself. The delegate sees what they are deciding for whom. |
| A delegate will approve their own record. | The same removal rule applies to delegates. |
| Changing a chair mid-process will leave decisions with the old one. | Approvers are resolved when listed and when decided, from the current appointments. The old chair loses the step as soon as they are vacated. |
| Position data will drift from the registry. | Registry holdings are marked `scim` and cannot be edited by hand. Hand-made holdings are marked `api`. A re-sync removes only the registry's own. |
| The org tree has no standard for this. | The W3C Organization Ontology does. The names differ and the meaning is the same. |
