# Admin console — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 0 has no dependencies. The other phases list theirs.

The admin console is where an institution runs Scaffoldry: people, workspaces, data labels, policy, business processes, and the audit trail. It is the human side of the MCP platform. An agent builds apps. An administrator governs them here.

The areas below are the ones an enterprise admin panel covers, such as Airtable's. They come from general knowledge of that product, not from its current screens. This brief is the specification. It is not a copy of Airtable.

| Area | Console section | Phase |
| --- | --- | --- |
| Account overview and health | Overview | 1 |
| Users, groups, admin roles | People | 2 |
| Organization units | Organization | organization.md phase 3 |
| Workspaces and bases | Workspaces & apps | 3 |
| Field manager, data classification | Data governance | 4 |
| Security policy, compliance mapping | Policy & OSCAL | 5 |
| Automations manager | Business processes | 6 |
| Audit log, API tokens, integrations | Audit & security | 7 |
| Account-wide settings | Settings | foundation.md phase 9 |

## Decisions already made

Do not reopen these.

| Question | Answer |
| --- | --- |
| Who may open the console? | A Platform Admin: `central_admin`, or a `platform_admin` appointment on the root unit. Two panels also admit affiliation `compliance`: Data governance labels, and the audit log with OSCAL requirements |
| What do Org Unit Admins get? | Nothing here. They keep the workspace rail from `docs/plans/organization.md`. `/admin` stays closed to them |
| Who enforces access? | The server. The client hides a menu item for tidiness. It decides nothing |
| Can an agent change institution configuration? | No. Admin writes are not MCP tools. One read-only tool, `admin_inventory`, lets an agent inspect (phase 7) |
| Where do process definitions get edited? | In the workflow builder inside the app. The console governs them: inventory, history, enable or disable, simulate. It does not duplicate the editor |
| Is there delete? | No. Not for people, units, workspaces, apps, rules, or ledger rows. Hold, revoke, disable, and cancel exist |
| What is a reason? | Every admin write carries a `reason` of 1 to 500 characters. It becomes the ledger rationale |
| Who may activate a policy? | A different person from the author, always. With one Platform Admin, the setup identity from `scaffoldry-server setup-token` counts as the second person. No setting relaxes this |
| What builds the screens? | The kit in `apps/web/src/ui/` from `ux-standards.md`. The console adds no component that duplicates it |
| Are control ids shown bare? | No. A control id always carries its title, from the NIST catalogs in `oscal-catalog.md` |

## What exists today

Each row was read in the code on 2026-10-08.

| Fact | Where |
| --- | --- |
| The console has five tabs held in one `adminTab` state: org, policy, ledger, infra, impersonation | `apps/web/src/AdminDesk.tsx`, `AdminConsoleView.tsx` |
| The console opens only when the active persona's affiliation string is `central_admin`. The default persona is a faculty member, so `/admin` shows `403` | `AdminDesk.tsx` `PERSONAS[0]` |
| Vite has no proxy. `GET /api/v1/...` returns the app's HTML with status 200. Every desk call ends in `.catch(() => {})`, so the failure is silent | `apps/web/vite.config.ts`, `AdminDesk.tsx` |
| The policy tab lists `INITIAL_SOURCE_RULES`, a hard-coded array. Nothing is editable | `AdminDesk.tsx` |
| The infrastructure tab shows a hard-coded Cloud Run address | `AdminConsoleView.tsx` |
| The ledger tab's verify button shows a toast. It never calls the server | `AdminConsoleView.tsx` |
| The OSCAL download is built in the browser from a fixed object | `AdminDesk.tsx` `handleDownloadOscal` |
| The server's OSCAL export turns each ledger entry into an "implemented requirement". An auditor will not accept that | `state.rs` `export_oscal_component_definition` |
| Ledger control ids look like `AC-02`. OSCAL's 800-53 ids look like `ac-2` | `routes/governance.rs`, `state.rs` |
| `GET /policies` returns three hard-coded policies. The engine runs a different, compiled-in, 21-rule string. No one can change it | `routes/policy.rs`, `scaffoldry-policy/src/lib.rs` |
| `POST /scim/v2/Users` always inserts a new row with a new id. A second push for the same `userName` makes a second user | `routes/scim.rs` `create_user` |
| A saved automation overwrites the old one. There is no history | `routes/apps.rs`, `workflow_automations` |
| A process instance names a rule id but not a version. Editing a rule changes decisions already waiting | `scaffoldry-core/src/workflow.rs` `ProcessInstance` |
| A ledger row stores its `payload`. The Rust `LedgerEntry` keeps only `payload_hash`, so nothing can show the payload | `repository.rs` `get_ledger` |
| `docs/plans/business-process.md` ends with a "Completion Report" that says phases 1 to 6 are verified. It names routes (`/workspaces/{id}/process-instances`) and effects (`CedarPolicyDenied`, `UserTaskCreated`) that are not in the code. The code has `/apps/{slug}/processes` and four `ActionEffect` variants | `routes/apps.rs`, `workflow.rs` |

Treat that completion report as unverified. Phase 6 starts by running the tests it names.

## Out of scope

