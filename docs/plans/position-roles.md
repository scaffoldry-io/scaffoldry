# Positions that grant roles — design note

```
Title:  A position can grant a workspace role to whoever holds it, and platform admin comes from a held role.
Risk:   notable
Ask:    Approve the design. Phase 1, 2a, and 3 are the core. Phase 2b and phase 4 are optional and need separate decisions.
IfYes:  A position can be given a role on one app, or on a whole workspace. Platform admin is a role a person holds, not a field on the person. Direct grants to people still work, and an admin can list them.
IfNo:   Access is workspace-wide or nothing, so a position that needs one app gets the whole workspace. Platform admin stays an attribute on the user record. Rights keep arriving by who a person is.
Checks: None yet. This is a design. Each phase below names its failing test first.
```

## The decision

The goal is that people hold no rights themselves. They hold positions and roles, and those carry the rights.

We keep both ways in. A workspace can still name a person directly, and a workspace can still name a position. We keep both to win buy-in and to avoid forcing a rewrite on units that run on named people today. A direct grant is allowed. It must be visible and it must say why.

## What exists today

| Fact | Where |
| --- | --- |
| A person holds a position at a unit as a `roles` row with `position_key` | `0021_positions.sql` |
| A position type has a key, name, description, org types, and `max_holders`. It has no rights | `PositionType` in `state.rs` |
| A workspace membership can name a `user`, `group`, `unit`, or `position` principal. The highest matching role wins | `members.rs`, `member_role` |
| A membership is keyed by workspace only. There is no way to grant one app. `authorize_app` finds the app's workspace and uses the caller's workspace role | `workspace_collaborators`, `access.rs`, `authorize_app` |
| Row rules narrow what a member reads in an app. They never grant. They read the person's unit ids, position keys, groups, department, and affiliation | `service/scope.rs`, `access-rules.md` |
| A position principal is written `key@unit-id` | `members.rs`, `Kind::Position` |
| An approver rule can name a position and walk up the unit tree | `ApproverSpec::Position` |
| `resolve_user` sets a person's affiliation to `central_admin` only when they hold a `platform_admin` role on the root unit. A record cannot grant it. About 35 server sites and the Cedar policies read that derived value. This note first said it came from a field on the SCIM user. That was wrong, and phase 1 found it | `identity.rs`, `resolve_user`, `root_roles` |
| Before phase 1, two things did not follow that rule: the setup identity was `central_admin` forever, and the admin count also counted users whose SCIM title said `central_admin` | `identity.rs`, `repository_people.rs` |
| Departmental read and write compare `principal.department` to `resource.department`, both strings | `scaffoldry-policy/src/lib.rs` |

## Standard

NIST RBAC (ANSI INCITS 359) is the model. A user is assigned roles. Roles carry permissions. Roles may form a hierarchy. A position is an organisational role: a seat in a unit that a person is assigned to. This design gives the seat a role set, which is the RBAC shape.

OSCAL names `role` and `responsible-party` for governance documents. That is documentation about who is responsible. It is not the runtime store for access. We do not put access data in OSCAL here. An OSCAL export of positions as responsible parties is a later, separate job.

Searched and not found: a published schema for "a position in a university that grants software roles". Higher-education identity work (eduPerson, SCIM enterprise extension) carries affiliation and title, not grants. So the grant field below is ours. It is one column and it reuses the existing role names.

## Decisions already made

| Question | Answer |
| --- | --- |
| Do direct person grants stay? | Yes. Both paths stay |
| Can a person hold a right as a field on their record? | Not for platform admin. This was already true except for the setup identity and an admin count. Phase 1 fixed both |
| Can access be given to one app instead of a whole workspace? | Yes. A membership gets an optional app. No app means the whole workspace, as now. This works for every principal kind |
| What is the main way a position gets access? | A membership naming `key@unit`, limited to the apps it needs |
| Does a position type grant a role by itself? | Optional, and off by default. It is a broad grant across a unit's workspaces. It needs a preview of how many workspaces it reaches before it saves |
| What does a grant give? | One role: viewer, commenter, editor, admin, or owner. Workspace-level actions (manage, delete) ignore app-scoped grants |
| Do row rules change? | No. They still only narrow. App scope decides who gets in. Row rules decide which rows they see |
| What if a person gets a role from two places? | The highest wins. This is how it works now |
| Does a held position grant platform admin? | No. Platform admin stays a `roles` row at the root unit. See the open question |

## Phase 1 — platform admin from a held role (done)

This note first described a larger change: move Cedar and about 35 server sites off `affiliation == "central_admin"`. Reading `resolve_user` showed the premise was wrong. The affiliation is already derived from a held `platform_admin` role on the root unit, and `scim_affiliation` can only return eduPerson affiliations, so no record can grant it. Rewriting 35 sites to read a boolean instead of a derived string would be churn with no gain in safety. It is not done. If the string ever confuses a reader, an `is_platform_admin` field on `AuthUser` is a safe later cleanup.

What was wrong, and is fixed:

- **The setup identity never ended.** `resolve_user` gave `setup@scaffoldry.local` `central_admin` for as long as a token existed. It now works only while no one holds `platform_admin` on the root unit. When the first admin is appointed it stops resolving, and its token gets a 401. If the database cannot say whether an admin exists, the identity is refused.
- **The admin count counted titles.** `platform_admin_names` also returned users whose SCIM title said `central_admin`. The overview showed that as a platform admin. It now counts held roles only.

