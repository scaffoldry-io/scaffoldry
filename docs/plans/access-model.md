# Access model — implementation brief

> **Status: Open**

Hand this file to the build agent. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs nothing that is still open. Phase 2 needs `stateless-server.md` phase 3. Each later phase needs the one before it. Phase 7 links to `policy-library.md` phase 4 when that has run. Phase 10 reads findings from `sensitive-content.md` phase 2 when that has run. Phase 12 needs phase 6. Phase 13 needs phase 4 and `authority.md` phase 1. Phase 14 needs phases 7 and 13.

A dean of the College of Science should see the grant records of every department in the college, without anyone adding the dean to forty workspaces. A department chair should see their own department, and when they hand the chair to a successor, the successor should see it the next morning. A registrar's table should show each department only its own students. Nobody should be able to answer "why can this person see this?" with "it's complicated".

Today none of that is possible. Access comes from workspace membership only. An org unit is attached to a workspace and to nothing below it. "Where a person sits" is defined twice, once as a free-text department string and once as the org tree, and the two disagree. Several paths fail open.

This brief gives every object coordinates and every grant one shape. **Every object has a place on three axes: an org unit in each hierarchy, a data domain, and a sensitivity. Every grant is one row: a holder, a role, and a reach over those same axes.** A person's role on an object is the highest role among the grants whose reach contains it. Cedar's forbids, workspace guards, and row and column rules then narrow that, as they do today.

The rule that keeps it maintainable: **a grant's reach depends only on its own row and the trees. It never depends on anyone else's assignment.**

The research behind this design, with sources, is in the owner's report "Hierarchical access grant design". It is not in the repository.

## Standard

- NIST SP 800-162 (ABAC): every object receives its attributes when it is created, and a subject may not change the attributes that decide its own access.
- NIST SP 800-178 and NIST Next Generation Access Control (INCITS 565): grants as explicit assignments that can be reviewed per person and per object. Access must hold on every axis that applies.
- NIST RBAC (INCITS 359) and Kuhn, Coyne and Weil, "Adding Attributes to Role-Based Access Control" (IEEE Computer, 2010): role explosion comes from multiplying roles by scope. Put scope on the assignment, not in the role.
- NIST SP 800-53: AC-3 and AC-6 for enforcement and least privilege, AC-5 for separation of duties, AC-2 for account review, AU-3 for the content of an audit record, AU-9(3) for audit integrity.
- SCIM 2.0 (RFC 7643 §4.3): the enterprise extension's `manager` attribute is the source of the supervisory hierarchy.
- RFC 2606: the reserved top-level domain `.invalid` for synthetic people.
- W3C Organization Ontology, as `approvers.md` uses it: a post in an organization, held by a person.

Record the search in the first commit message of phase 2: NIST SP 800-162, SP 800-178, NGAC, INCITS 359, RFC 7643. No standard defines a multi-hierarchy grant model for an application platform. The shape here is the platform's own, with a published JSON schema for every structure.

## Decisions already made

| Question | Answer |
| --- | --- |
| How many org hierarchies? | As many as the institution needs. Each is a tree with its own root and one parent per unit. The existing tree becomes the `academic` hierarchy. Units never have two parents |
| How is a hierarchy added? | As data. A row in `hierarchies`, confirmed by a second person. No code change, no migration, no deploy. No hierarchy name appears in code or in a Cedar policy |
| What kinds of hierarchy are there? | `units` and `people`. In a `units` hierarchy, a row is placed by a field that names a unit. In a `people` hierarchy, such as supervision, a row is placed by a field that names a person, through that person's appointments. Behavior depends on the kind, never on the name |
| What coordinates does an object have? | At most one unit in each hierarchy, one domain, and one sensitivity. A workspace's unit in `academic` is required. The root of each axis is the default, so nothing is untagged |
| Can a child object widen its container's coordinates? | No. An app, table or field may only narrow what its container has: a deeper unit, a narrower domain, a higher sensitivity |
| Is a role a tag on an object? | No. A role is the verb on a grant: `viewer`, `commenter`, `editor`, `admin`, `owner`, as today. An object says where it sits. A grant says what its holder may do there |
| What is a grant? | One row: holder, role, hierarchy, anchor, depth, domain, clearance, validity window, source, reason. Holders are a person, a position, a unit, or a group |
| What anchors are there? | `container`: a workspace, app or table, and everything inside it. `unit`: every object placed in that unit's subtree, to a depth. `holder_unit`: wherever the holder holds the named position, so one row serves every department |
| Do grants permit? | Yes. Grants permit. Guards forbid. Rules narrow. A rule still never grants, as `access-rules.md` decided |
| How do grants combine? | The highest role among the grants that reach the object. Within one grant every axis must match |
| Can an object stop inheritance? | A workspace, app or table can be **sealed**. Grants anchored above it stop at it. Only grants anchored at the sealed object itself reach inside. Guards still apply. Rows cannot be sealed. Row exceptions are row rules |
| Who may create grants? | A workspace owner or admin: container grants on their own workspace and what is inside it, as today. An Org Unit Admin: unit grants inside their own subtree, never above their own role. A Platform Admin or `compliance` caller: any grant. Every grant change is a ledger entry. Every widening change needs a second person |
| What is an Org Unit Admin? | A built-in grant derived from the appointment: holder the person, role `admin`, anchor `unit` at their unit, whole subtree. It is evaluated like any other grant, so a guard can forbid it |
| Does a vacancy pass access up the tree? | No. There is no "unassigned subordinates" mode. A grant reaches what its own anchor reaches. The approver `walk_up` in `approvers.md` is a routing rule for decisions, not a path to data |
| Is a decision logged? | Changes are ledger entries, as today. Decisions that matter leave a receipt: every denial, every privileged action, every read that returns a protected field, and everything done under view-as or impersonation |
| Who may impersonate? | Full impersonation, with writes, only of people marked synthetic. A real person can be viewed only through view-as, which is read-only, for one app, for 30 minutes. Johann may change this |
| Is the tree stored on records? | No. A record stores only the unit id or person it names. The tree's ancestors live in one closure table. A reorganization rewrites closure rows and never touches records |

## How people meet this model

The model has many parts. Almost nobody should see them. `ux-standards.md` rules 11 to 16 apply to every screen and tool in this brief.

| Person | What they see | What the system and their AI tool handle |
| --- | --- | --- |
| A faculty member | Their apps. A denial with `Request access`, already filled in. Masked values where they are not cleared, such as a birth year | Which grants reach them, why, and how long. The request's route |
| An approver | One request or one drafted change at a time, as a sentence, with who gains and who loses, and `Approve` or `Reject` | Whether the request fits a rule that approves it alone. Who else could decide |
| An administrator | Suggestions such as "These 40 memberships are covered by one rule for chairs", drafts their AI tool made, and the impact of each | The structure: anchors, depths, domains, clearances, the closure |
| An AI tool | Tools that draft, preview and explain, and that answer in sentences | Turning what a person asked for into a structured draft that the server can check |