- Billing, seats, or licences.
- SSO configuration beyond the three `oidc.*` settings from `foundation.md`.
- Editing SCIM groups. They are listed read-only. A group is not an org appointment.
- Archiving or deleting a workspace or app.
- A process definition editor, a graph, or a node library.
- Sign-in events and usage charts. The server does not record sign-ins. `last_used_at` on a token is all there is.
- Showing record contents in the console. It shows ids and metadata only.
- Branding, themes, or layout options.
- Any new npm package or crate.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency. Build every screen from the kit in `apps/web/src/ui/` (`ux-standards.md` phase 1). Do not write a second table, drawer, or confirm component.
3. Every admin route calls `service::admin::require_platform_admin(caller, state)` first, or `require_platform_admin_or_compliance` for the routes marked in this brief. Do not compare `affiliation == "central_admin"` in new code.
4. Every admin route is listed once in a constant `ADMIN_ROUTES: &[(&str, &str)]` (method, path) in `service/admin.rs`. A table-driven test iterates it and expects `403` for a faculty caller and `401` with no token. A route added without a row fails a second test that reads the router's path list.
5. Every list route returns `{ "rows": [...], "next_cursor": "..." | null }`, takes `limit` up to 200 and an opaque `cursor`, and never returns more than 200 rows. Write `encode_cursor` and `decode_cursor` once, in `service/admin.rs`.
6. Every write takes `reason`. A missing or empty reason is `400`. Use one helper, `admin_write`, that appends the ledger entry first and runs the change only if the append succeeded. Do not write `let _ =` in front of a ledger or repository call.
7. Every value from a caller is a bound parameter. A column name used for sorting comes from a fixed list in the code, never from the request.
8. New web code goes in `apps/web/src/admin/`. Do not add panels to `AdminDesk.tsx` or `AdminConsoleView.tsx`, except the mount point in phase 1.
9. Keep these test ids: `admin-access-denied`, `admin-org-tab-btn`, `admin-impersonation-tab-btn`, `menu-impersonation-hub-btn`. Do not rename an existing test.
10. Plain words in every label and error. No acronym without its name on first use in a panel.

Ledger controls for admin actions. These are a proposal to the institution's compliance officer. A wrong mapping is a one-line change.

| Action | `oscal_control_id` |
| --- | --- |
| User hold, release, create, role grant or revoke, token revoke | `AC-02` |
| Data label and dataset classification change | `RA-02` |
| Policy activation, policy case change, OSCAL requirement edit | `AC-03` |
| Process definition change | `CM-03` |
| Process reassign and cancel | `AC-03` |

## Phase 0 — the console opens and tells the truth

Web only. Run it after Gemini's current organization work is committed. It touches `AdminDesk.tsx`.

1. `apps/web/vite.config.ts`: add `server.proxy` for `/api`, `/scim`, and `/.well-known`. The target is `process.env.SCAFFOLDRY_API_URL`, default `http://127.0.0.1:8080`.
2. `apps/web/src/api.ts` `request`: a 2xx response whose `content-type` is not JSON is an `ApiError` with status 0 and the message `The server answered with a web page, not data. Is the API running?`
3. `AdminDesk.tsx`: replace the `.catch(() => {})` calls in the sync effect with one `apiStatus` state: `ok`, or `unreachable` with the message. Keep the sample data as the fallback, and label it.
4. When `apiStatus` is `unreachable`, show a banner above the main canvas: `data-testid="api-unreachable"`, the text `The API server is not reachable. What you see may be sample data.`, and a `Retry` button that runs the sync again.
5. Admin panels never show sample data. On a failed load a panel shows its error message, `data-testid="admin-load-error"`, and a `Retry` button.
6. Add a top-bar link `Admin Console`, `data-testid="admin-console-link"`, visible when the active persona's affiliation is `central_admin`.
7. `README.md`: under local development, one line: the Vite server proxies `/api` to `SCAFFOLDRY_API_URL`.

Tests (vitest, `fetch` stubbed):

1. A `200` response with `content-type: text/html` makes `apiClient.listWorkspaces` throw an `ApiError` with status 0. Put it in `api-client.test.ts`.
2. When `listWorkspaces` rejects, `api-unreachable` is on screen. After a stubbed success and a click on `Retry`, it is gone.
3. The organization panel with a rejecting `listOrganizations` shows `admin-load-error` and does not show the sample tree.
4. `admin-console-link` is present for `jordan.lee@state.edu` and absent for a faculty persona.

## Phase 1 — a real shell, one guard, one overview

Needs `foundation.md` phases 1 to 9, `organization.md` phases 1 to 3, and `ux-standards.md` phase 1.

Backend.

1. In `service/organizations.rs`, add `is_platform_admin(caller, orgs, roles) -> bool`. It is true for `central_admin`, or for a `platform_admin` role on the root unit. Make `unit_in_scope` call it. Do not write the rule twice.
2. New file `service/admin.rs`: `require_platform_admin`, `require_platform_admin_or_compliance`, `ADMIN_ROUTES`, `encode_cursor`, `decode_cursor`, and `admin_write`.
3. `GET /api/v1/auth/me` gains `is_platform_admin: bool`.
4. `DecisionType` gains: `UserAccessChanged`, `AccessRoleRevoked`, `TokenRevoked`, `DataLabelChanged`, `ProcessDefinitionChanged`, `ProcessInstanceReassigned`, `ProcessInstanceCancelled`.

   Today a variant must be added in three places: the enum, the match in `compute_entry_hash`, and `parse_decision_type`. Replace that with one definition. Add `DecisionType::ALL`, `as_str`, and `parse`. `compute_entry_hash` and the repository call them. Before the change, compute and write down the `entry_hash` of one fixed entry for each existing variant. After the change, the same entries must give the same hashes.
5. `GET /api/v1/admin/overview`:

   | Key | Value |
   | --- | --- |
   | `server` | Version, applied migration filenames from `schema_migrations`, database worker count |
   | `people` | Users active, on hold, inactive. Platform Admins. Active tokens by kind |
   | `organization` | Unit count |
   | `workspaces` | Workspace count, app count |
   | `processes` | Waiting instance count |
   | `ledger` | Entry count and head hash. Not a verification. Verification is an explicit action in phase 7 |

   Later phases add keys. Each adds its own test.

