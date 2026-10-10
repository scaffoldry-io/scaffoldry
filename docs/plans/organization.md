# Organization scope — implementation brief

> **Status: COMPLETED (Phases 1–5 Finished and Verified)**  
> **Test Coverage:** `organization_scope_test.rs`, `api_integration_test.rs` (`test_phase2_organization_api_and_scoping`, `test_phase5_scim_academic_and_position_sync`), `organization-admin.test.tsx`, `organization-rail.test.tsx`.


Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. This plan is the container for the data grid and the business process. An app, a grid view, and a workflow live in a workspace. A workspace lives in one organization unit. A unit lives in a tree.

## Names

Use these labels in the UI. Do not write Super Admin, System Admin, or Org Admin.

| Label | Stored value | Do not rename the stored value |
| --- | --- | --- |
| Platform Admin | affiliation `central_admin` | Tests and `/admin` already require this string |
| Org Unit Admin | `roles.scoped_affiliation = "unit_admin"` | One appointment row per unit |

`unit_in_scope` and the API keep those stored strings. Buttons, headings, and empty states say Platform Admin and Org Unit Admin.

## Decision

Units form a tree. People form a matrix.

| Question | Answer |
| --- | --- |
| Can a department have two parents? | No. `organizations.parent_id` is one column. A center that spans colleges is its own unit under the institution, not a second parent. |
| Can one person administer two units? | Yes. The `roles` table is already `(person_id, organization_id)` and allows many rows. One row has `is_primary = true`. |
| What is the Airtable "org"? | The root row, `org_type = Institution`. There is one root. |
| What is an Airtable workspace? | A row in `workspaces`, with a required `organization_id`. |
| What is an Airtable base? | An app inside that workspace. Do not move apps onto the org row. |

Do not build a matrix of organization nodes. A unit with several parents makes "who is the admin above this workspace" unanswerable, and Cedar cannot evaluate it in one walk.

## Standard

NCES CEDS: an education organization has one parent organization. The column is already `organizations.parent_id` in `crates/scaffoldry-core/migrations/0001_initial_schema.sql`.

REFEDS eduPerson: affiliation is scoped to an organization. The appointment row is `roles.scoped_affiliation`.

Closed `org_type` values: `Institution`, `College`, `Department`, `Center`, `Program`. Reject any other string. `Institution` is legal only when `parent_id` is null. Only one row may have a null parent.

## What already exists

| Fact | Where |
| --- | --- |
| Org tree table | `organizations(id, parent_id, name, code, org_type)` in `0001_initial_schema.sql` |
| Appointments | `roles(person_id, organization_id, scoped_affiliation, is_primary)`. No Rust code reads this table |
| Workspaces are not in that tree | `workspaces.organization` is a free string. `0002_workspaces_and_ledger.sql` |
| Apps in SQL point at an org, not a workspace | `apps.organization_id`. The running app model uses `workspace_id` on the manifest. Leave `apps` alone |
| Super admin screen | `/admin`, `adminTab === "org"` in `apps/web/src/AdminConsoleView.tsx`. It lists DNS names. It is not an org tree |
| Who may open `/admin` | `activePersona.affiliation === "central_admin"`. Keep that gate. `data-testid="admin-access-denied"` must stay |
| Workspace list already calls Cedar | `list_workspaces` in `crates/scaffoldry-server/src/service/workspaces.rs` |

## Scopes

Three scopes. Do not add a fourth.

| Scope | How you hold it | What you see |
| --- | --- | --- |
| Platform Admin | `affiliation == "central_admin"` | Every unit, every workspace, the full `/admin` console |
| Org Unit Admin | A `roles` row with `scoped_affiliation = "unit_admin"` on that unit | That unit and its descendants. Not siblings. Not `/admin` policy, ledger, infra, or impersonation |
| Workspace role | Existing collaborator role `owner`, `admin`, `editor`, `viewer` | That workspace only. Unchanged |