## What exists today

| Fact | Where |
| --- | --- |
| `member_role` turns a workspace's memberships into a person's workspace-wide role. `member_role_in_app` adds memberships limited to one app, from `workspace_collaborators.app_slug` (migration `0055_member_app_scope.sql`, `position-roles.md` phase 2a). Memberships can name a person, a group, a unit with its descendants, or a position at one unit | `service/members.rs` |
| Platform admin comes only from a held `platform_admin` role on the root unit. The setup identity stops working once the first admin is appointed. `GET /admin/direct-grants` lists every grant that names a person | `position-roles.md` phases 1 and 3, `service/identity.rs` |
| Cedar decides workspace and app actions from 25 policies compiled into the binary, plus forbid-only workspace guards. Every entity has `"parents": []`. A record is one entity per app, never per row | `service/access.rs` `decide`, `scaffoldry-policy/src/lib.rs`, `entities.rs` |
| Policies 1, 3, 9, 14 and 16 compare a free-text `department` string from SCIM. Guards and org scope use the unit tree | `scaffoldry-policy/src/lib.rs`, `service/identity.rs` |
| Cedar sees a person's units with all their ancestors. Row and column rules see only their direct units | `entities.rs` `unit_ids_for`, `service/scope.rs` `person_of` |
| Four separate code paths walk the tree in memory, capped at 32 or 64 steps: `unit_ids_for`, `unit_in_scope`, `members::within`, and the approver's `walk_up`. `decide` rebuilds a map of every unit and scans every role row on each call | `entities.rs`, `service/organizations.rs`, `service/members.rs`, `scaffoldry-core/src/approver.rs`, `service/access.rs` |
| **Row and column rules are lost on restart.** They live in two in-memory maps that start empty and are written only when a proposal is approved. With no rules, `scope_for` returns `AllowAll`. After a restart, or on a second instance, every rule-protected app shows every row and field | `state.rs` `access_rules`, `column_rules`; `service/proposals.rs`; `service/scope.rs` `scope_for` |
| **Twelve call sites skip guards.** They call the old `authorize_*` wrappers, which load no guards and pass empty unit ids: six in `service/workspaces.rs` (list, get, update, add, update and remove a collaborator), `routes/apps.rs` (`manage_app`), `routes/datasets.rs` (`publish_dataset`), `routes/governance.rs` and `service/tools.rs` (`record_decision`), `routes/policy.rs` and `service/tools.rs` (simulation). The guards brief's check missed them because each call is split across lines | as listed |
| **A guard's deny can be overridden.** When Cedar denies `read_app` or `manage_app`, `authorize_app` still allows an Org Unit Admin through `unit_in_scope` | `service/access.rs` `authorize_app` |
| **The sensitive-data guard never fires.** Production `read_app` is evaluated on a Workspace entity, which has no sensitivity attribute. A guard that fails to compile is skipped with `if let Ok` and `let _ =` | `service/access.rs`, `scaffoldry-core/src/guards.rs` |
| SCIM can create `platform_admin` and `unit_admin` appointments. PR 104 made `compliance` appointment-only and left these two | `routes/scim.rs` |
| Impersonation gives any `central_admin` full write rights as the target for one hour. Writes are recorded as the target's own. Only the ledger's start and end entries show it | `routes/auth.rs`, `guard.rs`, `service/history.rs` |
| When a person has no directory record, name, affiliation and department are guessed from substrings of their address | `service/identity.rs` `default_persona_attributes` |
| Only workspaces carry an org unit. Apps carry free-text `organization_code` and `department`. `herm_capability_id`, workspace `data_classification` and `allowed_affiliations` are stored and read by no access decision | `state.rs`, `scaffoldry-engine/src/lib.rs`, `service/workspaces.rs` |
| Fields carry a sensitivity flag and categories. A `viewer` loses every protected field. Other roles see them unless a column rule hides them | `service/records.rs` `hidden_fields` |
| Row rules compile to SQL over `record_index` for lists and run in Rust for one record. A 2,000-case test holds the two equal | `scaffoldry-engine/src/access.rs`, `tests/access_scope_test.rs` |
| `record_index` has B-tree indexes on `(app_slug, table_id, field, num|txt|ts, record_id)` | migration `0015_record_index.sql` |
| `roles.end_date` and `effective_date` exist. No server code reads them | `service/` |
| No access decision has ever been timed. The million-row test ran with no access rules | `tests/million_rows_test.rs` |
| The `cache_versions` counters refresh cached collections at a measured 0.46 to 0.57 ms per request | `stateless-server.md` phase 1 |
| A `Collaborator` field type stores addresses | `scaffoldry-engine/src/lib.rs` |

## Out of scope

- A separate authorization service, a relationship database, or a second source of truth. Grants are PostgreSQL rows.
- Cedar's experimental type-aware partial evaluation (`tpe`). The row filter is always "unit in a set", which the existing compiler handles.
- PostgreSQL row-level security. It is a sound backstop and a poor primary filter. A later brief may add it as defense in depth.
- A per-user or per-record table of who can see what. It turns every reorganization into a recalculation.
- Units with two parents, and hierarchies an end user can create.
- Delegation of data access. `approvers.md` phase 4 delegates decisions, not reach.
- Purpose-based access and break-glass access. A later brief can add a grant `source` for each.
- Machine-learning classification. Phase 10 turns existing detector findings into proposals.
- The policy text a grant enforces. `policy-library.md` binds a rule sentence to a grant rule in phase 7.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. `reach` and `role_at` are pure functions over loaded rows and a time. They do no I/O. Everything else calls them.
4. No hierarchy key, unit id, domain key, or position key is written in code or in a Cedar policy. A test scans the source and the policy text for the keys of every row in `hierarchies`.
5. A change that widens access needs a second person. A change that only narrows follows its normal path. Every change is a ledger entry written before the change, with a reason.
6. Nothing fails open. A read, a compile, or a load that fails is an error the caller sees. Never `let _ =` or `if let Ok` in front of anything that decides access.
7. Use the next free migration number on `main`. Name each file as this brief does.
8. Every phase that adds a tool or changes a decision adds scenarios to the agent suite in `agent-coverage.md`.
9. Every new error code goes on the closed list in `ux-standards.md` with a row in `explainError`.
10. Use the kit in `apps/web/src/ui/` for every screen.

## Phase 1 — close what fails open

No new model. Six defects, each with a test that fails today.