Frontend. Create `apps/web/src/admin/`:

- `AdminConsole.tsx`: a left navigation and a content area. Routes are `/admin/{section}`. `/admin` opens Overview.
- Panels use `DataTable`, `Drawer`, `ConfirmAction`, and `useAsync` from `apps/web/src/ui/`. A failed load shows the kit's `ErrorState` inside an element with `data-testid="admin-load-error"`.
- `Overview.tsx`: cards from the overview response.

`AdminDesk.tsx` renders `<AdminConsole />` when the path is an admin path. Keep the Policy, Ledger, and Impersonation tabs working unchanged inside the new shell. Phases 2, 5, and 7 replace them. Delete the Cloud Run infrastructure tab. The overview's server card replaces it.

The shell shows `admin-access-denied` when `GET /auth/me` says `is_platform_admin` is false and the affiliation is not `compliance`.

Tests.

1. `ADMIN_ROUTES` covers `GET /admin/overview`. A faculty caller gets 403. No token gets 401.
2. Every `DecisionType` in `ALL` satisfies `parse(as_str(v)) == Some(v)` and `as_str(v) == format!("{v:?}")`. The pinned hashes from step 4 are unchanged.
3. `unit_in_scope` and `is_platform_admin` agree for a `platform_admin` role on the root and for a `central_admin`. The existing `organization_scope_test.rs` still passes.
4. The overview counts match a fixture: three users, one on hold, two units, one workspace.
5. Web: a faculty persona sees `admin-access-denied`. A Platform Admin sees the overview cards from a stubbed response. `DataTable` with a stubbed `load` shows the first page and `Load more` calls `load` with the cursor.

## Phase 2 — People

Needs phase 1 and `foundation.md` phase 7 (`resolve_user`).

First, fix SCIM. `create_user` in `routes/scim.rs` must update the existing row when the `userName` already exists, and keep its id. Return 200 for an update and 201 for a create. Add `admin_hold: bool` with `#[serde(default)]` to `ScimUser`. `update_user` copies the stored `admin_hold` forward and ignores any value in the body. SCIM never sets or clears a hold. `resolve_user` returns `None` when the user is inactive or on hold.

Routes. All Platform Admin.

| Method | Path | Behavior |
| --- | --- | --- |
| GET | `/admin/users` | Query `search`, `active`, `hold`, `affiliation`, `unit`, `cursor`, `limit`. Sorted by `user_name`. Search matches the start of user name, display name, or email. Each row: id, user name, display name, email, affiliation, units, active, hold, `platform_admin`, `unit_admin_of`, active agent token count, latest token use |
| GET | `/admin/users/{id}` | The row, plus appointments with their `source`, workspace memberships, the user's agent tokens (never hashes), and the last 20 ledger entries where the user is the principal |
| POST | `/admin/users` | Body `userName`, `name`, `email`, `affiliation`, `department`, `title`, `reason`. Writes the same user store SCIM writes. A later SCIM push for the same `userName` updates this row |
| POST | `/admin/users/{id}/hold` | Body `hold` (bool), `reason`. Ledger `UserAccessChanged`. The user's tokens stop working on the next request and work again when the hold is lifted |
| POST | `/admin/users/{id}/revoke-tokens` | Revokes every `agent` and `impersonation` token of that user. Ledger `TokenRevoked` with the count. Does not touch `scim` tokens |
| GET | `/admin/groups` | SCIM groups: name, member count. Read-only |
| DELETE | `/orgs/{id}/appointments/{eppn}/{scoped_affiliation}` | Revokes an appointment. Platform Admin, or an Org Unit Admin in scope for a `unit_admin` appointment. Refused with 409 when its `source` is `scim`: the registry owns it. Refused with 409 when it is the last `platform_admin` appointment. Ledger `AccessRoleRevoked` |

Also extend `POST /orgs/{id}/appointments` to accept `platform_admin`, only on the root unit and only from a Platform Admin. Ledger `AccessRoleGranted`.

Frontend: `admin/People.tsx`.

- An `DataTable` with search and the filters Active, On hold, Platform Admin, Org Unit Admin.
- A row opens a drawer, `data-testid="admin-user-detail"`, with the detail response and these actions: `Hold` or `Release`, `Revoke tokens`, grant or revoke Org Unit Admin on a unit chosen from a list, grant or revoke Platform Admin, `Impersonate`.
- Every action asks for a reason in the drawer and has a `Confirm` button. Do not use `window.confirm`.
- Form `admin-user-create` for `POST /admin/users`.
- `Impersonate` calls the existing `apiClient.impersonateUser`. Replace the hard-coded persona list in the Identity & Impersonation tab with this People table. Keep `admin-impersonation-tab-btn` and make it open People filtered to a Platform Admin's view.

Tests.

1. Three users, `limit` 2: the first page has two rows and a cursor. The second has one and `null`.
2. `POST /scim/v2/Users` twice with the same `userName` leaves one user. The second call returns 200 and the same id.
3. Put a user on hold. Their next request with a valid token is 401. Release. It is 200. A SCIM `PUT` for that user does not clear the hold.
4. Revoking tokens makes that user's agent token 401 and leaves a `scim` token working.
5. Revoking the only `platform_admin` appointment is 409. Revoking a `source = scim` appointment is 409. Revoking an `api` `unit_admin` appointment as an Org Unit Admin in scope is 200.
6. Every route above is in `ADMIN_ROUTES`, except the appointment route, which has its own scope test.
7. Each write adds one ledger entry with the admin as principal, the stated reason as rationale, and the matching `DecisionType`.
8. Web: the detail drawer shows tokens and appointments. `Hold` without a reason is not sent. `Confirm` sends `POST /admin/users/{id}/hold`.