`unit_in_scope(caller, org_id)` is true when:

1. The caller is `central_admin`, or
2. Walking `parent_id` from `org_id` to the root, some node has a `unit_admin` role for `caller.eppn`.

An Org Unit Admin of a college sees that college, its departments, and their workspaces. An Org Unit Admin of one department does not see the college's other departments.

A Platform Admin is the only principal who creates, renames, reparents, or deletes a unit. An Org Unit Admin creates workspaces inside units in scope. An Org Unit Admin does not reparent a unit.

## Out of scope

- A second hierarchy (matrix parents, dotted lines, "also reports to").
- LDAP or registry sync. Appointments are rows you write. SCIM stays as it is.
- Moving the DNS manager, the policy tab, the ledger, or impersonation.
- Org settings inside `DataGrid` or `WorkflowBuilder`.
- A chart library for the tree. Nested buttons are the tree.
- Editing `deploy/migrations`. The server applies `crates/scaffoldry-core/migrations` from `repository.rs`.

## Phase 1 — attach workspaces to the tree

Migration `crates/scaffoldry-core/migrations/0004_organization_scope.sql`:

```sql
ALTER TABLE workspaces ADD COLUMN IF NOT EXISTS organization_id UUID REFERENCES organizations(id);
CREATE INDEX IF NOT EXISTS idx_workspaces_organization ON workspaces(organization_id);
```

Do not drop `workspaces.organization` or `workspaces.department`. They stay display strings.

On startup, after the migration, if zero `organizations` rows exist, insert one: code `INST`, name `Institution`, `org_type` `Institution`, `parent_id` null. Then set `workspaces.organization_id` to that root where it is null. Do this in `repository.rs` next to the other schema consts, inside the advisory lock, as `SCHEMA_0004`.

Add `organization_id: Option<Uuid>` on the workspace record struct with `#[serde(default)]`. New workspaces must send `organization_id`. Reject a create with 400 when it is missing or the id is unknown.

`unit_in_scope` lives in `crates/scaffoldry-server/src/service/organizations.rs` (new file). It takes the org list and the caller's role rows. Pure function. No Cedar string parsing.

Load `roles` the way workspaces are loaded: one `RwLock<Vec<RoleRow>>` on `SharedState`, filled from `SELECT id, person_id, organization_id, scoped_affiliation, is_primary FROM roles` at startup. Join `persons.eppn`. Store `eppn` on the role row so the scope check does not join later.

Tests in `crates/scaffoldry-server/tests/organization_scope_test.rs` (no database; call the pure function):

1. College admin is in scope for a child department.
2. Department admin is not in scope for a sibling department or for the college parent.
3. `central_admin` is in scope for every node.
4. A cycle in the fixture (`A.parent = B`, `B.parent = A`) returns false and does not loop. Cap the walk at 32 steps.

## Phase 2 — org API

Routes, session-authenticated like `/api/v1/workspaces`:

| Method | Path | Who | Behavior |
| --- | --- | --- | --- |
| GET | `/api/v1/orgs` | any signed-in user | Units where `unit_in_scope` is true, plus `parent_id`, `org_type`, `code`, `name`. Flat list. The client builds the tree |
| POST | `/api/v1/orgs` | Platform Admin | Body `name`, `code`, `org_type`, `parent_id`. 400 on a bad type, a second root, or an unknown parent. 403 otherwise |
| PATCH | `/api/v1/orgs/{id}` | Platform Admin | `name` and, optionally, `parent_id`. Reject a parent that is the node itself or its descendant. 403 otherwise |
| POST | `/api/v1/orgs/{id}/appointments` | Platform Admin, or Org Unit Admin of that id | Body `eppn`, `scoped_affiliation`. The only accepted value in this phase is `unit_admin`. Upsert the `roles` row. 403 outside scope |