1. **Rules survive a restart.** The approved manifest is the source of an app's row and column rules. Load them from every live manifest when the server starts. Keep them in step with the `manifests` counter from `stateless-server.md`. A boot load that fails is an error. Add a test that an app whose stored manifest has rules can never evaluate to `AllowAll` because a map is empty. If a separate fix has already landed, verify it against these tests and say so.
2. **Every check goes through `decide`.** Replace the twelve direct `authorize_*` calls listed above. Delete the old wrappers from `scaffoldry-policy`, so a new call cannot compile. Replace the guards brief's grep with a test that parses the source and fails on any call to a removed wrapper, however it is split across lines.
3. **A forbid always wins.** Delete the `unit_in_scope` fallback in `authorize_app`. An Org Unit Admin's access becomes a Cedar `permit` for a principal whose `admin_unit_ids` contain the workspace's unit or an ancestor of it. A guard's forbid overrides it.
4. **Sensitivity reaches Cedar.** `read_app` and `export` on an app carry `is_ferpa_sensitive` and `categories` computed from the app's fields with `effective_ferpa_sensitive`. A guard that fails to compile fails the request with `internal`. It is never skipped.
5. **One set of units.** One function returns a person's units with their ancestors. Cedar and row and column rules both call it. A row rule `unit In person.unit_ids` matches a person in Physics when the value is the College of Science. Row rules also get each position as `key@unit`, beside the bare key, so a rule can tell Chemistry's chair from Physics's. This settles the two open points in `position-roles.md`.
6. **SCIM grants no admin role.** SCIM may no longer create `platform_admin` or `unit_admin` appointments, as PR 104 did for `compliance`. Existing rows with `source = scim` and one of those values are kept and listed in the pull request. A Platform Admin re-appoints or removes them. Do not delete them in a migration.

Tests.

1. Two instances on one database. An app with a row rule is approved through A. B hides the out-of-scope rows on its first request. A restarted A hides them too.
2. A workspace guard `deny_access_outside_units` hides the workspace from `GET /workspaces` and returns 403 on a direct get for a caller outside the units.
3. A guard that forbids `read_app` denies an Org Unit Admin of the workspace's unit.
4. `deny_sensitive_unless_affiliation` denies a student `read_app` on an app with a protected field, and allows staff.
5. A guard whose stored source no longer compiles makes the request fail. It does not widen access.
6. The same person and the same rule `unit In person.unit_ids` give the same answer in Cedar, in Rust and in SQL.
7. A SCIM user with `roles[type=scaffoldry, value=platform_admin]` gets no `platform_admin` row.
8. The source scan fails when a removed wrapper is called across three lines.

## Phase 2 — hierarchies as data, one closure, one notion of place

Migration `*_hierarchies.sql`:

```sql
CREATE TABLE IF NOT EXISTS hierarchies (
    key VARCHAR(32) PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    kind VARCHAR(8) NOT NULL,
    source VARCHAR(16) NOT NULL,
    status VARCHAR(8) NOT NULL DEFAULT 'Active',
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    retired_at TIMESTAMPTZ
);
INSERT INTO hierarchies (key, name, kind, source, created_by)
VALUES ('academic', 'Academic and administrative', 'units', 'api', 'system')
ON CONFLICT DO NOTHING;

ALTER TABLE organizations ADD COLUMN IF NOT EXISTS hierarchy_key VARCHAR(32) NOT NULL DEFAULT 'academic' REFERENCES hierarchies(key);
CREATE UNIQUE INDEX IF NOT EXISTS idx_one_root_per_hierarchy ON organizations (hierarchy_key) WHERE parent_id IS NULL;

CREATE TABLE IF NOT EXISTS org_closure (
    hierarchy_key VARCHAR(32) NOT NULL REFERENCES hierarchies(key),
    ancestor UUID NOT NULL REFERENCES organizations(id),
    descendant UUID NOT NULL REFERENCES organizations(id),
    depth SMALLINT NOT NULL,
    PRIMARY KEY (ancestor, descendant)
);
CREATE INDEX IF NOT EXISTS idx_org_closure_descendant ON org_closure (descendant, depth);
```

Closed lists: `kind` is `units` or `people`. `source` is `api`, `scim`, `import`, or `connection`. `status` is `Active` or `Retired`.

The seeded institution root becomes the root of `academic`. Platform Admin and `compliance` appointments stay on it.

1. A unit's parent must be in the same hierarchy. The service refuses another with `400`. A test that inserts across hierarchies directly in SQL shows the service check and a `CHECK` through a trigger both refuse it.
2. The migration fills `org_closure` from `parent_id`, including each unit's row to itself at depth 0.
3. Creating, moving and retiring a unit rewrites the closure rows of its subtree in the same transaction, under the row lock from `stateless-server.md` phase 3, and bumps the `organizations` counter.
4. The closure is loaded with the `organizations` cached collection. One module, `crates/scaffoldry-core/src/tree.rs`, answers `ancestors(unit)`, `descendants(unit, depth)` and `contains(ancestor, unit)` from it.
5. Replace all four walkers with that module: `unit_ids_for`, `unit_in_scope`, `members::within`, and the approver's `walk_up`. Delete the step caps. `decide` no longer rebuilds a map or scans role rows.
6. One notion of place. Cedar gets a `Unit` entity type with a `hierarchy` attribute. Unit entities carry their ancestors as parents. A workspace and an app carry their unit as a parent. Policies 1, 3, 9, 14 and 16 test whether the resource's unit is in one of the principal's units, with Cedar's `in`, instead of comparing the `department` string. Show each rewritten policy and its old text in the pull request. The `department` string stays for display and is read by no policy.
7. Routes: `GET /admin/hierarchies` and `GET /admin/hierarchies/{key}/tree` for a Platform Admin or `compliance`. Creating a hierarchy is phase 11.

Tests.

1. For a generated tree of 2,000 units, the closure equals a walk of `parent_id` for every pair.
2. Moving a subtree of 500 units rewrites only that subtree's closure rows and no record. Count both. Record the time.
3. A cycle through a move is refused and the closure is unchanged.
4. A second root in `academic` is refused by the unique index.
5. A grep of the server for the four old walker names prints nothing.
6. Each of the five rewritten policies allows a person in a descendant unit and denies a person outside, and every case in `cedar_authorization_test.rs` still passes.
7. The source scan from rule 4 passes.

## Phase 3 — every object has coordinates

Migration `*_coordinates.sql`:

```sql
CREATE TABLE IF NOT EXISTS access_domains (
    key VARCHAR(64) PRIMARY KEY,
    parent_key VARCHAR(64) REFERENCES access_domains(key),
    name VARCHAR(255) NOT NULL,
    status VARCHAR(8) NOT NULL DEFAULT 'Active'
);
CREATE TABLE IF NOT EXISTS workspace_coordinates (
    workspace_id VARCHAR(64) NOT NULL REFERENCES workspaces(id),
    axis VARCHAR(48) NOT NULL,
    value VARCHAR(64) NOT NULL,
    PRIMARY KEY (workspace_id, axis)
);
```

`axis` is `unit:<hierarchy key>`, `domain`, or `sensitivity`. A workspace's `academic` unit stays in `workspaces.organization_id`. Seed `access_domains` with one root, `institution`, and one row per HERM capability in `crates/scaffoldry-core/src/standards`, under its parent capability. The sensitivity ladder is `Public`, `Directory`, `Internal`, `Restricted`. Categories are the existing sensitivity categories.