## Phase 3 — Workspaces and apps

Needs phase 1 and `foundation.md` phase 5 (apps belong to workspaces).

| Method | Path | Behavior |
| --- | --- | --- |
| GET | `/admin/workspaces` | Query `search`, `unit`, `classification`, `visibility`, `cursor`, `limit`. Row: id, name, code, unit name, classification, visibility, owners, collaborator count, app count, record count, created. Record counts come from one `SELECT app_slug, count(*) ... WHERE app_slug = ANY($1) GROUP BY app_slug` for the page's apps |
| GET | `/admin/workspaces/{id}` | The row, plus collaborators and apps |
| PATCH | `/admin/workspaces/{id}` | Body any of `organization_id`, `data_classification`, `visibility`, and `reason`. Calls the existing `update_workspace` service, which already evaluates Cedar and writes the ledger. Add `organization_id` to `UpdateWorkspacePayload`. The unit must exist. Closed lists: classification `Public`, `Internal`, `Restricted`, `FERPA Sensitive`. Visibility `restricted`, `departmental`, `institutional`. The ledger payload holds the old and new value of each change |
| POST | `/admin/workspaces/{id}/transfer-ownership` | Body `eppn`, `reason`. The new owner must resolve through `resolve_user`. They become `owner`, added as a collaborator if missing. Existing owners become `admin`. The workspace always keeps one owner. Ledger `WorkspaceMemberRoleUpdated` |
| GET | `/admin/apps` | Query `search`, `workspace`, `cursor`, `limit`. Row: slug, title, workspace, version, table count, page count, custom page count, record count per table, updated |

`ponytail`: the user and manifest lists are built in memory from `ServerState` and sorted per request. That is comfortable to a few thousand users and apps. Move the sort into SQL when a list is measured above 100 ms.

Frontend: `admin/Workspaces.tsx`.

- A workspaces table, and a drawer `admin-workspace-detail` with collaborators, apps, and edit controls for unit, classification, and visibility. The unit control is a list from `GET /orgs`.
- `Transfer ownership` takes a user chosen with the People search.
- An apps table with an `Open` link to the app.

Tests.

1. A workspace fixture with two apps and five records: counts are correct and use one statement for the page.
2. `PATCH` moving a workspace to another unit changes `organization_id` and writes one ledger entry holding both ids. An unknown unit is 400. A faculty caller is 403.
3. Transfer ownership: the new owner is `owner`, the old owner is `admin`, one owner remains. Transferring to a held user is 400.
4. A bad classification string is 400.
5. Web: the drawer sends only the changed fields with the reason.

## Phase 4 — Data governance

Needs phases 1 and 2. Run it after `mcp-apps.md` phases 4 and 6 and `row-scale.md` phase 5. Those add the places that read a field's sensitivity, and this phase changes all of them.

Split. Run the first part of this phase early, in the README row named for it: `effective_ferpa_sensitive` with an empty label set, and the grep that replaces every read of a field's flag. Every brief that reads sensitivity calls that function from the day it exists. The label table, the routes, and the screens are the later part.

Why a label and not an edit. Changing a field's sensitivity in the manifest is a structural change and needs a proposal. A compliance officer must be able to raise a field's sensitivity at once, with no one's approval. The label table is an overlay that can only add protection.

Migration `crates/scaffoldry-core/migrations/0017_data_labels.sql`:

```sql
CREATE TABLE IF NOT EXISTS data_labels (
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    field VARCHAR(64) NOT NULL,
    ferpa_sensitive BOOLEAN NOT NULL,
    note TEXT NOT NULL,
    set_by VARCHAR(255) NOT NULL,
    set_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (app_slug, table_id, field)
);
```

Effective sensitivity. In `scaffoldry-engine`, add `effective_ferpa_sensitive(field: &FieldSpec, label: Option<&DataLabel>) -> bool`. It is true when the manifest flag is true or the label's flag is true. A label of `false` never lowers a manifest flag. `ServerState` holds the labels in one `RwLock<HashMap>`, loaded at boot and replaced after each change. The table holds exceptions only, so it stays small.

Run `grep -rn "ferpa_sensitive" crates/` and replace every read of a `FieldSpec`'s flag, outside `validate_manifest` and this function, with `effective_ferpa_sensitive`. Paste the grep output before and after. That covers record submit, the viewer filter, search, aggregates, and the page sensitivity check.

Recompute stored flags. `dataset_records.is_ferpa_sensitive` is set when a record is written. After a label change, one statement fixes it for the table:

```sql
UPDATE dataset_records
SET is_ferpa_sensitive = jsonb_exists_any(data, $3)
WHERE app_slug = $1 AND table_id = $2
```

`$3` is the array of the table's effective sensitive field names. It runs in the same transaction as the label change.

Routes.

| Method | Path | Who | Behavior |
| --- | --- | --- | --- |
| GET | `/admin/fields` | Platform Admin or `compliance` | Query `search`, `app`, `table`, `sensitive`, `type`, `suggested`, `cursor`, `limit`. One row per field of every manifest: app, table, field, label, type, manifest flag, overlay flag, effective flag, CEDS code, `suggested`, and `used_in` |
| PUT | `/admin/labels` | Platform Admin or `compliance` | Body `changes: [{ app_slug, table_id, field, ferpa_sensitive }]` and `reason`. `ferpa_sensitive: null` removes the overlay. Up to 200 changes. Every field must exist in its manifest, or the whole request is 400. One ledger entry, `DataLabelChanged`, `RA-02`, listing the changes. Then labels are written, flags recomputed, and the cache replaced, in one transaction |
| GET | `/admin/datasets` | Platform Admin or `compliance` | Published datasets: id, name, department, `sensitivity_level`, record count |
| PUT | `/admin/datasets/{id}/classification` | Platform Admin or `compliance` | Body `sensitivity_level` and `reason`. Closed list: `Public`, `Directory`, `Internal`, `Restricted / FERPA`. Updates the column and the stored payload together. Ledger `DataLabelChanged` |