Tests, in `platform_admin_source_test.rs`:
- A record that says `central_admin`, by title and by affiliation, grants nothing. This passed before the change and pins the behaviour.
- The setup identity is an admin on first boot and stops resolving once the first admin is appointed. This failed before the change.
- `platform_admin_names` lists the holder and not the titled user. This failed before the change.
- First boot through the API: the setup token works, appoints the first admin with `POST /orgs/{root}/appointments`, and then gets a 401. I wrote this test after the fix and did not watch it fail. The unit test above, which checks the same change, did fail first.

Not done in this phase: a browser pass of first boot, and a read of each of the 35 sites for a separate flaw.

## Phase 2a — grant one app

Failing test first: `chair@Chemistry` is a member of workspace W, limited to app A. The holder can write records in A. The holder cannot read app B in W. The holder cannot manage W. A holder of `chair` at Physics cannot reach A.

Change:
- Migration: `workspace_collaborators.app_slug TEXT NULL`.
- The membership uniqueness must include the app, so one principal can hold a workspace-wide role and a narrower one. I have not read that constraint's definition. Read it first.
- `member_role` takes the app. An app-scoped membership counts only for that app. `authorize_app` passes the slug. `authorize_workspace` ignores app-scoped memberships.
- Row rules: `scope_for` passes the app to the role lookup, so a rule exception for "editor" uses the role held on that app.
- Every record reader on the `RECORD_READERS` list already applies the row scope. Each also needs to take the app-aware role. The existing test that lists readers should fail until it does.

Open question, not verified: what an app-only member can see of the workspace around the app, such as its name and navigation. Read the workspace routes before this phase.

## Phase 2b — a position type grants a role (optional, off by default)

Failing test first: a person holds `chair` at Chemistry. The `chair` type grants `editor`. A workspace under Chemistry has no membership naming the person or the position. The person can write records. A person at Physics cannot.

Change:
- Migration: `position_types.workspace_role TEXT NULL`. Checked against the role names.
- `member_role` also considers the roles of the positions the person holds, for workspaces at or below that unit.
- Setting the grant is a ledger entry with a reason. The save screen shows how many workspaces it reaches first.
- The cache version for `position_types` already exists. Bump it on change.

This is the broad grant. Many institutions will leave it off. Do it only after phase 2a.

## Row rules and positions

Two points to settle with phase 2a, both found by reading `scope.rs`, neither tested:

- `person_of` gives rules bare position keys. A rule for "chair" cannot tell Chemistry's chair from Physics's. Decide whether `Person` carries `key@unit` pairs.
- `person_of` lists only the units where the person holds a role, not the units below. Cedar's check expands to descendants with `unit_ids_for`. Decide whether row rules should match.

## Phase 3 — direct grants are visible and carry a reason

Failing test first: an admin lists direct person grants and gets every `user` membership, every `Person` approver, and every `assigned_to`, each with who made it, when, and why. The same list marks which grants are app-scoped and which are workspace-wide.

Change: one read-only admin list. A direct grant made through the API requires a reason. Grants that exist today show a blank reason and are marked "before reasons were required".

## Phase 4 — department attribute becomes a unit (optional)

Cedar grants departmental read and write when two strings match. That is a right earned by a field on the person. The principal already carries `unit_ids` from held roles. The workspace has `organization_id`.

Do this only if you want it. It changes who can see departmental workspaces. Workspaces with no `organization_id` would stop matching. A count of those comes first. Do not start without it.

## Enterprise objection list

| Objection | Answer |
| --- | --- |
| "Two ways to grant means nobody knows who has access." | Phase 3 lists every direct grant. The existing member list shows position and unit grants with labels. Reviewers get one view per path, and the highest-role rule is already tested |
| "A position grant is broad. One seat can open every workspace in a unit." | The main path is a membership limited to one app. The type-level grant is optional and off by default. When used, it is scoped to the unit and below, shows how many workspaces it reaches before saving, and is a ledger entry with a reason |
| "We must not over-provision. A position that needs one app should not get the workspace." | Phase 2a. An app-scoped grant reaches that app only. Manage and delete stay workspace-level. Row rules still narrow inside the app |
| "People change seats. Access must follow." | It does. The grant is computed from the roles table on each check, as `Kind::Position` is today. No copy of the grant is stored on the person |
| "A setup identity that is admin forever is a standing backdoor." | It was. It now ends when the first admin is appointed, and it is refused if the database cannot say whether one exists. A test proves both ends |
| "Cedar already decides access. Why add a second place?" | There is no second decider. `member_role` already feeds Cedar the `member_role` and `is_member` fields. Phase 2 changes its input, not Cedar |
| "Departments differ in size and shape." | The role set is per position type, so each institution defines its own. The built-in names are only the five workspace roles |
| "Audit wants to see who could do what on a given date." | Position changes and the new grant field are ledger entries. Holder history is in `roles`. A point-in-time query is not built. See below |

## Not decided, and not verified

- **Platform admin as a position.** An institution may want "CIO" to carry platform admin. This design keeps it a role row. A position-granted platform role is possible later, and is a high-risk grant that deserves its own review.
- **Point-in-time access reports.** The ledger and `roles` hold the facts. No query joins them. Not in scope.
- **Other routes.** I listed every `central_admin` use in the Rust sources. All but the setup identity and the admin count read the derived value. I did not read each of the 35 sites for a separate flaw.
- **Direct grants of platform rights.** Phase 3 lists workspace-level direct grants only. Admin rights by role row are already visible in the roles list.

## Prompt for the session

Read this file and `docs/plans/foundation.md` "Decisions already made". Phase 1 is done. Do phase 2a only. Read the membership uniqueness constraint and the workspace routes first. Write the failing test, show it fail for the stated reason, make it pass, run the full workspace suite, and stop. Do not start phase 2b.