Apps and tables carry coordinates in the manifest, so they change only by proposal:

```json
"coordinates": { "units": { "<hierarchy>": "<unit id>" }, "domain": "<key>", "sensitivity": "Internal" }
```

on the app and on each table. A field keeps its `categories` and gains an optional `sensitivity`.

1. `effective_coordinates(workspace, app, table, field)` is a pure function. Each level takes its container's value unless it narrows it: a unit that is the container's or below it, a domain that is the container's or below it, a sensitivity that is the same or higher. A missing value is the container's. The workspace's missing values are each axis's root and `Internal`.
2. `validate_manifest` refuses a coordinate that would widen its container's, with the field path and the container's value.
3. A table may designate one field per hierarchy that places its rows: `"row_place": { "<hierarchy>": "<field name>" }`. For a `units` hierarchy the field has the new type `Unit { hierarchy }`, whose value is validated against that hierarchy and indexed in `record_index` as text. For a `people` hierarchy it is a `Collaborator` field. Designating or changing it is a manifest change.
4. Cedar: App and Workspace entities carry their unit in each hierarchy and their domain as parents, and their sensitivity as an attribute. Add a guard template `deny_outside_domain` and one `deny_above_sensitivity`, both forbid-only. Coordinates are now read by decisions, which keeps the rule that no stored field goes unevaluated.
5. `allowed_affiliations` on a workspace becomes a `deny_write_for_affiliation`-style guard on migration, or is removed. Say which in the pull request. Nothing stored may stay unread.
6. Proposal checks: a coordinate that is lowered or removed raises the flagged check `coordinates_widened`. Add it to `FLAGGED_CHECKS` with its sentence.
7. Workspace coordinates change through `PUT /workspaces/{id}/coordinates` by an owner, admin, Org Unit Admin in scope, or Platform Admin. A widening change is stored as pending and confirmed by a different person, as `authority.md` does for matrix changes.

Tests.

1. `effective_coordinates` table test: inherit, narrow, and refuse to widen on each axis.
2. A manifest whose table sits in a unit above its app's is refused and names both units.
3. A `Unit` field refuses a unit from another hierarchy, and its value appears in `record_index`.
4. A guard `deny_outside_domain` forbids `read_app` on an app in another domain.
5. Lowering a table's sensitivity raises `coordinates_widened` and needs a second person.
6. A workspace's widened coordinate is not in force until a different person confirms it.

## Phase 4 — one grant shape, and `role_at`

Migration `*_access_grants.sql`:

```sql
CREATE TABLE IF NOT EXISTS access_grants (
    id BIGSERIAL PRIMARY KEY,
    holder_kind VARCHAR(8) NOT NULL,
    holder_id VARCHAR(255) NOT NULL,
    role VARCHAR(16) NOT NULL,
    hierarchy_key VARCHAR(32) REFERENCES hierarchies(key),
    anchor_kind VARCHAR(16) NOT NULL,
    anchor_id VARCHAR(255),
    depth SMALLINT,
    domain VARCHAR(64) REFERENCES access_domains(key),
    clearance VARCHAR(16),
    valid_from TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    valid_until TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    revoked_by VARCHAR(255),
    source VARCHAR(16) NOT NULL,
    reason TEXT NOT NULL,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_access_grants_holder ON access_grants (holder_kind, holder_id) WHERE revoked_at IS NULL;

CREATE TABLE IF NOT EXISTS access_changes (
    id UUID PRIMARY KEY,
    change JSONB NOT NULL,
    widens BOOLEAN NOT NULL,
    reason TEXT NOT NULL,
    status VARCHAR(10) NOT NULL DEFAULT 'Pending',
    proposed_by VARCHAR(255) NOT NULL,
    proposed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    decided_by VARCHAR(255),
    decided_at TIMESTAMPTZ
);
```

Closed lists. `holder_kind`: `person`, `position`, `unit`, `group`. `holder_id`: an address, `key@<unit id>`, `key@*` for every holder of a position, a unit id, or a group id. `anchor_kind`: `container`, `unit`, `holder_unit`. `depth`: 0 for the anchor only, null for the whole subtree. `clearance`: a sensitivity, or null for the role's default. `source`: `membership`, `appointment`, `rule`, `request`, `task`, `flow`. `access_changes.status`: `Pending`, `Confirmed`, `Rejected`, `Withdrawn`. Add `governance/schema/access-grant.schema.json`.

Grants are not cached. They are read on each request by holder, with the index above. Add a cache only if phase 6 measures a need.

The pure functions, in `crates/scaffoldry-core/src/reach.rs`:

1. `holder_set(person, roles, groups, now)`: the person's address, each `position@unit` they hold, `position@*` for each position key they hold, each unit where they hold any role with its ancestors, and their groups. Roles outside `effective_date` and `end_date` are not counted.
2. `reaches(grant, object, tree, now) -> bool`. A `container` grant reaches the container and everything inside it. A `unit` grant reaches an object whose unit in the grant's hierarchy is in the anchor's subtree, within `depth`. A `holder_unit` grant on a position reaches as a `unit` grant anchored at each unit where the holder holds that position. Every grant also requires the object's domain to be in the grant's domain subtree when the grant names one. An object with no unit in the grant's hierarchy is not reached. A sealed container (phase 7) stops every grant anchored above it.
3. `role_at(holders, grants, object, tree, now) -> Option<(Role, Vec<GrantId>)>`: the highest role among the grants that reach the object, and the ids of those grants.

Wire it in.

1. `member_role` and `member_role_in_app` both become calls to `role_at`. Every caller keeps its shape: the workspace-wide question asks about the workspace, the app question asks about the app. Workspace collaborator rows are read as `container` grants by the same function, on the workspace when `app_slug` is empty and on that app otherwise, with `source = membership`. Do not move them into `access_grants` in this phase. Workspace-level actions still ignore grants anchored on an app, as `position-roles.md` phase 2a decided.
2. A `unit_admin` appointment is a grant derived when the holder set is built: holder the person, role `admin`, anchor `unit` at the appointment's unit, whole subtree, `source = appointment`. It is not stored. Remove the Cedar `permit` added in phase 1 and its `admin_unit_ids`.
3. Policies 22 to 25 read the role from `role_at`. A Platform Admin's access is unchanged.
4. Routes, each with `reason`:

| Method | Path | Who | Behavior |
| --- | --- | --- | --- |
| GET | `/admin/access/grants` | Platform Admin, `compliance`, Org Unit Admin for their subtree | Query `holder`, `anchor`, `hierarchy`, `source`, `state`, cursor |
| POST | `/admin/access/grants` | as in "Decisions already made" | Validates against the schema. An Org Unit Admin may anchor only inside their subtree and grant at most `admin`: otherwise `403 grant_outside_scope`. Every new grant widens, so it is stored as a pending `access_changes` row. A grant whose holder is the root unit of a hierarchy, or a group of everyone, is refused with `400 grant_too_broad` unless its anchor is one container and its role is `viewer`. Access for everyone is a workspace's institutional visibility, which already exists |
| POST | `/admin/access/grants/{id}/revoke` | the same | Narrows. Applied at once. Sets `revoked_at` and `revoked_by`. Grants are never deleted |
| POST | `/admin/access/changes/{id}/decide` | a different person with the same right | `confirm` or `reject`. The proposer gets `403`. On confirm, in one transaction: ledger `AccessGrantChanged`, then the row |

Add `AccessGrantChanged`, `CoordinatesChanged` and `HierarchyChanged` to `DecisionType::ALL`.

Tests.

1. With no rows in `access_grants`, every existing test passes unchanged. Paste the count before and after.
2. A `holder_unit` grant "every `chair@*` gets `viewer` on their own unit's subtree" gives the Physics chair viewer on a Physics workspace and nothing on Chemistry. Appointing a new chair moves the access with no other change.
3. A `unit` grant with depth 1 reaches a department and not a lab below it.
4. A grant with domain `research` does not reach an app in `finance`.
5. An expired or revoked grant reaches nothing. A role row past its `end_date` is not in the holder set.
6. An Org Unit Admin cannot anchor a grant above their unit or grant `owner`.
7. A grant is not in force until a different person confirms it. The proposer's confirm is `403`.
8. `role_at` returns the ids of the grants that gave the role.
9. A workspace guard forbids `read_app` to a person who holds a matching unit grant.

## Phase 5 — a synthetic institution, and a reference to test against

1. A command, `scaffoldry-server synthesize --seed <n> --people <n> --units <n> --hierarchies academic,supervisory --apps <n> --records <n>`. It builds a synthetic institution in its own subtree of each hierarchy, from a fixed seed, with a small generator written in the code. Do not add a random-number crate. Faculties, schools and departments with random fanout. People with skewed appointments and positions. Workspaces, apps and records whose units are skewed so a few departments hold most rows. Grants of every anchor kind.
2. Migration `*_synthetic.sql` adds `synthetic BOOLEAN NOT NULL DEFAULT FALSE` to `scim_users`, `persons`, `organizations` and `workspaces`, and a trigger that refuses changing it from true to false. Synthetic people have addresses in the reserved domain `synthetic.invalid`.
3. A synthetic unit may only sit under a synthetic unit. A grant held by a synthetic person may only anchor inside the synthetic subtree. The service refuses both with `400`.
4. `crates/scaffoldry-core/src/reach_reference.rs`: a reference evaluator that walks `parent_id` upward from an object's unit and checks each grant directly, with no closure and no index. It is slow and obviously correct. It is used only by tests.
5. A differential test, `tests/reach_differential_test.rs`. Over 20 generated worlds, for every synthetic person and every object, the reference and `role_at` agree. For lists, the row ids from SQL and from the reference agree. Then it checks these properties:
   - Monotonic: adding a grant never removes a row.
   - Move safety: after a subtree moves, nobody keeps access through the old ancestor.
   - Cache transparency: the answer is the same with and without the cached collections.
   - View-as bound: what someone sees as another person is in both people's reach.

Tests.

1. The same seed produces the same institution twice. Compare a hash of every row it wrote.
2. Changing `synthetic` from true to false is refused by the database.
3. A synthetic person's grant anchored in a real unit is refused.
4. The differential test passes on 20 worlds of 2,000 people and 200 units each, in under 60 seconds. Record the time.

## Phase 6 — lists and fields at a million rows

1. **Row filter.** For a table with a designated `row_place` field, the caller's reach in that hierarchy becomes a set of unit ids, and the existing compiler adds one more set to its plan: record ids whose field has a `txt` value in that set, served by the existing text index. Pass the set as one array parameter, `= ANY($n)`. A reach that covers the whole hierarchy adds no predicate. A `container` grant on the table gives `AllowAll` for reach.
2. **People hierarchies.** For a `people` hierarchy, the reach is the set of addresses of people with an active appointment at a unit in the caller's reach. The filter is the same shape over the `Collaborator` field.
3. **Several grants.** Rows reached by any grant are the union. Each grant's own domain and depth apply to its part. Row rules then narrow the union, as today.
4. **Fields.** Each role has a default clearance that reproduces today's behavior. A `viewer` is cleared for nothing protected. `editor` and above are cleared for everything. A grant may set a lower clearance. Setting it above the role's default widens and needs a second person. In a list, a field is shown only if every grant that contributes rows clears it.
5. **When the set is large.** If a reach has more than a threshold of unit ids, the filter becomes a subquery on `org_closure`. Find the threshold by measurement in this phase. Put it in a constant with a `ponytail:` comment naming how it was measured.

The gate. Extend `tests/million_rows_test.rs`, or add a test beside it named `million_rows_with_grants`:

| Operation | Limit for the slowest of twenty |
| --- | --- |
| First page, a reach of 500 units in one hierarchy | 100 ms |
| First page, reaches in three hierarchies at once | 150 ms |
| First page, a `people` reach of 5,000 addresses | 200 ms |
| A page near the end by cursor, a reach of 500 units | 100 ms |
| Grouped count, a reach of 500 units | 1,500 ms |

And `tests/decide_benchmark_test.rs` on a synthetic institution of 50,000 people, 400 units and three hierarchies: `role_at` plus Cedar for one object, median under 1 ms and slowest of 1,000 under 10 ms.

If a limit fails and the query plan is correct, do not raise it. Paste the plan and the timings, and stop. The decision is Johann's. Write the results to the pull request and as a CI artifact.

Tests.

1. The gate above.
2. The differential test from phase 5 passes with row filters on.
3. A person cleared for FERPA in Biology and not in Chemistry sees the field on a Biology-only list and not on a list that also contains Chemistry rows.
4. Filtering or sorting on a field that a contributing grant does not clear is `field_hidden`.

## Phase 7 — sealing, the explanation, and review both ways