`suggested` is true when the field's CEDS code is sensitive under `CedsElement::is_ferpa_sensitive` and the effective flag is false. This uses the standard already in the repo. Do not add a name-based guess.

`used_in` is a count of: views that list, sort, or filter the field, other fields whose formula names it, custom page grants that read or write it, and process rules whose predicates or effects name it. Read `FieldPredicate` and `ActionEffect::SetFields` to find the field name. Build it from the manifests and rules in memory.

Frontend: `admin/DataGovernance.tsx` with two tabs.

- Fields: a `DataTable` with the filters Sensitive, Not sensitive, Suggested. Rows can be selected. `Mark sensitive` and `Remove label` ask for a reason and send one `PUT /admin/labels`. A row expands to show `used_in`. A badge shows when a field is sensitive only through an overlay.
- Datasets: a table with a classification list per row, a reason, and `Save`.

Tests.

1. Raise a label on a field of a table with three records, two of which hold that field. After the call, those two have `is_ferpa_sensitive = true` and the third does not. Remove the label. The two return to false.
2. A label of `false` on a field that the manifest flags leaves the effective flag true.
3. A viewer's list response no longer contains a field after a label raises it. A new record with that field is flagged sensitive. Search on that table is refused for a viewer. Write the three assertions against whatever the earlier briefs built, and say which exist.
4. A batch with one unknown field is 400 and changes nothing.
5. 201 changes is 400.
6. A `compliance` caller is allowed on `fields`, `labels`, and `datasets`. A faculty caller is 403 on all three. A `compliance` caller is 403 on `/admin/users`.
7. The ledger entry holds the reason and the list of changes.
8. `suggested` returns a field with a sensitive CEDS code and no flag, and omits it once labelled.
9. Web: selecting two rows and `Mark sensitive` sends one request with two changes.

## Phase 5 — Policy and OSCAL

Needs phase 1, `foundation.md` phase 8, `guards.md` phase 1, and `oscal-catalog.md` phase 1.

Today the policy is one string compiled into the binary, and the console shows a different, hard-coded list. This phase makes the running policy a stored, versioned, testable thing.

Migration `crates/scaffoldry-core/migrations/0018_policy_versions.sql`:

```sql
CREATE TABLE IF NOT EXISTS policy_versions (
    id UUID PRIMARY KEY,
    version INTEGER NOT NULL UNIQUE,
    source TEXT NOT NULL,
    status VARCHAR(8) NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    activated_by VARCHAR(255),
    activated_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_policy_one_active ON policy_versions ((status)) WHERE status = 'Active';

CREATE TABLE IF NOT EXISTS policy_cases (
    id UUID PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    principal JSONB NOT NULL,
    action VARCHAR(64) NOT NULL,
    resource JSONB NOT NULL,
    expected VARCHAR(5) NOT NULL,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS governance_documents (
    kind VARCHAR(32) PRIMARY KEY,
    document JSONB NOT NULL,
    updated_by VARCHAR(255) NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

`status` is `Draft`, `Active`, or `Retired`. A case's `principal` and `resource` are Cedar entity JSON: `{ "type": "...", "id": "...", "attrs": { ... } }`.

Engine, in `scaffoldry-policy`.

1. Add `is_allowed(principal_entity, action, resource_entity) -> AuthorizationResult`. Refactor `authorize_record_action`, `authorize_workspace_action`, `authorize_institutional_action`, and `authorize_departmental_action` to build their entity JSON and call it. Every existing test in `cedar_authorization_test.rs` and `automation_test.rs` passes unchanged.
2. Add `source()` to return the policy text.
3. Add annotations to each of the 21 default policies: `@id("...")`, `@oscal("ac-3")`, and `@source("...")` where a statute applies. Annotations do not change what a policy decides. Add `policy_summaries()` returning id, effect, oscal control, source, and text, read with `Policy::annotation`. If that method has another name in `cedar-policy` 4.13, use the real one and say so.

Server.

1. `ServerState.policy_engine` becomes `RwLock<Arc<ScaffoldryPolicyEngine>>`, read through `state.policy()`. Replace every use. This includes `AutomationEngine::new(...)` and any engine that `ManifestEngine` builds for itself.
2. Boot. With no `policy_versions` row, insert version 1 as `Active`, from the compiled default, and insert one `policy_cases` row for each assertion in `cedar_authorization_test.rs`. With an `Active` row, load it. If its source does not parse, boot fails. Do not fall back to the default.

Routes. All Platform Admin unless marked.

| Method | Path | Behavior |
| --- | --- | --- |
| GET | `/admin/policy/active` | The active version, its source, and `policy_summaries()` |
| GET | `/admin/policy/versions` | Version, status, note, created by and at, activated by and at |
| GET | `/admin/policy/versions/{id}` | One version with its source |
| POST | `/admin/policy/versions` | Body `source`, `note`. Parses the source. A parse error is 400 with each message and its line and column. Stores a `Draft` |
| POST | `/admin/policy/versions/{id}/clone` | A new `Draft` from any version. This is rollback: clone the old version, then activate |
| POST | `/admin/policy/versions/{id}/run-cases` | Runs every case against that version in a temporary engine. Returns each case's expected result, actual result, and pass or fail |
| POST | `/admin/policy/versions/{id}/activate` | Body `reason`. Refused with 409 listing failing cases when any case fails. Refused with 403 when the caller created the draft. There is no exception. The setup identity from `scaffoldry-server setup-token` is a different person and may activate. In one transaction: ledger entry `PolicyRevision` holding the version number, the SHA-256 of the source, and the previous version, then the old row `Retired`, this row `Active`. After commit, swap the engine |
| GET, POST, DELETE | `/admin/policy/cases`, `/admin/policy/cases/{id}` | List, add, and remove cases. Adding and removing each write a `PolicyRevision` ledger entry |
| POST | `/admin/policy/simulate` | Body `version_id` (optional, default active), `principal`, `action`, `resource`. Returns the decision, reasons, and diagnostics from the real engine |

OSCAL. The mapping from statute to control to policy is OSCAL data. Store it as an OSCAL `component-definition`, in `governance_documents` with `kind = 'component-definition'`. Extend OSCAL through `props` only.

| Method | Path | Who | Behavior |
| --- | --- | --- | --- |
| GET | `/admin/oscal/requirements` | Platform Admin or `compliance` | One row per OSCAL control id found in the stored document or in an `@oscal` annotation of the active policy: control id, statement, `implementation-status`, linked policy ids, ledger evidence count, latest evidence time |
| PUT | `/admin/oscal/requirements/{control_id}` | Platform Admin or `compliance` | Body `statement`, `status`, `reason`. `status` is one of `implemented`, `partial`, `planned`, `alternative`, `not-applicable`. Updates the stored document. Ledger `PolicyRevision`, `AC-03` |
| GET | `/admin/oscal/component-definition` | Platform Admin or `compliance` | The exported document |

The export holds one `implemented-requirement` per control. Each carries the `implementation-status` prop, and one `cedar-policy-id` prop per linked policy. Use the namespace `https://scaffoldry.io/ns/oscal` for `cedar-policy-id`. Evidence is the count of ledger entries for that control, as a prop. A ledger entry is evidence. It is not a requirement. Delete the code that turns ledger entries into requirements. `GET /governance/oscal/export` and the MCP tool `export_oscal_compliance` call the same function. Delete `handleDownloadOscal` from the web app.