`list_workspaces` drops a workspace whose `organization_id` fails `unit_in_scope`, even if Cedar would allow it. Membership in the workspace still applies after that filter. An Org Unit Admin sees workspaces in scope without a collaborator row. A faculty member who is only a collaborator still sees that workspace and no org tree above it.

Creating a workspace: `organization_id` must be in scope. 403 if not.

Tests in `api_integration_test.rs` or `organization_scope_test.rs`, whichever file already boots the router. Follow that file's `authed_req` helper.

1. Faculty collaborator on one workspace does not receive a sibling department's workspace.
2. `central_admin` POST creates a `Department` under the root and GET returns it.
3. A department Org Unit Admin POST of a sibling under the college returns 403.

Do not add a delete route.

## Phase 3 — Platform Admin organization screen

The rail button in `AdminDesk.tsx` that sets `adminTab` to `"org"` is labeled `Org & DNS Manager`. Rename the visible label to `Organization`. Add `data-testid="admin-org-tab-btn"`. Leave the other admin tabs as they are.

Replace the body of `adminTab === "org"` in `AdminConsoleView.tsx`. Keep the component's `central_admin` gate and `admin-access-denied`.

Layout, one column, no new dependency:

- Heading `Organization`.
- Tree: a nested list. Each unit is a button, `data-testid="org-node-{code}"`, showing `name` and `org_type`.
- Selected unit panel, `data-testid="org-detail"`: code, type, parent name, and the workspaces whose `organization_id` is that unit.
- Form `Create unit`, `data-testid="org-create-form"`: name, code, type select of the five types, parent fixed to the selected unit. Submit POSTs `/api/v1/orgs`. Institution type is absent from the select.
- Form `Appoint Org Unit Admin`, `data-testid="org-appoint-form"`: one eppn field. POST the appointments route with `scoped_affiliation` `unit_admin`. Show the eppn in a list under the form. The list heading is `Org Unit Admins`.
- Under the workspace list, keep the existing app domain rows that this tab already renders. Move them. Do not delete them.

Tests in `builder-usability.test.tsx` or a new `apps/web/src/test/organization-admin.test.tsx` if the admin console is not mounted by the usability file. Read the file before choosing. The test must:

1. As `central_admin`, open the Organization tab and see a root node.
2. As a faculty persona, still see `admin-access-denied`.
3. Submit the create form and assert `fetch` was called with `POST /api/v1/orgs`. Stub `fetch`.

Wire the screen to the API. If the running UI still uses the in-memory persona switch, keep that switch and load orgs through the existing `apiClient`. Do not add a second HTTP helper.

## Phase 4 — Org Unit Admin on the workspace rail

A caller who is not `central_admin` and who has at least one `unit_admin` appointment sees a block on the left rail of `AdminDesk.tsx`, above the workspace list. The block heading is `Org Unit Admin`.

- Label `Your units`. `data-testid="unit-admin-rail"`.
- One button per in-scope unit, `data-testid="unit-rail-{code}"`.
- Selecting a unit filters the workspace list to that unit and its descendants.
- A `New workspace` button is visible only then. It POSTs the existing workspace create with `organization_id` set to the selected unit. Hidden for callers with no `unit_admin` row.
- This block is not rendered on `/admin`. An Org Unit Admin does not gain policy, ledger, infra, or impersonation.

Test: a fixture Org Unit Admin sees the rail and does not see `admin-org-tab-btn`. A student collaborator sees neither the rail nor the admin tab.

## Phase 5 — members from academic and position data

SCIM is the source that places a person in an org. The appointment form in phase 3 is a manual grant. It does not replace the registry feed.

Standards already in the server:

- SCIM User, RFC 7643: `title`, `active`, `roles[]`.
- Enterprise User extension: `organization`, `division`, `department`. The create route already stores this JSON and does nothing with it.
- `roles[].type == "eduPersonScopedAffiliation"` with a value like `faculty@university.edu`. Parse it with `EduPersonAffiliation::from_str`.
- Do not read SCIM Groups. Group membership is not an org appointment.