1. **Sealing.** A workspace, app or table may be sealed. Store it as an axis `sealed` in `workspace_coordinates`, and as `"sealed": true` on an app or table in the manifest. `reaches` stops every grant anchored above a sealed object. Grants anchored at the sealed object itself, and grants anchored inside it, still reach. Sealing narrows and follows the normal path. Unsealing widens and needs a second person. A row cannot be sealed.
2. **The explanation.** `explain(caller, person, object)` returns: the person's role on the object, each grant that reached it with its id, holder, anchor, and the closure path from the anchor to the object's unit, each grant that did not reach it and the first axis that failed, the Cedar policies that allowed and forbade the action, each guard and rule that narrowed it, and any sealed container that stopped a grant. `GET /access/explain?person=&object=` for a Platform Admin, `compliance`, an Org Unit Admin in scope, or the person themselves. The read-only MCP tool `explain_access` calls it for the caller's own access only.
3. **Review per person.** `GET /admin/access/people/{eppn}`: their holder set, every grant that names them or a holder they belong to, and the objects each reaches, as counts per hierarchy.
4. **Review per object.** `GET /admin/access/objects/{kind}/{id}`: every grant whose reach contains it, from one join of `org_closure` with `access_grants`, plus memberships.
5. **Impact before a change.** For a pending grant, coordinate, hierarchy or seal change, `POST /admin/access/changes/{id}/impact` returns who gains and who loses, per role, computed with `role_at` before and after over the affected people. It changes nothing.
6. When `policy-library.md` phase 4 has run, a `rule`-source grant can be bound to a library rule, and the library shows it as Enforced.

Tests.

1. A sealed table in a department workspace hides its rows from a dean's college grant and from the workspace's editors. A grant anchored at the table still reaches.
2. Unsealing is not in force until a different person confirms it.
3. `explain` for a denied person names the axis that failed. For an allowed person it names the grant id and the closure path.
4. The per-object review lists a college grant on a department's table. The per-person review counts it.
5. The impact of a pending grant lists exactly the people who gain, checked against `role_at` run by hand in the test.
6. `explain_access` refuses to explain another person to an agent token.

## Phase 8 — decision receipts

Migration `*_access_receipts.sql`:

```sql
CREATE TABLE IF NOT EXISTS access_receipts (
    id BIGSERIAL PRIMARY KEY,
    at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    actor VARCHAR(255) NOT NULL,
    acting_as VARCHAR(255),
    action VARCHAR(48) NOT NULL,
    object_kind VARCHAR(16) NOT NULL,
    object_id VARCHAR(255) NOT NULL,
    outcome VARCHAR(8) NOT NULL,
    grant_ids BIGINT[] NOT NULL DEFAULT '{}',
    policy_ids TEXT[] NOT NULL DEFAULT '{}',
    narrowed_by TEXT[] NOT NULL DEFAULT '{}',
    counters JSONB NOT NULL,
    batch_id BIGINT
);
CREATE INDEX IF NOT EXISTS idx_access_receipts_object ON access_receipts (object_kind, object_id, at);
CREATE INDEX IF NOT EXISTS idx_access_receipts_actor ON access_receipts (actor, at);

CREATE TABLE IF NOT EXISTS access_receipt_batches (
    id BIGSERIAL PRIMARY KEY,
    first_id BIGINT NOT NULL,
    last_id BIGINT NOT NULL,
    hash CHAR(64) NOT NULL,
    previous_hash CHAR(64) NOT NULL,
    ledger_sequence BIGINT
);
```

1. A receipt is written for: every denial; every privileged action (every `/admin` write, impersonation, view-as start and every request under it, grant and coordinate decisions); and every read that returns a protected field, one per call, not per row. Nothing else.
2. A receipt holds ids, never record values. `counters` is the `cache_versions` tuple in force, so the decision can be replayed. `acting_as` is set under view-as and impersonation.
3. A scheduled job seals batches every minute: it hashes the receipts since the last batch with the previous batch's hash, stores the batch, and appends one ledger entry `AccessReceiptsSealed` holding the batch hash. A gap or an edit in a sealed batch fails verification.
4. The disclosure log stays. Each disclosure also writes its receipt. Do not merge the two tables in this phase.
5. `GET /admin/access/receipts` for a Platform Admin or `compliance`, filtered by actor, object, outcome and time, with a cursor. Retention follows `lifecycle.md` when its retention phase has run.

Tests.

1. A denial writes one receipt with the policy id. An allowed read of an unprotected field writes none.
2. A list call that returns 200 rows with a protected field writes one receipt.
3. Editing a receipt in a sealed batch makes verification fail and name the batch.
4. Replaying a receipt with its counters reproduces the outcome.
5. Measure the added time of one receipt write. Record the median of 1,000. If it is above 1 ms, stop and report.

## Phase 9 — impersonation of synthetic people only

1. `POST /auth/impersonate` is refused with `403 impersonation_not_synthetic` unless the target is synthetic. The refusal's remedy names view-as.
2. Every write's history origin gains `acted_by`. Under impersonation it holds the impersonator and the target is the actor. Under view-as, which cannot write, it is unused. `routes/auth.rs`, `service/history.rs` and every writer that builds an origin set it.
3. Delete `default_persona_attributes` and every guess from an address. A person with no directory record resolves with an empty department and the affiliation `member`. Move every test that relied on a guessed persona to people from the generator in phase 5, or to explicit SCIM fixtures. Paste the count of tests moved.
4. The seeded demo command, if kept, builds its people with the generator and marks them synthetic.

Tests.

1. Impersonating a real person is `403 impersonation_not_synthetic`. Impersonating a synthetic person works, and a write made under it records both addresses.
2. View-as of a real person still works and is still read-only.
3. A grep for `default_persona_attributes` and for the old persona names in `crates/scaffoldry-server/src` prints nothing.
4. An address with "curie" in it and no directory record is `member` with no department.

## Phase 10 — provisioning from the systems of record

1. **Supervision as a hierarchy.** Read the SCIM enterprise `manager` attribute. When a Platform Admin has created a `people` hierarchy with `source = scim` (phase 11), the SCIM sync keeps it as a tree of supervisory units: one unit per manager, its parent the unit of that manager's manager, and each person appointed at their manager's unit. The manager holds the position `supervisor` there. A person with no manager hangs from the hierarchy's root. Changes are applied in the sync's transaction and bump the `organizations` counter.
2. **Validity.** A grant with `source = request` needs a `valid_until`, at most the setting `access.person_grant_max_days` (integer 1 to 365, default 180). Add the key to the closed settings list.
3. **Review.** A grant older than the setting `access.review_days` (integer 30 to 730, default 365) and not reviewed since is due. `GET /admin/access/reviews` lists due grants. `POST /admin/access/grants/{id}/review` with `keep` or `revoke` and a reason. Keeping writes a ledger entry and resets the clock. A person cannot review a grant they hold.
4. **Separation of duties.** The setting `access.conflicts` holds pairs of holders that one person may not hold at once, for example two positions, or a position and a role on a domain. Creating a grant or an appointment that completes a pair is refused with `409 grant_conflict` naming the pair. Existing conflicts are listed in the review.
5. **Classification.** When `sensitive-content.md` phase 2 has run, a confirmed finding in a field whose sensitivity is lower than its category requires creates a proposal to raise it. A detector never changes a coordinate itself.

Tests.

1. A SCIM user whose manager changes moves to the new manager's unit, and the old manager's `holder_unit` grant stops reaching their records.
2. A requested grant without `valid_until`, or beyond the maximum, is `400`.
3. A grant past `access.review_days` appears in the review list. Keeping it removes it and writes a ledger entry.
4. Completing a conflicting pair is `409 grant_conflict`.
5. A finding on a field produces a proposal and does not change the field.