Normalize control ids with `catalog::normalize` from `oscal-catalog.md` phase 1. Control titles, validation of control ids, baselines, and coverage are `oscal-catalog.md` phase 2. It edits the requirements routes and the OSCAL tab built here.

Read `governance/scripts/validate-oscal.py` and `governance/schema/oscal_complete_schema-1.1.2.json`. Check that `implementation-status` is a defined property name in OSCAL 1.1.2. If it is not, use a prop in the Scaffoldry namespace and say so. Add a test that writes the export to the path in `OSCAL_OUT` when set. Add a step to `ci.yml` that sets it, runs that test, and runs `validate-oscal.py` on the file. If the script takes no path argument, add an optional first argument.

Frontend: `admin/PolicyOscal.tsx`. Delete `INITIAL_SOURCE_RULES`. Five tabs:

- Active policy: the source, read-only, and the summary table from `policy_summaries()`.
- Versions: a list, `New draft` with a monospace editor, and for a draft: `Validate`, `Run cases`, `Activate`. Parse errors show line and column. Failing cases show their names.
- Test cases: a table, an add form with JSON fields for principal and resource, and `Remove`.
- Simulator: principal, action, and resource as JSON, a version list, and `Run`.
- OSCAL requirements: a table with `Edit` for statement and status, and `Export` that downloads the server's document.

Tests.

1. A fresh database has one `Active` version and the default cases. All cases pass against it.
2. A draft with a syntax error is 400 and reports a line.
3. A draft that breaks a case: activate is 409 and names the case. After adding a case that expects the new behavior and editing the draft to match, activate is 200 and the next request reflects it with no restart.
4. Activation writes one `PolicyRevision` entry holding the SHA-256 of the source.
5. The creator cannot activate their own draft, even as the only Platform Admin. The setup identity can.
6. A database whose `Active` source does not parse makes `ServerState::new` return an error.
7. `policy_summaries()` returns an id, effect, and OSCAL control for each default policy.
8. The export has no `implemented-requirement` whose only source is a ledger entry. A control with three ledger entries shows evidence count 3.
9. Control ids in the export are normalized with `catalog::normalize`.
10. A `compliance` caller can edit OSCAL requirements and cannot create or activate a policy version.
11. The CI validation passes on the exported file.
12. Web: a failing-case response lists case names and disables `Activate`.

Workspace rules are `guards.md`. The institutional policy here and a workspace's guards are evaluated together by `service::access::decide`. A draft is validated against the Cedar schema from `guards.md` phase 1 as well as parsed. A draft that does not type-check is 400 with each error and its line.

## Phase 6 — Business processes

Needs phases 1 and 2, `approvers.md` phase 3, and `mcp-apps.md` phase 4a. Those build rule versions, `save_rule`, `describe_rule`, the proposal path for definitions, approver resolution, and `can_decide`. This phase is the console on top of them. Do not repeat that work.

The console reads `workflow_rule_versions`. It calls `save_rule` only for enable and disable, below. Verifying the old completion report is `README.md` row 9, before this phase.

Routes. All Platform Admin.

