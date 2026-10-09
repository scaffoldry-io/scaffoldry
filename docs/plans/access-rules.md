# Access rules — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `approvers.md` phase 2. Phase 2 needs `row-scale.md` phase 2 and `guards.md` phase 1. Phase 5 needs `mcp-apps.md` phase 4a.

An academic advisor must see the students they advise and not the others. A department chair must see their department's requests. A scholarship reviewer must see a row without the column that holds the applicant's financial details. Workspace roles and workspace guards cannot say any of this. They decide who may use a workspace, not which rows and columns inside it.

This brief adds rules at the row and column level. They must hold at a million rows, on every path that reads data, and an owner must be able to see who a rule affects before saving it.

## Decisions already made

| Question | Answer |
| --- | --- |
| Can a rule grant access? | No. A rule narrows. Workspace members can read an app's rows by default, as today. A rule says which rows or columns a person may not see. This matches `guards.md` |
| How is a condition written? | As structured data, never free text. A small predicate tree over fields of one table and attributes of the signed-in person. It has a published schema |
| Is it evaluated by Cedar? | No. Row rules are evaluated by one Rust predicate evaluator and compiled to SQL for lists. A test proves both agree. Cedar keeps deciding app-level questions |
| What does a table-level question return when the answer depends on the row? | A scope of `AllowAll`, `DenyAll`, or `Mixed(predicate)`. A list applies the predicate in SQL. A single row is checked in Rust |
| What does a person outside a row's scope see? | A single row is "not found". Nothing says it exists. A list simply does not include it |
| Can a person filter, sort, search, or total on a column they cannot see? | No. It would let them read it by inference |
| Are rules part of the app's governed definition? | Yes. They live in the manifest and change by proposal. Removing or changing a rule is flagged and needs a second person |
| Who may ever see everything? | Platform Admins, and the roles an owner lists as exceptions on a rule |

## What exists today

| Fact | Where |
| --- | --- |
| The viewer rule hides `ferpa_sensitive` fields from a `viewer` | `mcp-apps.md` phase 2 |
| `effective_ferpa_sensitive` combines the manifest flag and a label | `admin-console.md` phase 4 |
| Workspace guards forbid by affiliation, unit, and sensitivity | `guards.md` |
| Workspace members are individuals only. Roles are `owner`, `admin`, `editor`, `viewer`, and `commenter` was added in `record-history.md` | `state.rs` |
| `record_index` lets SQL filter on indexed fields | `row-scale.md` phase 2 |
| Positions, groups, and units resolve to people | `approvers.md`, SCIM, `organization.md` |

## Out of scope

- Rules that look at a row in another table. A rule reads the row it protects.
- Rules that grant.
- Rules by time or place.
- A free-text condition language.
- Per-cell rules. A rule is for a row or a column.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. Every code path that reads a record, a count, a total, a search hit, a history entry, a comment, a file, or a link target goes through `row_scope` and `field_visibility`. A test lists the paths.
4. A denial never reveals that a hidden row or column exists.
5. Use the kit for every screen.

## Phase 1 — share with more than individuals

Migration `crates/scaffoldry-core/migrations/0030_member_principals.sql`:

```sql
ALTER TABLE workspace_collaborators ADD COLUMN IF NOT EXISTS principal_type VARCHAR(8) NOT NULL DEFAULT 'user';
ALTER TABLE workspace_collaborators ADD COLUMN IF NOT EXISTS principal_id VARCHAR(255);
UPDATE workspace_collaborators SET principal_id = eppn WHERE principal_id IS NULL;
```

`principal_type` is `user`, `group`, `unit`, or `position`. For a non-user principal, `eppn` holds `{type}:{principal_id}` so the existing uniqueness constraint still holds.

| Type | `principal_id` | Who it means |
| --- | --- | --- |
| `user` | the address | that person |
| `group` | a SCIM group id | the members the group currently lists |
| `unit` | an org unit id | everyone with a role at that unit or any unit below it, using `unit_ids` |
| `position` | `{key}@{unit_id}` | the current holders of that position at that unit |

Roles rank `viewer`, `commenter`, `editor`, `admin`, `owner`. Only a `user` principal may be an `owner`, and a workspace always has at least one.