On `POST /scim/v2/Users` and `PUT /scim/v2/Users/{id}`, after the SCIM user is saved, call one function `sync_org_roles(user, orgs) -> Vec<RoleRow>`. `DELETE /scim/v2/Users/{id}` deletes rows it had written for that user.

Migration `crates/scaffoldry-core/migrations/0006_role_source.sql`:

```sql
ALTER TABLE roles ADD COLUMN IF NOT EXISTS source VARCHAR(8) NOT NULL DEFAULT 'api';
```

Register it in `repository.rs` after `0005`, inside the same advisory lock. `source` is `scim` or `api`. A SCIM sync deletes only `source = 'scim'` rows for that person, then inserts. Rows the appointment API wrote (`source = 'api'`) stay.

Match each of `department`, `division`, and `organization` to a unit. Try `organizations.code` first, then `name` case-insensitive. Unknown values create no unit and no row. The SCIM write still returns 201.

For each matched unit, insert one row:

| Column | Value |
| --- | --- |
| `organization_id` | the matched unit |
| `role_title` | core User `title`. Empty string when `title` is absent. Persist `title` on `ScimUser` (`#[serde(default)]`) so PUT does not drop it |
| `scoped_affiliation` | the eduPerson token (`faculty`, `staff`, `student`, `employee`, `member`, `affiliate`, `alum`). `member` when the payload has none |
| `is_primary` | true only on the deepest match. Depth is `department`, then `division`, then `organization` |
| `source` | `scim` |

`active: false` deletes that user's `scim` rows and inserts none.

Admin is not inferred from the job title. "Chair" or "Dean" in `title` does not grant either admin role.

| SCIM `roles[]` entry | Effect |
| --- | --- |
| `{ "value": "unit_admin", "type": "scaffoldry" }` | Also insert `scoped_affiliation = "unit_admin"` on the deepest matched unit, `source = scim` |
| `{ "value": "platform_admin", "type": "scaffoldry" }` | Also insert `scoped_affiliation = "platform_admin"` on the Institution root, `source = scim` |
| absent on a later PUT | Delete the previous `scim` admin rows. Do not delete `api` rows |

`unit_in_scope` treats `platform_admin` on the root like `central_admin`: every unit is in scope. The `/admin` screen allows `central_admin` or this role. Do not write `central_admin` into the SCIM user.

Org membership does not add a workspace collaborator. A faculty appointment in Physics does not make that person an editor of every workspace in Physics.

API, same session auth as the other org routes:

| Method | Path | Who | Body |
| --- | --- | --- | --- |
| GET | `/api/v1/orgs/{id}/members` | in scope | `eppn`, `role_title`, `scoped_affiliation`, `is_primary`, `source` |
| POST | `/api/v1/orgs/{id}/appointments` | already specified | Writes `source = api` |

No SCIM resource named Org. Units stay on `/api/v1/orgs`. Do not add a PATCH route. `ServiceProviderConfig.patch.supported` is currently `true` and no PATCH handler exists. Set it to `false` in this phase.

Tests, added to the SCIM test file that already posts a user:

1. Enterprise `department` equal to an existing unit code and `roles` `faculty@university.edu` inserts a `faculty` row with `is_primary` true and `source` `scim`.
2. `title` `Department Chair` with no scaffoldry role does not insert `unit_admin`.
3. The same user plus `{ "value": "unit_admin", "type": "scaffoldry" }` does insert `unit_admin` on that department.
4. PUT that removes the scaffoldry role drops the `scim` admin row and keeps an `api` appointment.
5. GET `/api/v1/orgs/{id}/members` as a Platform Admin returns the faculty row. A caller outside the unit gets 403.

Do not add a dependency. Do not create an org from a department string that does not match.

## How this binds the other plans