## Phase 11 — the Access section of the console

Add an **Access** section to the console, built from the kit. It is listed in `admin-console.md` under "Panels added by other briefs".

- **Hierarchies.** The list, each tree, and `New hierarchy`: key, name, kind, source, reason. Creating one is a pending change confirmed by a different person. `Retire` with a reason: its grants stop reaching, and its units and coordinates stay for audit. A hierarchy is never deleted.
- **Grants.** A table filtered by holder, anchor, hierarchy, source and state. `New grant` builds one row in a form that shows, before it is proposed, the sentence it means ("Every department chair can view everything placed in their own department, in Research, up to Internal") and its impact from phase 7. `Revoke`, `Confirm` and `Reject` as the routes allow.
- **Objects.** Search a workspace, app or table. Show its coordinates, whether it is sealed, and the per-object review.
- **People.** Search a person. Show their holder set and the per-person review. `Explain` takes an object and shows the phase 7 answer as sentences with links.
- **Reviews.** Due grants and conflicts, with `Keep` and `Revoke`.
- **Receipts.** The phase 8 list.

Tests.

1. A hierarchy created by one Platform Admin is not usable until another confirms it.
2. The grant form shows the sentence and the impact before `Propose` is enabled, and disables it again when a value changes.
3. `Explain` renders each grant as a sentence with a link to the grant and a path through named units, not ids.
4. An Org Unit Admin sees only grants anchored in their subtree.
5. Every list in the section pages in SQL. With 10,000 grants, each page returns in under 200 ms. Record the times.

## Phase 12 — masks instead of blanks

Today a person who is not cleared for a field gets nothing. Often a safe part would do: the birth year without the date, the last four digits of an identifier, a salary band.