Add one function, `member_role(user, workspace, state) -> Option<Role>`. It returns the highest role among every membership that matches the person. Replace every place that looks up a collaborator row by address, including `is_member` and `member_role` inputs to the policy engine, with this function. Run `grep -rn "collaborators" crates/` and paste the list before and after.

Routes. `POST /workspaces/{id}/collaborators` takes `principal_type` and `principal_id`, defaulting to `user`. Listing returns each with a display label. Adding a `group`, `unit`, or `position` is a change of access, so it is a ledger entry (`AccessRoleGranted`) with the principal and role.

Tests.

1. A workspace shared with a SCIM group gives each current member the group's role. Adding a person to the group gives them access on the next request. Removing them takes it away.
2. A person in two memberships, viewer through a unit and editor directly, has the editor role.
3. A `unit` membership covers a person whose role is at a unit below it, and not a sibling's.
4. A `position` membership follows the holder when the position changes hands.
5. A group cannot be made an owner: 400. Removing the last user owner is 409.
6. The grep list shows no remaining direct collaborator lookups outside `member_role`.
7. Every grant writes one ledger entry.

## Phase 2 — rules as data, and one scope

Types, in `scaffoldry-core`, with a schema at `governance/schema/access-rules.schema.json`:

```rust
pub enum Pred {
    True,
    Cmp { field: String, op: Op, value: Operand },
    And(Vec<Pred>),
    Or(Vec<Pred>),
    Not(Box<Pred>),
}
pub enum Op { Eq, Ne, In, IsEmpty, IsNotEmpty, Gt, Lt }
pub enum Operand { Literal(serde_json::Value), Person(PersonAttr) }
pub enum PersonAttr { Address, Department, Affiliation, UnitIds, PositionKeys, GroupIds }
pub enum RowScope { AllowAll, DenyAll, Mixed(Pred) }
```

`In` against a set (a literal list or a multi-valued person attribute) is true when the field's value, or any of its values for a multi-valued field, is in the set. A missing field is blank. `Eq` of a blank is false. `Ne` is the negation of `Eq`. Text comparison is case-insensitive over the first 128 characters, the same as the index. A literal longer than 128 characters is rejected. Numbers compare as numbers and dates as instants.

Functions.

- `substitute(pred, person) -> Pred`: replaces `Person` operands with the person's values and folds constants. A predicate that folds to `True` is `AllowAll`. One that folds to false is `DenyAll`. Otherwise it is `Mixed`.
- `eval(pred, record) -> bool`: the Rust evaluator for single records.
- `to_sql(pred, indexed) -> (String, Vec<Param>)`: each `Cmp` becomes an `EXISTS` over `record_index` for the same record, with every value a bound parameter. `And`, `Or`, and `Not` compose them. A field that is not in the indexed set is a compile error, not a scan.

Multi-valued fields. Change `row-scale.md` phase 2: `record_index` has the primary key `(record_id, field, ord)`, with `ord` a small integer, and a multi-valued field writes one row per value. `MultiSelect` and multi-valued collaborator fields are therefore indexable.

`row_scope(person, app, table, op) -> RowScope` takes the app's rules for that operation, skips any rule whose `except_roles` includes the person's role (Platform Admins always skip), combines the rest with `And` after `substitute`, and returns the result.

The equivalence test is the heart of this phase. Without adding a dependency, write a seeded generator (a simple xorshift) that builds, per case, a small table of records with random values, a random predicate of depth up to four, and a random person. It loads the records into PostgreSQL, runs the compiled SQL, runs `eval` over every record, and requires identical id sets. Run at least 2,000 cases. A failing case prints its seed.

Tests.

1. The 2,000-case equivalence test passes.
2. `substitute` folds `Cmp(Address eq Address)` to `True` and `Cmp(Department eq "Physics")` for a Chemistry person to `DenyAll`.
3. `to_sql` on a field outside the indexed set is an error.
4. A multi-valued field with values `[a, b]` matches `In [b, c]` and not `In [c]`, in both evaluators.
5. A literal over 128 characters is rejected.
6. The plan of a compiled predicate over 200,000 rows has no `Seq Scan` on `dataset_records` or `record_index`.
7. Blank and missing values behave the same in both evaluators for every `Op`.

## Phase 3 — every path