| Method | Path | Behavior |
| --- | --- | --- |
| GET | `/admin/processes` | Query `app`, `enabled`, `search`, `cursor`, `limit`. Row: rule id, name, app, workspace, version, enabled, trigger, step count, user task roles, waiting count, last changed by and at |
| GET | `/admin/processes/{rule_id}` | The live rule, `describe_rule` sentences, version history (version, changed by and at, summary, SHA-256), instance counts by status |
| GET | `/admin/processes/{rule_id}/versions/{v}` | That version's JSON and sentences, read-only |
| PATCH | `/admin/processes/{rule_id}` | Body `enabled`, `reason`. Creates a new version through `save_rule` with no proposal, because turning a rule on or off is the administrator's own authority and is ledgered. A disabled rule starts no new instances. Waiting ones can still be decided |
| POST | `/admin/processes/{rule_id}/simulate` | Body `record` (JSON) or `record_id`, and `principal` (a user name). Resolves the principal through `resolve_user`. Calls the same function as the app's simulate route. Extract that function from `routes/apps.rs` into `service/processes.rs` and call it from both |
| GET | `/admin/process-instances` | Query `status`, `app`, `rule`, `stale`, `assigned`, `unassigned`, `cursor`, `limit`. Row: id, app, rule, rule version, record id, status, who decides (names), `no_approver`, assigned to, started, age in days, `stale` |
| GET | `/admin/process-instances/{id}` | The instance and its log. Not the record's contents |
| GET | `/admin/process-instances/{id}/approvers` | The users who can decide now, from `resolve_approvers`, with each person's `via` and the unit the position was found at |
| POST | `/admin/process-instances/{id}/reassign` | Body `eppn` (or `null` to return to role-based), `reason`. The instance must be `Waiting`, else 409. The target must resolve, be active and not on hold, have read access to the app, and not be the record's creator or the person whose change started the process (400 `separation_of_duties`). Adds a log line. Ledger `ProcessInstanceReassigned`, `AC-03` |
| POST | `/admin/process-instances/{id}/cancel` | Body `reason`. A `Waiting` instance becomes `Failed` with the log line `cancelled by {eppn}: {reason}`. No effects run. The record is unchanged. Ledger `ProcessInstanceCancelled`, `AC-03` |

`stale` is true for a `Waiting` instance older than the `process.stale_days` setting. Add that key to the closed list in `foundation.md` phase 8: integer 1 to 365, default 14. Nothing in this plan sends a reminder. The console shows what is stuck. A person acts.

Frontend: `admin/Processes.tsx` with two tabs.

- Definitions: a `DataTable`. A row opens a drawer, `admin-process-detail`. It shows the sentences, a toggle to see the JSON, the version history with each version viewable, `Enable` or `Disable` with a reason, `Simulate`, and `Edit in app builder`, which opens the app's workflow tab.
- Instances: a `DataTable` with the filters Waiting, Stale, and Assigned. A row has `Who can decide`, `Reassign` (user chosen with the People search, plus a reason), and `Cancel` (confirm plus a reason).
- The Overview gains Waiting and Stale counts.

Tests.

1. Disable a rule. A new matching record starts no instance. An existing waiting instance can still be decided.
2. Disable writes a new version with no proposal id and one ledger entry.
3. Reassign to a held user is 400. Reassign to the record's creator is 400 `separation_of_duties`. Reassign a `Completed` instance is 409. After a reassign, `approvers` returns only the assignee.
4. Cancel leaves the record unchanged, sets `Failed`, and writes one ledger entry.
5. `stale` is true for an instance whose `started_at` is 15 days old and false at 13, with the default setting.
6. `unassigned=true` returns exactly the instances that have a `no_approver` problem.
7. The admin simulate route and the app simulate route return the same result for the same input.
8. Every route above is in `ADMIN_ROUTES`.
9. Web: a definition drawer shows sentences. `Disable` without a reason is not sent. `Reassign` posts the chosen user. The Unassigned filter lists an instance and its row says why in words.

## Phase 7 — Audit and security

Needs phases 1 and 2 and `mcp-apps.md` phase 1. The page controls need `mcp-apps.md` phase 6. If it is not built, build only the tool controls and say so.

Migration `0020_ledger_filters.sql`, which indexes fixed platform columns only:

```sql
CREATE INDEX IF NOT EXISTS idx_ledger_decision ON governance_ledger (decision_type, sequence DESC);
CREATE INDEX IF NOT EXISTS idx_ledger_app ON governance_ledger (app_slug, sequence DESC);
CREATE INDEX IF NOT EXISTS idx_ledger_control ON governance_ledger (oscal_control_id, sequence DESC);
```

Ledger. Platform Admin or `compliance`.

| Method | Path | Behavior |
| --- | --- | --- |
| GET | `/admin/ledger` | Query `principal`, `decision_type`, `app`, `control`, `from`, `to`, `cursor`, `limit`. Newest first by sequence. Each row includes the stored `payload`. Add `LedgerRow { entry, payload }` and a repository function that reads it |
| POST | `/admin/ledger/verify` | Walks the whole chain. Returns `valid`, entry count, head hash, and the first bad sequence when invalid. This replaces the toast. It is a full scan and the console says so |
| GET | `/admin/ledger/export` | Same filters, `format` of `json` or `csv`, at most 50,000 rows, else 400 telling the caller to narrow the filter. JSON starts with the head hash and the verify result |

Tokens. Platform Admin.

| Method | Path | Behavior |
| --- | --- | --- |
| GET | `/admin/tokens` | Query `kind`, `owner`, `state` (`active`, `expired`, `revoked`), `cursor`, `limit`. Row: id, kind, owner, label, created, expires, last used, revoked, and `original_admin` for impersonation. Never the hash |
| DELETE | `/admin/tokens/{id}` | Body `reason`. Revokes. Ledger `TokenRevoked`. Works for impersonation sessions too |

SCIM tokens are minted with the existing `POST /auth/tokens` and `kind: "scim"` from `foundation.md`. The panel has a `Mint SCIM token` button that calls it and shows the token once.

Agent controls. Add three keys to the closed settings list in `foundation.md` phase 8:

| Key | Type | Meaning |
| --- | --- | --- |
| `mcp.disabled_tools` | array of strings | Each must be a name in `TOOLS`, else 400 |
| `pages.disabled` | array of `"{app_slug}/{page_id}"`, at most 200 | Custom pages that may not run |
| `process.stale_days` | integer 1 to 365 | Added in phase 6 |

These keys only remove capability. None turns a check off. `tools::call` refuses a disabled tool with `Tool disabled by administrator` before it checks scope. `tools/list` omits it. `page_call` and `get_page` refuse a disabled page. `PageFrame` shows `This page was disabled by an administrator.`

`admin_inventory`: one MCP tool, scope `PlatformAdmin`, `read_only: true`. Arguments `kind` (`users`, `workspaces`, `apps`, `fields`, `processes`, `process_instances`, `tokens`, `ledger`), the same filters as the matching REST route, `limit` up to 200, and `cursor`. It calls the same service function as the route. There is no admin write tool.

Frontend: `admin/Audit.tsx` and `admin/Security.tsx`. Replace the old ledger tab.

- Audit: ledger table with the filters, a row drawer that shows the payload and the hashes, `Verify chain` showing the result, and `Export` for JSON and CSV.
- Security: tokens table with `Revoke`, and an impersonation sessions filter with `End session`. `Mint SCIM token`.
- Agent access: a table of tools from `GET /admin/tools` (name, description, read-only flag, disabled) with a toggle each, and a table of custom pages (app, page, grants sentence, approved by when known) with a toggle each. Both write through `PUT /api/v1/settings/{key}`, the one ledgered path.
- The Overview gains token counts, stale processes, and pending proposals when that table exists.

Tests.

1. 120 ledger rows, filter by `decision_type`, `limit` 50: pages are newest first with no repeats. Each row has a payload.
2. `verify` on a clean ledger returns `valid`. With one row's `rationale` changed (drop the trigger inside the test), it returns the first bad sequence.
3. Export with 60,000 matching rows is 400. A JSON export starts with the head hash. A CSV export has a header row.
4. Revoking a token makes it 401 on the next request and adds one ledger entry. The response never contains a hash.
5. Setting `mcp.disabled_tools` to `["create_record"]` removes it from `tools/list` and makes a call a tool error. A name not in `TOOLS` is 400. A faculty caller cannot change the setting.
6. `admin_inventory` is absent from `tools/list` for a faculty caller, is `read_only`, and returns the same rows as `GET /admin/users` for the same filter.
7. No `TOOLS` entry whose name starts with `admin_` is a write tool.
8. The tool-scope test from `mcp-apps.md` phase 1 still passes with the new tool.
9. Every route above is in `ADMIN_ROUTES`, with the `compliance` exceptions tested.
10. Web: the payload drawer shows the stored payload. Toggling a tool sends one `PUT /api/v1/settings/mcp.disabled_tools` with the new list.

## Panels added by other briefs

Each of these adds its routes to `ADMIN_ROUTES`, is built from the kit, writes a ledger entry for every change, and asks for a reason.

| Section | Brief |
| --- | --- |
| Jobs | `jobs.md` phase 2 |
| Disclosures | `record-history.md` phase 4 |
| Shares | `views.md` phase 6 |
| Templates | `import-export.md` phase 4 |
| Connections | `connections.md` phase 6 |
| Outbound access, Secrets, Webhooks | `integrations.md` phase 5 |
| Mail and Notifications | `notifications.md` phases 2 and 3 |
| Process insights | `process-v2.md` phase 1 |
| Retention, Purges, Legal holds | `lifecycle.md` phases 3 and 4 |
| Guests | `guests.md` phase 4 |
| Positions | `approvers.md` phase 2 |
| Flows | `workflows.md` phase 11 |

## How to prompt Gemini

```
Read docs/plans/admin-console.md, docs/plans/README.md, and docs/plans/foundation.md "Decisions already made".
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
Do not compare affiliation to "central_admin" in new code. Use require_platform_admin.
Do not write a route that is missing from ADMIN_ROUTES.
Do not write an admin change without a reason, and do not run it before its ledger entry is written.
Do not put new panels in AdminDesk.tsx.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| An admin console is the biggest target in the product. | Every route checks the caller on the server. `ADMIN_ROUTES` is tested for a faculty caller and for no token, and a second test fails when a route is missing from the list. An agent cannot call any admin write. |
| One administrator can quietly weaken policy. | A policy activation needs a different person from the author, always, and every activation is a ledger entry with the hash of the source. The cases must pass first. |
| A compliance officer cannot wait for a change request to mark a field sensitive. | The label overlay takes effect at once and can only add protection. Removing a manifest's own flag still needs a proposal and a second person. |
| Labels will be forgotten in a code path. | Phase 4 replaces every read of a field's flag with one function, and shows the grep before and after. |
| The audit log will be too big to read. | It is filtered and paged by indexed columns. Export is capped at 50,000 rows and says to narrow the filter. |
| Verifying the ledger will stall the server. | It is an explicit action that reads the table once. The overview shows the head hash and the count without verifying. |
| Administrators will read student records through the console. | The console shows ids and metadata. Process instances show the prompt and the record id, never the record. |
| A held user is still logged in. | A hold takes effect on the next request, because identity is resolved from stored rows on every call. Releasing it restores access. |
| Admins want to edit workflows here. | The builder stays the one editor. The console shows what a rule says in sentences, who changed it and when, and lets an administrator turn it off. |
| The OSCAL export will be rejected by an auditor. | Evidence is no longer relabelled as a requirement. The document is validated against the OSCAL 1.1.2 schema in CI. Control ids are checked against the NIST catalogs and shown with their titles. |
| Moving a workspace between units will expose it. | The move needs a Platform Admin, a stated reason, and a ledger entry holding both unit ids. Scope checks read the new unit on the next request. |