1. A closed list of masks in `crates/scaffoldry-core/src/mask.rs`, one pure function `mask(value, kind) -> Value`: `hide` (today's behavior), `last4` (all but the last four characters replaced), `initials`, `year` and `month` (a date reduced to its year, or its year and month), and `band` (a number reduced to its band, with the band width stored in the mask). Add a mask schema to `governance/schema/`. A keyed hash for joining two masked tables needs the secrets store and is out of scope.
2. Each sensitivity category gains `default_mask`, in its settings and in each file in `governance/sensitivity-presets/`. FERPA and PHI default to `hide`. The others are chosen in the preset and shown in the pull request.
3. A field may name a `mask`. A column rule may use `mode: Masked { mask }` beside `Hidden` and `ReadOnly`. A mask that shows more than the field's category default is a widening change and raises the flagged check `mask_weakened`.
4. When a reader is not cleared for a field, its value is replaced by its mask in every response: records, pages, `page_call`, client calls, exports and history. One function applies it, in the same place `redact_records` removes hidden fields today. Values are masked after the row scope chooses rows, so the scope and the SQL compiler do not change.
5. Filtering, sorting, searching or totalling a masked field is refused with `field_hidden`, as for a hidden field. A mask must not leak order.
6. A masked read is not a disclosure of the protected value. Only a read with no mask is.
7. Every mask has a sentence: "Shown as the year only."

Tests.

1. Each mask kind on a table of values, including empty values, short strings for `last4`, and dates at the edges of a year.
2. A viewer reads a FERPA date of birth whose field mask is `year` and gets the year. An editor gets the date.
3. Sorting on a masked field is `field_hidden`.
4. Setting a field's mask to show more than its category default raises `mask_weakened`.
5. An export and a page call apply the same mask as a list.

## Phase 13 — asking for access

A person who is refused should be able to ask, and most routine asks should not need anyone.

Migration `*_access_requests.sql`:

```sql
CREATE TABLE IF NOT EXISTS access_requests (
    id UUID PRIMARY KEY,
    requester VARCHAR(255) NOT NULL,
    filed_by_client VARCHAR(64),
    task VARCHAR(200),
    object_kind VARCHAR(16) NOT NULL,
    object_id VARCHAR(255) NOT NULL,
    role VARCHAR(16) NOT NULL,
    clearance VARCHAR(16),
    reason TEXT NOT NULL,
    days SMALLINT NOT NULL,
    route VARCHAR(10) NOT NULL,
    rule_id UUID,
    status VARCHAR(16) NOT NULL,
    decided_by VARCHAR(255),
    decided_at TIMESTAMPTZ,
    grant_id BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS access_request_rules (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id),
    rule JSONB NOT NULL,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    retired_at TIMESTAMPTZ
);
```

`route` is `auto`, `self`, or `approver`. `status` is `Waiting for person`, `Pending`, `Approved`, `Rejected`, `Withdrawn`, or `Expired`.

1. **Rules that approve alone.** A rule is a closed set of conditions, all of which must hold: the highest role it allows, the highest sensitivity of the object, the workspace visibilities it covers (a rule can never cover `restricted`), whether the object must be in the requester's own unit or below it, the longest duration, and optionally affiliations. Rules are set like the authority matrix: the institution sets them, a unit may only add conditions, and a second person confirms each change. A request that fits a rule is approved at once with `route = auto`, and the grant records the rule.
2. **The default rule**, created at migration and shown in the console: "A viewer request for an app at Internal or below, in the requester's own unit or below, in a workspace whose visibility is departmental or institutional, for up to 30 days, is approved automatically." **A request for an app in a restricted workspace is never approved automatically. It always routes to that workspace's owner.** An institution can retire the rule. Settled 2026-10-18.
3. **Within the person's own rights.** When an AI tool working with a narrowed session asks for something the person may already do, `route = self`. The person confirms in the desk, or in the AI tool when it supports MCP elicitation. The approval widens only that session, until it ends. No grant is stored.
4. **Everything else goes to a person.** The request becomes a change with signals `request_write`, `request_sensitive`, and `request_outside_unit` as they apply. `authority::required` names who decides. Add the three to the sentence table beside `FLAGGED_CHECKS`. The requester is never the approver.
5. **Approval makes a grant** with `source = request`, the requested role and clearance, and `valid_until` after `days`, at most the setting `access.request_max_days` (integer 1 to 365, default 90). It expires by itself.
6. **Where requests start.** Every denial from `decide` and every `field_hidden` carries a `request` object filled with what was refused, and the desk shows `Request access` from it. The MCP tool `request_access` files one for the signed-in person. When an AI tool files it, the status starts at `Waiting for person` until the person confirms, so a tool can never ask in a person's name without them.
7. When `workflows.md` phase 5 has run, a pending request is a My Work item of kind `decision` for each approver. When `notifications.md` phase 1 has run, the requester is told the outcome.
8. Routes: `POST /access/requests`, `GET /me/access/requests`, `POST /access/requests/{id}/confirm` for the person, `POST /access/requests/{id}/decide` for an approver, `POST /access/requests/{id}/withdraw`. Admin routes for rules, with reasons and the ledger.

Tests.

1. A viewer request inside the requester's own unit, for an Internal app, for 14 days, is approved at once by the default rule, and the grant expires after 14 days.
2. An editor request is not approved by the default rule and is routed to the people `authority::required` names.
2a. A viewer request for an Internal app in a restricted workspace, in the requester's own unit, for 14 days, is not approved by the default rule and is routed to the workspace's owner. A rule that names `restricted` is refused when it is saved.
3. A request filed by an AI tool waits for the person, and an approver cannot decide it before then.
4. A `self` request widens only the session that asked, and a new session does not have it.
5. The requester cannot approve their own request, even as an owner.
6. A denial carries a filled `request` object that, sent back unchanged, files the request.
7. A unit rule that removes a condition from the institution's rule is refused.

## Phase 14 — AI tools draft, people decide

Most of this brief's structure should be written by an administrator's AI tool, checked by the server, and confirmed by a person.

1. **Draft tools.** `draft_access_change` takes one change of a kind this brief defines: a grant, a revocation, coordinates, a seal, a mask, a request rule, or a hierarchy. The caller must have the right to propose that change in the console. It stores a pending `access_changes` row with `proposed_via = agent`, the person it was drafted for, the tool (the client from `external-apps.md` phase 8 when there is one), and the task sentence, and returns the change as sentences, its checks, and the impact from phase 7. **A draft expires 7 days after it is stored and can no longer be confirmed.** It applies nothing. `preview_access_change` returns the same without storing.
2. **Suggestions, computed, not guessed.** `GET /admin/access/suggestions`, and the read-only tool `access_suggestions`, list what could be simpler. Each comes from a query, not a model:
   - Memberships already covered by a grant.
   - Groups of person memberships that match one position at one unit, which a `holder_unit` grant could replace.
   - Grants and memberships past their validity that were never revoked.
   - Grants that reached nothing in the last 180 days, when receipts exist.
   - Fields with a sensitive category and no mask chosen beyond the default.
3. **A prompt for the administrator's AI tool**, `tidy_access`: read the suggestions, draft at most five changes that remove the most memberships while losing no one's access, preview each, and present the sentences and impact to the person. It never confirms.
4. **Confirming.** Drafts appear in the console's Access section with "Drafted by an AI tool used by {name}" and their sentences and impact. Confirming follows phase 4: widening needs a person other than the one who drafted it.
5. The cross-brief rule in the README states it: no MCP tool applies an admin write. A draft is a pending change that applies only when a person with the right confirms it.

Tests.

1. `draft_access_change` stores a pending change and changes no access. Run `role_at` for every fixture person before and after.
2. Its sentences and impact equal what the console shows for the same change.
3. A caller without the right to propose a change cannot draft it.
4. Each suggestion kind finds its seeded case and not its neighbor.
5. Replacing the suggested memberships with the suggested `holder_unit` grant leaves every fixture person's role unchanged on every object. Check with the reference evaluator from phase 5.
6. A widening draft cannot be confirmed by the person who asked their AI tool to draft it.
7. A stored draft records the person, the tool, and the task. A draft 7 days old is expired: confirming it is refused with a message that says it expired, and no access changes.

## How to prompt the build agent

```
Read docs/plans/access-model.md, docs/plans/stateless-server.md, docs/plans/ux-standards.md, and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
reach and role_at are pure. They do no I/O.
No hierarchy, unit, domain or position key in code or in a Cedar policy.
Nothing fails open. A failed load or compile is an error, never an empty list.
A widening change needs a second person. Every change is a ledger entry before the change.
Use the next free migration number.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| This is Workday's security model again. | Workday has about twenty group types, scope baked into the group, and inheritance modes that depend on who else holds a role lower down. Here there is one grant shape, scope is on the grant, and a grant's reach depends only on its own row and the trees. One `holder_unit` row serves every department, so roles do not multiply per unit. |
| Attribute systems cannot say who has access before the fact. | Grants are explicit rows with anchors. Review per object is one join of the closure with the grants. Review per person lists their grants and counts what each reaches. |
| A reorganization will trigger a recalculation of everyone's access. | No record stores access. A move rewrites the closure rows of one subtree and bumps one counter. Phase 2 measures a 500-unit move. |
| Hierarchical filtering will be slow at a million rows. | The filter is one more indexed set in the existing compiler. Phase 6 is a gate with stated limits, at a million rows with active grants and three hierarchies. If it fails, the work stops and the owner decides. |
| Adding a hierarchy later will need code. | A hierarchy is a row. No key appears in code or policy, and a test scans for them. The million-row gate runs with three hierarchies so the cost of the next one is known. |
| Builders will tag their way into wider access. | Coordinates only narrow from container to child. Lowering one raises a flagged check that needs a second person. A subject cannot set the attributes that decide its own access. |
| Grants break "guards only forbid" and "rules only narrow". | They do not. Grants permit, guards forbid over them, and rules narrow after them. That is now a settled decision. Task and flow grants in `workflows.md` use the same shape. |
| Cedar and SQL will drift apart. | Both read one reach function. A slow reference evaluator checks both on generated worlds, with properties for monotonicity, moves, caching, and view-as. |
| Impersonation in production is a liability. | Full impersonation is limited to synthetic people, and every write under it records both people. A real person can only be viewed, read-only, for one app, for 30 minutes. |
| Logging every decision will be too much data. | Receipts are written for denials, privileged actions, protected reads, and anything under view-as or impersonation: one per call, never per row. Phase 8 measures the cost and stops above 1 ms. |
| A tampered audit trail will go unnoticed. | Receipts are hash-chained per batch, and each batch hash is a ledger entry. Editing or removing a sealed receipt fails verification. |
| A vacancy will strand access. | A vacancy removes the holder, and with it that holder's reach. Nothing passes access up or down the tree because a seat is empty. Decisions that need a person route through `approvers.md`, which handles vacancies separately. |
| Stale appointments will keep access forever. | Appointments past their `end_date` leave the holder set. Requested grants expire. Every grant comes up for review on a schedule. |
| Automatic approval will hand out access nobody looked at. | Only requests that fit a rule the institution wrote and a second person confirmed are approved alone, and the default rule covers viewing non-sensitive apps in a person's own unit for a month. Every automatic approval names its rule, and every grant it makes expires. |
| An AI tool will grant itself access. | No MCP tool applies an access change. A tool drafts, and a person with the right confirms. A request an AI tool files waits for the person it acts for before anyone else sees it. |
| A mask is a weaker hide that will leak. | A masked field cannot be filtered, sorted, searched or totalled, so its order and range do not leak. A mask that shows more than its category allows needs a second person. |
| This much structure will need a specialist to run. | People see sentences, safe defaults and one decision at a time. Suggestions come from queries, drafts come from the administrator's AI tool, and the server checks every draft and shows its impact before anyone confirms it. |