Define `RECORD_READERS: &[&str]`, the names of every service function that returns record data or anything derived from it: list, get, aggregate, search, export, history, comments, disclosures, link picker, process queue record fetch, page calls, and each MCP tool that reads. A test iterates it with a restricted person and asserts each applies the scope. Adding a reader without adding it to the list fails a second test that scans the service module for functions that load records.

Reads. A list adds `to_sql(scope)` to its `WHERE`. A single get checks `eval` and returns not found outside scope. A count or total is computed over the scoped set only.

Writes. `create` evaluates the `create` predicate against the new record. `update` evaluates the `update` predicate against both the stored record and the new one, so someone cannot hand a record outside their own scope. `delete` evaluates against the stored record. Platform Admins and excepted roles skip.

Inference. `field_visibility(person, app, table) -> hidden fields` is built in phase 4. In this phase, add the structure that uses it: a request to filter, sort, group, search, or total on a hidden field is refused with error code `field_hidden`. Add `field_hidden` and `field_read_only` to the closed list in `ux-standards.md`.

Tests.

1. A person restricted to rows where `advisor` is themselves lists 12 of 4,000 rows, counts 12, totals over 12, searches only within 12, and exports 12.
2. The same person gets not found for a row outside scope, and the response is byte-identical to a row that does not exist.
3. `create` with an advisor other than themselves is refused. `update` that changes the advisor to someone else is refused. An admin can.
4. Every function in `RECORD_READERS` applies the scope. The scan test finds no unlisted reader.
5. Realtime refetch, history, comments, and the link picker for a hidden row are all not found.
6. A page call through a custom page applies the scope.
7. At 200,000 rows a scoped list returns in under 200 ms and the plan has no sequential scan.

## Phase 4 — columns, and denials that explain

Column rules, in the manifest:

```rust
pub struct ColumnRule {
    pub id: String,
    pub table_id: String,
    pub fields: Vec<String>,
    pub mode: ColumnMode,     // Hidden or ReadOnly
    pub unless: Who,
    pub message: Option<String>,
}
pub struct Who { pub affiliations: Vec<String>, pub units: Vec<Uuid>, pub positions: Vec<String>, pub roles: Vec<Role>, pub addresses: Vec<String> }
```

A person who is not in `unless` (and is not a Platform Admin) cannot see a `Hidden` field and cannot change a `ReadOnly` one. `field_visibility(person, app, table)` returns the hidden set, merging these rules with `effective_ferpa_sensitive` for a viewer. `visible_changes` in `record-history.md`, every response serializer, exports, page calls, and lookups through links use this one function.

A hidden field is removed from responses. It is also removed from the field list in `describe_app`, so an agent working for that person is not told it exists. A write that includes a read-only field is refused with `field_read_only` naming the fields. Search is refused on a table that has any hidden field for the person.

Denials explain. A rule may carry a `message`: one plain sentence, 200 characters at most, written by the owner, such as `Financial details are visible to the scholarship office only.` The error body gains `reason` (the message, or a generated sentence) and `remedy` (generated from the rule: `Ask the scholarship office or an owner of this workspace.`). Add both to `ServiceError` JSON and to `explainError` in `ux-standards.md`. For a row outside scope the answer stays not found with no reason, because a reason would confirm it exists.

Tests.

1. A hidden field is absent from list, get, export, history masking, search results, and `describe_app` for a restricted person, and present for a person in `unless`.
2. A filter or sort on a hidden field is `field_hidden`.
3. A write to a read-only field is `field_read_only` and names it.
4. A rule's `message` appears as `reason` in the error. A generated remedy names the `unless` roles in words.
5. A lookup through a link never returns a hidden field of the target table.
6. A Platform Admin always sees everything.

## Phase 5 — authoring and approval

Rules join the manifest as `access: { rows: [RowRule], columns: [ColumnRule] }`. A row rule is `{ id, table_id, ops, visible_when: Pred, except_roles, message }`. The manifest validator checks: fields exist, are of a supported type (text, select, collaborator, number, date, boolean), and are indexable; the predicate has at most 20 nodes and depth 4; and rule fields join the indexed set, counting toward its limit of 20 per table.

Problem detection, computed on save and shown before approval:

| Problem | Meaning |
| --- | --- |
| `never_true` | The predicate folds to false for everyone, so only excepted roles ever see rows |
| `field_missing` | A field is gone or retired |
| `unsupported_field` | A field type a rule cannot use |
| `hides_all` | After the rule, no non-excepted role can see anything |

