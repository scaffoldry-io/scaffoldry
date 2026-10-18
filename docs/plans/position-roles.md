# Positions that grant roles — design note

```
Title:  A position can grant a workspace role to whoever holds it, and platform admin comes from a held role.
Risk:   notable
Ask:    Phases 1, 2a, 2b, and 3 are built. Phase 4 is replaced by the dataset-level authorization design at the end of this note. Approve that design before any of it is built.
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

## Phase 2a — grant one app (done, with one open question)

A membership can be limited to one app. It works for every principal kind: person, group, unit, position.

What changed:
- Migration `0055_member_app_scope.sql`: `workspace_collaborators.app_slug`, `NOT NULL DEFAULT ''`. Empty means the whole workspace. The unique key is now `(workspace_id, eppn, app_slug)`, so one principal can hold both a workspace-wide and an app-limited membership. An empty string is used instead of NULL because a NULL in a unique key counts as distinct.
- `member_role` still means the workspace-wide role. Every workspace-level path is unchanged, and an app grant never reaches them. `member_role_in_app` is the highest of the workspace-wide role and the roles limited to that app.
- App-level paths use it: `authorize_app`, both row-rule lookups and the field-visibility lookup in `scope.rs`, and the viewer check in `records.rs`. Row rules still only narrow.
- Adding a member accepts `app_slug`. The app must belong to the workspace. An app membership cannot be an owner. The ledger entry carries the app.
- Removing a member takes `?app=` and removes only that scope. Changing a role touches the workspace-wide membership only. To change an app membership, remove it and add it again.

Tests, in `member_principals_test.rs`:
- A chair given Admin on app A can write A, cannot reach app B in the same workspace, and cannot add members to the workspace. A chair of another unit reaches nothing. This failed before the change, because the app was ignored.
- The app must be in the workspace, an app membership cannot be an owner, a principal can hold both kinds, and removal takes one scope at a time. I wrote this after the code and did not watch it fail.

Decided: a separate "My apps" list, not a change to the workspace routes. An app-only member still does not see the workspace in `GET /workspaces`, and `GET /workspaces/{id}` still returns 403. `GET /api/v1/me/apps` lists every app the caller can reach, each with how: `workspace` (a workspace-wide role), `app` (a membership limited to that app), or `admin`. The desk has a "My apps" page and a rail entry. It shows no workspace details, only the workspace id on each app.

Pinned apps: the desk already pinned workspaces, in this browser's local storage. Apps can now be pinned the same way, from the My apps page. Pinned apps show in their own "Pinned apps" group in the rail, above Data and Studio Tools, and first on the My apps page. Limit: pins are per browser, as workspace pins are. A pin does not follow a person to another browser. Storing pins on the server would fix that and is a separate decision.

Desk fix made on the way: `/app/{slug}` used to open the first app when the slug was not in the desk's list. For a person reached through an app grant, that would show the wrong app. It now says the app cannot open in the desk yet. The published-app screen is still a mock-up (`1c-1`), so opening an app from My apps shows that screen only for apps the desk already knows.

Also not done: app-limited members are not in `people_with_roles`, so they are not offered as approvers by workspace role. A decision comes with the list above.

## Phase 2b — a position type may grant a role (done, off by default)

**Decided:** it is an option on each position type. No type has to use it. A new type grants nothing.

Built. The first test failed because a chair with no membership could not write. It passes now. It also checks that a chair at another department gets nothing, that a workspace above the chair's unit is not reached, and that a type that grants nothing or is retired gives nothing. The test was watched to fail with the new code switched off.

- Migration `0057_position_type_role.sql`: `position_types.workspace_role`, null by default.
- `member_role` and `member_role_of` also take the highest role from the positions a person holds. A held position reaches workspaces at and below the unit where it is held. `people_with_roles` includes these holders too, so reports and approver lists are not blind to them.
- `PATCH /api/v1/admin/positions/{key}` takes `workspace_role`. A role name sets it. `null` removes it. Leaving the key out changes nothing. It needs a reason and writes one `PositionChanged` ledger entry.
- The roles a type can grant are `viewer`, `commenter`, `editor`, and `admin`. Owner is refused. Owning a workspace is a person's act. I chose this. Say if you want owner allowed.
- The admin list of position types shows `workspaces_reached`: the workspaces at or below a unit where someone holds the position today. A workspace with no unit is never reached. The desk save screen should show this number before the admin confirms. The desk has no screen for it yet.
- The cache version for `position_types` already exists and is bumped on change.

Not built: the desk screen.

## Row rules and positions

Two points were open. The decision on organization (below) answers both.

- `person_of` gives rules bare position keys. A rule for "chair" cannot tell Chemistry's chair from Physics's. **Decided:** the person carries `key@unit` pairs, so a rule can name a position at a unit.
- `person_of` lists only the units where the person holds a role, not the units below. **Decided:** the rule says whether it covers the unit alone or the unit and below.

Neither is built. They are part of phase 5a.

## Phase 3 — direct grants are visible (3a done), and carry a reason (3b not done)

**3a, done.** `GET /api/v1/admin/direct-grants` lists every grant that names a person. Only a Platform Admin may call it. Each row has a kind:
- `membership`: a person named in a workspace or app membership. The row says whether it is workspace-wide or limited to an app, the role, who made it, when, and the ledger's rationale. Who and when come from the latest ledger entry for that member and scope, found with one statement. A membership with no ledger entry shows no granter and the time it was added.
- `approver`: an approver rule that names a person, with the app, rule, and step. Rules change by proposal, so no granter is shown here.
- `assignment`: an open instance assigned to a person.

Position, group, and unit memberships are not listed. They are not direct grants.

Tests: the membership list with who made each grant, and the refusal for a non-admin, in `member_principals_test.rs`. The approver and assignee rows in `approver_engine_test.rs`. The first failed before the change, on the missing route. The second I wrote after the code and did not watch fail.

**3b, decided: the reason is optional.** The add-member request accepts an optional `reason`. When given, the ledger records it as the rationale and the report shows it. Without one, the ledger keeps its generated wording, so nothing that sends no reason today changes. The desk has no field for it yet. Test: a grant with a reason and one without, in `member_principals_test.rs`. It failed first, because the reason was ignored.

Not done in 3a: the desk has no screen for this list. It is an API only.

## Phase 4 — replaced

The first idea was to turn the department attribute into a unit and let it grant departmental workspaces. That is not the goal. Your decision:

- An organization is one hierarchy that a position sits in. Having a unit does not grant a workspace.
- The unit combines with a role to authorize rows where that department and that role meet.
- Access to an app or dataset is scoped to roles, perhaps to roles at particular units.
- Security is primarily at the dataset level. Apps and pages can be shared in many ways. OSCAL rules are enforced from data labels, so that the roles, positions, and organization tags a person holds decide access to labeled data.

The old department-string match in Cedar stays as it is until phase 5 replaces what it protects. Nothing in phase 5 changes who can open a workspace.

## Dataset-level authorization (proposal, phase 5, not built)

**Update:** the direction widened. Data becomes its own layer, grouped into datasets that keep the labels, with row tags first and a dataset floor and default. That is written in [data-layer.md](data-layer.md), which now owns phases 5b and 5c below. Phase 5a (people carry `key@unit` pairs) is now `access-model.md` phase 1, item 5. Do not build it from this note.

Needs your approval. I read `scope.rs`, `access-rules.md`, and `sensitive-content.md` to write it. I did not read `oscal-catalog.md` yet. The OSCAL mapping below must be checked against it before anything is built.

### What exists

- A label or a category marks a field protected. Today a protected field is hidden from the `viewer` role and kept out of exports and links. The test is by role only. It does not ask which unit or position the reader holds.
- Row rules narrow. They never grant. They can read the person's unit ids, position keys, groups, department, and affiliation.
- Sharing an app gives access to the app. It has no say over the labeled data inside, except the viewer rule.

### The idea

A label carries an **authorization requirement**: who may read data with this label. A requirement is a list of grants. Each grant names a role or a position, and a reach in the organization.

| Part | Meaning |
| --- | --- |
| Role or position | A workspace role, or a position key such as `registrar` |
| Reach | `unit only`, `unit and below`, or `anywhere` |
| Data tag | The unit the data belongs to. It comes from the dataset's owning unit, or from a field on the row |

A person may read labeled data when they hold a grant whose role or position matches, at a unit that covers the data's tag. Sharing an app never overrides this. Sharing says who may open the app. The label says who may read the data in it. This keeps the rule you already have, that a rule narrows and never grants.

Data with no label behaves as today. Labeled data with no requirement behaves as today. A requirement is opt-in per category, so nothing changes until an institution sets one.

### OSCAL

The requirement is governance data, so it follows `governance-data-is-oscal`. The matching NIST SP 800-53 controls are AC-3 (access enforcement), AC-16 (security and privacy attributes), AC-6 (least privilege), and the PT family for personal data. The requirement is stored on the category as OSCAL `props` and `links`. No new field is invented for something OSCAL names. A report can list, for any category, who is authorized and by which grant. That is the evidence an assessor asks for. The control ids must be confirmed against the catalog once `oscal-catalog.md` is read.

### Phases

| Phase | Failing test first | Change |
| --- | --- | --- |
| 5a | The Chemistry chair passes a rule for `chair@Chemistry`. The Physics chair does not. A rule that covers a unit and below passes a lab under it | `person_of` carries `key@unit` pairs. A row rule can name a pair and a reach |
| 5b | A field labeled `PHI` with a requirement `registrar, unit and below` is read by the registrar of the owning unit. A viewer who was shared the app, and the registrar of another unit, get no values, no count, and no search hit | A category holds a requirement. The one effective-sensitivity function returns it. Every read path already goes through that function, and a test lists the paths |
| 5c | A report for one category lists each authorized person and the grant that authorizes them | The report, with the OSCAL props. It is a ledger-backed read, not a new store |

### Open questions

1. (Decided in [data-layer.md](data-layer.md): a column on the row, and the dataset's unit when it is missing.) Where does a row's organization tag come from? The dataset's owning unit is simple. A field on the row (a `department` link to a unit) lets one dataset serve many units. I lean to the dataset's unit first, and a row field only when a rule needs it.
2. If a requirement is set and the data has no tag, who may read? I lean to deny, and say so in the error.
3. Does a platform admin read labeled data? Today they see everything. I lean to keep that, with every read of labeled data by an admin written as a disclosure.

### Enterprise objections

| Objection | Answer |
| --- | --- |
| "Two systems decide access, Cedar and labels." | Cedar still decides who opens a workspace or an app. The label requirement is an extra check on labeled data, in the same place the viewer rule is. It only narrows |
| "A label requirement can lock out a legitimate owner." | The owner sees the requirement and who it reaches before saving, as for phase 2b. Platform admins can read, with a disclosure |
| "This is slow at a million rows." | A requirement on a field removes the whole field for an unauthorized reader. A requirement on rows compiles to SQL through the same `record_index` path as row rules. It must pass `million_rows_test`. I have not measured it |
| "Who can change a requirement?" | It is a settings change by proposal, a ledger entry with a reason, and a second person for a removal, the same as rules |

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

Read this file and `docs/plans/foundation.md` "Decisions already made". Phases 1, 2a, 2b, and 3 are done. Phase 4 is replaced by phase 5, which needs approval and the open questions answered. Still open: a desk screen for the direct-grants list, a desk field for the optional reason, and a desk screen to set a position type's role and show its reach.