- Data grid (`docs/plans/data-grid-parity.md`): the grid edits one app in the open workspace. It does not display the org tree. Column layout stays on `AppView`.
- Process desk (`docs/plans/business-process.md` phase 5): `ProcessDesk` lists waiting decisions only for apps in workspaces that survived `list_workspaces`.
- Workflow builder: visible to workspace `owner` and `admin`, and to an Org Unit Admin in scope. Unchanged for `editor` and `viewer` (they do not open it).

## How to prompt Gemini

```
Read docs/plans/organization.md, docs/plans/README.md, and docs/ARCHITECTURE.md section 4 and section 7.
Implement phase N only.
Write the failing test first and run it.
Do not add dependencies.
Do not give a unit two parents.
Do not open /admin to anyone but a Platform Admin (affiliation central_admin, or a platform_admin role on the root).
Use the labels Platform Admin and Org Unit Admin. Do not write Super Admin, System Admin, or Org Admin.
Do not infer either admin role from the job title.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| A research center reports to two colleges. | The center is one unit under the institution. The two deans each get an Org Unit Admin appointment only if a Platform Admin grants it. They do not become a second parent. |
| Department staff will open the Platform Admin console. | `/admin` stays `central_admin` only. Org Unit Admins get a rail filter, not policy, ledger, or impersonation. |
| Workspace membership will leak across departments. | `list_workspaces` drops rows outside `unit_in_scope` before the collaborator check. A sibling department admin gets neither the row nor a 200 for that id. |
| The org tree will drift from CEDS. | `parent_id` is the parent organization. `org_type` is a closed five-value list. No extra hierarchy table. |
| Reparenting will orphan FERPA data. | Only a Platform Admin can change `parent_id`. The walk rejects a cycle. `ON DELETE RESTRICT` on `parent_id` stays. There is no delete route. |


## Completion Report (Phases 1–5 Verified)

| Phase | Description | Status | Verification |
| --- | --- | --- | --- |
| **Phase 1: Pure function, types, and schema** | Added `0004_organization_scope.sql`, `0006_role_source.sql`. Implemented `unit_in_scope` pure function with depth-32 cycle guard. Enforced closed `org_type` values (`Institution`, `College`, `Department`, `Center`, `Program`). Registered in `ServerState` and `PostgresRepository`. | **COMPLETED** | `crates/scaffoldry-server/tests/organization_scope_test.rs` |
| **Phase 2: Organization API and workspace scoping** | Implemented `GET/POST/PATCH /api/v1/orgs`, `POST /api/v1/orgs/{id}/appointments`, `GET /api/v1/orgs/{id}/members`. Updated `service/workspaces.rs` to filter workspaces by `unit_in_scope` before collaborator checks. | **COMPLETED** | `test_phase2_organization_api_and_scoping` in `api_integration_test.rs` |
| **Phase 3: Platform Admin console** | Renamed `/admin` tab to "Organization" (`data-testid="admin-org-tab-btn"`). Added interactive tree viewer (`org-node-{code}`), detail inspector (`org-detail`), new unit form (`org-create-form`), and appointment form (`org-appoint-form`). Preserved DNS vanity routing. | **COMPLETED** | `apps/web/src/test/organization-admin.test.tsx` (all 3 tests pass) |
| **Phase 4: Org Unit Admin rail** | Rendered `data-testid="unit-admin-rail"` with "Your units" when caller is non-central admin with unit appointments. Unit buttons filter workspace list; `New workspace` button appears. | **COMPLETED** | `apps/web/src/test/organization-rail.test.tsx` (all 3 tests pass) |
| **Phase 5: SCIM academic and position sync** | Implemented `sync_org_roles` matching `department`, `division`, and `organization` to units. Deepest match gets `is_primary = true`. Scoped affiliations parsed from eduPerson tokens. Scaffoldry roles (`unit_admin`, `platform_admin`) mapped. Deletes only `source = "scim"` roles. Set `ServiceProviderConfig.patch.supported = false`. | **COMPLETED** | `test_phase5_scim_academic_and_position_sync` in `api_integration_test.rs` |