Proposal checks: `access_rules` (`warn` for problems, `fail` for a schema error). A rule that is removed, or whose predicate, operations, or exceptions changed, raises the flagged check `access_loosened`, which needs a second person. Adding a rule does not.

The change list reads each rule as a sentence: `People who are not owners or admins can see only rows where Advisor is themselves.` Build the sentence from the predicate's shape. When a shape has no pattern, read the tree plainly.

Preview. `POST /apps/{slug}/access/preview`, app owner or admin, body a person and a table. Returns, for the proposed rules, how many rows that person would see, add, change, and remove, and whether any column would be hidden. Counts only, never rows. It uses `row_scope` and a `count` over the scoped set.

Screen, in the app settings, tab `Who sees what`, from the kit.

- A table choice, then rules as sentences with `Add a rule`.
- The rule builder: operations as checkboxes (`see rows`, `add rows`, `change rows`, `remove rows`), conditions as rows of `field`, `is` or `is not` or `is one of`, and a value or `the person`, `their unit`, `their department`, `their positions`. Exceptions as roles. A message box with its character count.
- `Test a person`: choose a person, see `Dr. Rivera would see 12 of 4,031 rows and could change 12.`
- Problems as `Banner`s in words.
- `Save` submits a proposal and shows the Proposals screen from `mcp-apps.md` phase 4a.

Tests.

1. A manifest with an access rule on a non-indexable field fails validation.
2. Removing a rule raises `access_loosened`. The proposer cannot approve. Another admin can. Adding a rule does not raise it.
3. `never_true` is reported for a contradiction such as `And(Eq a x, Ne a x)` and for any predicate that folds to false. A predicate that depends on record values is not reported.
4. The preview returns counts matching a direct `row_scope` count and never returns row data.
5. The sentence for `Eq advisor = Address` reads as written above. A tree with no pattern is read plainly and nonempty.
6. Web: the builder produces the JSON the server accepts. The test panel shows the counts. A problem shows as a banner.

## Phase 6 — view as another user

Add the token kind `viewas` to `api_tokens`. `POST /apps/{slug}/view-as`, body `eppn`, app owner or admin or Platform Admin. It mints a token with `original_admin` set, a 30-minute expiry, and a scope of this app only. Its effective rights are the target's rights in this app intersected with the caller's own, so view-as can never show more than the caller could see. Every write route and every tool whose `read_only` is false refuses it with error code `view_as_read_only`. Add that code to the closed list. Ledger `ViewAsStarted` (add to `DecisionType::ALL`) with the target and app.

The desk shows a persistent `Banner`: `You are viewing Applicants as Dr. Rivera. This is read-only. Exit.` Exiting revokes the token.

Tests.

1. A view-as read returns exactly what a direct read as the target returns, intersected with the caller's own.
2. A write is `view_as_read_only`.
3. The token fails on another app.
4. It expires at 30 minutes. Exit revokes it.
5. An editor cannot start view-as. An app admin can.
6. Each start is a ledger entry.
7. Web: the banner is present and exit clears it.

## How to prompt Gemini

```
Read docs/plans/access-rules.md, docs/plans/guards.md, and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
A rule narrows. Never write one that grants.
Never reveal that a hidden row or column exists.
Do not evaluate a rule differently in Rust and in SQL. The equivalence test must pass.
Do not read, copy, or adapt another product's source code.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| A row rule will be skipped by some code path. | `RECORD_READERS` lists every path, a test runs each with a restricted person, and a scan test fails when a reader is missing from the list. |
| The SQL and Rust versions will disagree. | A 2,000-case randomized equivalence test runs in CI and prints the seed of a failure. |
| Rules will slow lists at a million rows. | A rule field joins the index, the plan is checked for sequential scans, and phase 3 measures a scoped list. |
| A rule will let people infer hidden data through counts and filters. | Counts run over the scoped set only. Filtering, sorting, searching, and totalling a hidden field is refused. |
| Owners will write rules they do not understand. | They choose from structured pieces, see a sentence, and see who it affects by name and count before saving. |
| A rule can be quietly removed. | A removal or change is flagged and needs a second person, and every approval is a ledger entry. |
| View as will leak what the viewer should not see. | Its rights are the intersection of the target's and the viewer's. It is read-only, scoped to one app, and expires in 30 minutes. |
