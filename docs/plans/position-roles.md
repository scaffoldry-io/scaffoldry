# Positions that grant roles — design note

```
Title:  A position can grant a workspace role to whoever holds it, and platform admin comes from a held role.
Risk:   notable
Ask:    Approve the design. Phase 1 and 2 are small. Phase 4 is optional and needs a separate decision.
IfYes:  Holding "chair" at a unit can give a role across that unit's workspaces. Platform admin is a role a person holds, not a field on the person. Direct grants to people still work, and an admin can list them.
IfNo:   A position grants nothing until a workspace names it. Platform admin stays an attribute on the user record. Rights keep arriving by who a person is.
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
| A position principal is written `key@unit-id` | `members.rs`, `Kind::Position` |
| An approver rule can name a position and walk up the unit tree | `ApproverSpec::Position` |
| Platform admin in Cedar comes from `caller.affiliation == "central_admin"`, a field on the SCIM user | `access.rs`, `decide` |
| A `platform_admin` role row also exists, and unit scope reads it. Cedar does not | `identity.rs`, `unit_in_scope` |
| Departmental read and write compare `principal.department` to `resource.department`, both strings | `scaffoldry-policy/src/lib.rs` |

## Standard

NIST RBAC (ANSI INCITS 359) is the model. A user is assigned roles. Roles carry permissions. Roles may form a hierarchy. A position is an organisational role: a seat in a unit that a person is assigned to. This design gives the seat a role set, which is the RBAC shape.

OSCAL names `role` and `responsible-party` for governance documents. That is documentation about who is responsible. It is not the runtime store for access. We do not put access data in OSCAL here. An OSCAL export of positions as responsible parties is a later, separate job.

Searched and not found: a published schema for "a position in a university that grants software roles". Higher-education identity work (eduPerson, SCIM enterprise extension) carries affiliation and title, not grants. So the grant field below is ours. It is one column and it reuses the existing role names.

## Decisions already made

| Question | Answer |
| --- | --- |
| Do direct person grants stay? | Yes. Both paths stay |
| Can a person hold a right as a field on their record? | Not for platform admin, after phase 1 |
| What does a position grant? | One workspace role: viewer, commenter, editor, admin, or owner. Or nothing |
| Where does the grant apply? | Workspaces whose unit is the position's unit or below it. The same rule `Kind::Unit` uses today |
| What if a person gets a role from two places? | The highest wins. This is how it works now |
| Does a held position grant platform admin? | No. Platform admin stays a `roles` row at the root unit. See the open question |

## Phase 1 — platform admin from a held role

Failing test first: a user whose SCIM affiliation says `central_admin` but who holds no `platform_admin` role at the root unit is not a platform admin. A user with the role and any affiliation is.

Change: `decide` sets `is_platform_admin` from the stored role. Cedar policies that compare `scoped_affiliation == "central_admin"` read `is_platform_admin` instead.

Risk in this phase: `central_admin` appears in ten source files, not only `access.rs`: `guards.rs`, `lib.rs` (policy), `identity.rs`, `organizations.rs`, `scope.rs`, `access.rs`, `proposals.rs`, `workspaces.rs`, `admin.rs`, and `repository_people.rs`. Each is in scope. The test token helper in `identity.rs` writes the role row for `central_admin` personas. I have not read how the first-boot setup token makes the first admin (`create_setup_token`). That must be read before the change.

## Phase 2 — a position grants a workspace role

Failing test first: a person holds `chair` at Chemistry. The `chair` type grants `editor`. A workspace under Chemistry has no membership naming the person or the position. The person can write records. A person at Physics cannot.

Change:
- Migration: `position_types.workspace_role TEXT NULL`. Checked against the role names.
- `member_role` also considers the roles of the positions the person holds, for workspaces at or below that unit.
- The cache version for `position_types` already exists. Bump it on change.
- Setting or changing the grant is a ledger entry with a reason, like any position change.

## Phase 3 — direct grants are visible and carry a reason

Failing test first: an admin lists direct person grants and gets every `user` membership, every `Person` approver, and every `assigned_to`, each with who made it, when, and why.

Change: one read-only admin list. A direct grant made through the API requires a reason. Grants that exist today show a blank reason and are marked "before reasons were required".

## Phase 4 — department attribute becomes a unit (optional)

Cedar grants departmental read and write when two strings match. That is a right earned by a field on the person. The principal already carries `unit_ids` from held roles. The workspace has `organization_id`.

Do this only if you want it. It changes who can see departmental workspaces. Workspaces with no `organization_id` would stop matching. A count of those comes first. Do not start without it.

## Enterprise objection list

| Objection | Answer |
| --- | --- |
| "Two ways to grant means nobody knows who has access." | Phase 3 lists every direct grant. The existing member list shows position and unit grants with labels. Reviewers get one view per path, and the highest-role rule is already tested |
| "A position grant is broad. One seat can open every workspace in a unit." | It is scoped to the unit and below, and it is off unless the type sets it. Each change is in the ledger with a reason. We should add a preview of how many workspaces a change reaches before saving. That is not designed here |
| "People change seats. Access must follow." | It does. The grant is computed from the roles table on each check, as `Kind::Position` is today. No copy of the grant is stored on the person |
| "Removing the affiliation shortcut for admins could lock us out." | Phase 1 tests first boot and the test helper. Do not merge without a first-boot test that proves an admin exists |
| "Cedar already decides access. Why add a second place?" | There is no second decider. `member_role` already feeds Cedar the `member_role` and `is_member` fields. Phase 2 changes its input, not Cedar |
| "Departments differ in size and shape." | The role set is per position type, so each institution defines its own. The built-in names are only the five workspace roles |
| "Audit wants to see who could do what on a given date." | Position changes and the new grant field are ledger entries. Holder history is in `roles`. A point-in-time query is not built. See below |

## Not decided, and not verified

- **Platform admin as a position.** An institution may want "CIO" to carry platform admin. This design keeps it a role row. A position-granted platform role is possible later, and is a high-risk grant that deserves its own review.
- **Point-in-time access reports.** The ledger and `roles` hold the facts. No query joins them. Not in scope.
- **Other routes.** I read `access.rs`, `members.rs`, the Cedar policies, and the position and identity code. I did not audit every route for affiliation checks. Phase 1 starts from the ten files listed there. Any other hit is in scope.
- **Direct grants of platform rights.** Phase 3 lists workspace-level direct grants only. Admin rights by role row are already visible in the roles list.

## Prompt for the session

Read this file and `docs/plans/foundation.md` "Decisions already made". Do phase 1 only. Write the failing test, show it fail for the stated reason, make it pass, run the full workspace suite, and stop. Do not start phase 2.
