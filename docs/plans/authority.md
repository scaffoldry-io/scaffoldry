# Decision authority — implementation brief

> **Status: Open**

Hand this file to the build agent. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `mcp-apps.md` phases 4 and 4a, `approvers.md` phase 1, and `organization.md` phase 1. Phase 2 needs `admin-console.md` phase 1. Phase 3 links to `policy-library.md` phase 4 when that has run. Phase 4 needs phases 1 and 2.

A faculty member changes an app. Who has to say yes? Today the answer is fixed in the code. A change with no raised check is applied by a workspace owner. A change that raises one of four checks needs any second owner or admin. Every department gets the same answer, and nobody can see or change the rule.

That does not survive a large institution. The medical school wants its privacy officer on anything that touches patient data. A small department wants its two owners left alone. The compliance office wants to know, before it tightens a rule, how many requests will land on whom.

This brief makes the answer data. A change has a **tier**. A **matrix**, set by the institution and tightened by units, says what each tier needs. The builder is told why, in a sentence, with a link to the policy behind it. An administrator sees the effect of a change to the matrix before it applies.

## Standard

FIPS 199 names three impact levels: **Low**, **Moderate**, and **High**. This brief uses them as the tiers and invents no scale. NIST SP 800-53 CM-3 (configuration change control) and CM-4 (impact analysis) are the controls a tier and a matrix implement. AC-5 is separation of duties. RA-3 is the assessment the matrix records.

Record the search in the first commit message: FIPS 199 (adopted for tier names), SP 800-53 CM-3, CM-4, AC-5, RA-3. No published standard defines a delegation-of-authority matrix as data. The W3C Organization Ontology, used by `approvers.md`, has posts and roles and no notion of what a post may approve. The matrix is therefore the platform's own shape, with a published JSON schema, and each row may carry an OSCAL control id.

## Decisions already made

| Question | Answer |
| --- | --- |
| What decides a tier? | The checks a proposal already runs. Each check that can be raised has a tier. A proposal's tier is the highest tier among its raised checks, and Low when none is raised. No model and no score is involved |
| What can a tier require? | One of three paths, in order of strictness. `direct`: an owner or admin of the workspace decides, and may decide their own change. `second_person`: an owner or admin other than the proposer. `approver`: the people a named approver spec resolves to, other than the proposer |
| What is the floor? | The behavior the platform has today. A raised check on the flagged list always needs a person other than the proposer. No row can lower that. Low may not be lower than `direct` |
| Who sets the institution's rows? | A Platform Admin or a `compliance` caller proposes a change. A different person confirms it. The setup identity counts as a different person, as it does for policy activation |
| May a unit loosen the institution's rule? | No. A unit's rows can only raise a check's tier or require a stricter path. The effective answer for a workspace is the strictest one on the path from its unit to the root |
| Who sets a unit's rows? | An Org Unit Admin of that unit, confirmed by a different person who is an Org Unit Admin of that unit or of a unit above it, or a Platform Admin |
| When is the requirement computed? | When the proposal is made, so the builder can be told. Again when it is decided, from the matrix at that moment. The second one is the one enforced |
| What if nobody resolves? | The proposal stays `Pending`, is flagged `no_approver`, and appears in the console as Unassigned. It is never applied without a decision, and it never falls back to a looser path |
| Can a matrix change reach back? | No. It applies to decisions made after it is confirmed. A proposal already approved is not reopened |

## What exists today

| Fact | Where |
| --- | --- |
| Checks are JSON objects with `name`, `outcome` (`pass`, `warn`, `fail`), and `messages` | `service/proposals.rs` `check`, `warn` |
| `FLAGGED_CHECKS` is `sensitivity`, `data_loss`, `page_sensitivity`, `rule_effects`. A failed check on that list does not refuse the proposal. It requires a second person | `repository_proposals.rs` |
| `approve_proposal` returns `SecondPersonRequired` when a flagged check failed and the decider is the proposer | `repository_proposals.rs` |
| `submit_direct` applies a change in the same request when the caller owns or administers the workspace and no check failed | `service/proposals.rs` |
| Later briefs add flagged checks: `guest_exposure` in `guests.md`, a changed access rule in `access-rules.md` phase 5, grants in `workflows.md` | Those briefs |
| A workspace belongs to one organization unit, and units form a tree | `organization.md` phase 1 |
| `ApproverSpec` and `resolve_approvers`, pure, with delegation and vacancy handling | `crates/scaffoldry-core/src/approver.rs` |
| Guards show their impact before they are saved | `guards.md` phase 3 |

## Out of scope

- More than one approval on one proposal, quorums, and committees. `workflows.md` phase 1 adds `proposal_approvals` for flows across workspaces, and `process-v2.md` phase 4 adds votes. A `two_approvers` path belongs after both.
- Any path that applies a flagged change without a person.
- Scoring by a model, or thresholds on money or head count.
- Timers, reminders, and escalation for a waiting proposal. `process-v2.md` phase 3 owns timers.
- Authority over process steps. A step's approver is set in the flow by `approvers.md` phase 5. This brief governs changes to apps, rules, and definitions, which are proposals.
- A prediction of how long people take to decide. Phase 4 reports load and gaps. Waiting time needs real data from `process-v2.md` phase 1.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. `required` is a pure function. It does no I/O. Everything else calls it.
4. With no rows in either table, every existing test in the repository passes unchanged. The defaults are today's behavior.
5. No code path applies a proposal except through the path `required` returns at the time of the decision.
6. Every change to the matrix is a ledger entry written before the change, with a reason. Add `AuthorityChanged` to `DecisionType::ALL`, using the one-definition mechanism from `admin-console.md` phase 1.
7. Use the kit in `apps/web/src/ui/` for every screen. Add every new error code to the closed list in `ux-standards.md` and to `explainError`.

## Phase 1 — tiers, the matrix, and one pure function

Migration `crates/scaffoldry-core/migrations/0066_authority.sql`:

```sql
CREATE TABLE IF NOT EXISTS authority_signals (
    organization_id UUID NOT NULL REFERENCES organizations(id),
    check_name VARCHAR(64) NOT NULL,
    tier VARCHAR(8) NOT NULL,
    PRIMARY KEY (organization_id, check_name)
);

CREATE TABLE IF NOT EXISTS authority_rows (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id),
    tier VARCHAR(8) NOT NULL,
    path VARCHAR(16) NOT NULL,
    approver JSONB,
    control_id VARCHAR(32),
    library_policy_key VARCHAR(64),
    library_rule_key VARCHAR(64),
    UNIQUE (organization_id, tier)
);

CREATE TABLE IF NOT EXISTS authority_changes (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id),
    change JSONB NOT NULL,
    reason TEXT NOT NULL,
    status VARCHAR(10) NOT NULL DEFAULT 'Pending',
    proposed_by VARCHAR(255) NOT NULL,
    proposed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    decided_by VARCHAR(255),
    decided_at TIMESTAMPTZ
);
```

Closed lists. `tier`: `Low`, `Moderate`, `High`. `path`: `direct`, `second_person`, `approver`. `authority_changes.status`: `Pending`, `Confirmed`, `Rejected`, `Withdrawn`. `approver` is an `ApproverSpec`, validates against `governance/schema/approver-spec.schema.json`, and is required when `path` is `approver` and forbidden otherwise.

Add `governance/schema/authority.schema.json` for a signal row, a matrix row, and the body of a change. Add it to the validation step that already checks the other schemas.

Read both tables with one query each when a proposal is made or decided. They are small, and a proposal is not a hot path. Do not cache them and do not add a field to `ServerState`.

The pure function, in `crates/scaffoldry-core/src/authority.rs`:

```rust
pub enum Tier { Low, Moderate, High }
pub enum Path { Direct, SecondPerson, Approver }

pub struct Requirement {
    pub tier: Tier,
    pub path: Path,
    pub approver: Option<ApproverSpec>,
    /// One entry per raised check and per row that tightened the answer. Each is a sentence.
    pub reasons: Vec<Reason>,
}

pub struct Reason {
    pub check: Option<String>,
    pub organization_id: Option<Uuid>,
    pub sentence: String,
    pub control_id: Option<String>,
    pub library_rule: Option<(String, String)>,
}

pub fn required(raised: &[String], unit_path: &[Uuid], signals: &[SignalRow], rows: &[MatrixRow]) -> Requirement
```

`raised` holds the names of the checks whose outcome is `fail` and that are on `FLAGGED_CHECKS`. `unit_path` is the workspace's unit first, then each unit above it, ending at the root.

1. Built-in tier of every flagged check: `Moderate`. A check that is not flagged has no tier.
2. A check's tier is the highest of its built-in tier and every `authority_signals` row for it on `unit_path`.
3. The proposal's tier is the highest tier among `raised`, and `Low` when `raised` is empty.
4. Built-in path: `Direct` for `Low`, `SecondPerson` for `Moderate` and `High`.
5. The path is the strictest of the built-in path for the tier and the path of every `authority_rows` row on `unit_path` whose tier is the proposal's tier or lower. A requirement set for Moderate therefore also holds for High.
6. When the path is `Approver`, the approver spec is taken from the nearest unit on `unit_path` that has a row with path `Approver` at the proposal's tier or lower. Nearest means the workspace's own unit first.
7. `reasons` holds one sentence for each raised check, built from a fixed table of check name to sentence held beside `FLAGGED_CHECKS`, and one for each row that made the path stricter than the built-in one, naming the unit. A flagged check with no sentence in that table fails a test.

The built-in values in steps 1 and 4 are constants in the source, next to `FLAGGED_CHECKS`. No table can lower them, because the function takes the highest and the strictest.

Wire it in.

1. `propose` computes `required` and stores it under `authority` in the proposal's `checks` JSON: tier, path, the approver spec, the resolved people with `via`, and the reasons. This is what the builder reads.
2. `decide` recomputes `required` from the tables as they are now. Who may approve: for `Direct`, an owner or admin of the workspace. For `SecondPerson`, an owner or admin of the workspace who is not the proposer. For `Approver`, a person `resolve_approvers` returns for the spec with the workspace's unit as the origin, who is not the proposer. A Platform Admin may decide only where one of those lets them. Move the test in `approve_proposal` step 3 to use this answer. `Approval::SecondPersonRequired` stays, and gains `ApproverRequired { resolved: Vec<String> }`.
3. `submit_direct` applies in the same request only when the path is `Direct` and the caller owns or administers the workspace. Otherwise it returns `Pending` with the `authority` object.
4. When the path is `Approver` and nobody resolves, the proposal carries `no_approver` and is listed by `GET /admin/proposals?unassigned=true`. Add that query to the existing admin proposals route. If no such route exists, add it and add it to `ADMIN_ROUTES`.
5. Rejecting needs no special right beyond the right to approve. The proposer may withdraw their own proposal.

Tests.

1. With both tables empty, the whole existing proposal test suite passes unchanged. Paste the count before and after.
2. `required` table test, pure: no raised check is `Low`, `Direct`. One flagged check is `Moderate`, `SecondPerson`. A signal row at the college raising `sensitivity` to `High` makes a department's proposal `High`.
3. A row at the root, `Moderate` → `Approver` (the privacy officer position), applies to a `High` proposal in any unit.
4. A unit row with path `direct` for `Moderate` changes nothing. The answer stays `SecondPerson`.
5. Two units on the path each name an approver. The nearer unit's spec is used.
6. `Approver` path: a workspace owner who is not in the resolved set gets `403` with the names of who can decide. A resolved holder who is not the proposer gets `200`. A resolved holder who is the proposer gets `403`.
7. `Approver` path with a vacant position and no walk up: the proposal is `no_approver`, is listed as Unassigned, and stays `Pending`.
8. The matrix is tightened after a proposal is made and before it is decided. The decision uses the tightened answer.
9. Every name in `FLAGGED_CHECKS` has a sentence. The test iterates the list.
10. A scan test: `approve_proposal` is called from one place, and that place calls `required` first.

## Phase 2 — the matrix in the console, with its effect shown first

Routes. Add them to `ADMIN_ROUTES`.

| Method | Path | Who | Behavior |
| --- | --- | --- | --- |
| GET | `/admin/authority` | Platform Admin, `compliance`, or an Org Unit Admin | Query `organization_id`. The built-in defaults, the signal rows and matrix rows on the path from that unit to the root, and the effective answer for each tier, with the unit that set it |
| POST | `/admin/authority/impact` | The same | Body `organization_id` and a proposed `change`. Runs `required` for every proposal made in that unit and the units below it in the last 90 days, twice: with the matrix as it is and with the change. Returns the count on each path before and after, the count for each resolved approver before and after, and the proposals that would have had no approver. It changes nothing. It reads stored checks. It does not rerun them |
| POST | `/admin/authority/changes` | Platform Admin or `compliance` for the root. An Org Unit Admin for their units | Body `organization_id`, `change`, `reason`. Validates against the schema. Refuses with `400` and the code `authority_loosens` a signal tier below the built-in tier and a row whose path is below the built-in path for its tier. Stores a `Pending` change. Applies nothing |
| GET | `/admin/authority/changes` | The same | Pending and past changes, with who proposed and who decided |
| POST | `/admin/authority/changes/{id}/decide` | A different person with the right to propose a change for that unit or a unit above it | Body `decision` (`confirm` or `reject`), `note`. The proposer gets `403`. On confirm, in one transaction: ledger `AuthorityChanged` holding the unit, the change, and both people, then the rows |

A unit row that is looser than the institution's is not an error. It is stored, has no effect, and the screen says so: "The institution already requires more."

Frontend: `admin/Authority.tsx`, a section named **Decision authority**.

- A unit picker, using the org tree control from `organization.md`.
- A table with one row per tier: what raises it (the checks, in sentences), what it needs (the path, in a sentence, with the approver's position and the people it resolves to today), and where that comes from (built in, the institution, or a named unit).
- `Change` opens a form for one tier or one check. `See the effect` calls the impact route and shows the before and after counts and the list of people whose load changes. `Propose change` is disabled until the effect has been shown for the current form values.
- Pending changes, with `Confirm` and `Reject` for a person who is not the proposer.

Tests.

1. A change proposed by one Platform Admin is not in force until another confirms it. The proposer's confirm is `403`. The setup identity can confirm.
2. An Org Unit Admin of Physics can propose for Physics and cannot propose for Chemistry or for the root.
3. A change that would set `sensitivity` to `Low` is `400 authority_loosens`.
4. The impact route, on 200 seeded proposals: the counts before match the stored paths, the counts after match a hand-computed answer, and no row in any table changed.
5. The impact route names a proposal that would have had no approver when the proposed position is vacant in that unit.
6. One `AuthorityChanged` ledger entry per confirmed change, written before the rows. Make the ledger write fail and check that no row changed.
7. Web: `Propose change` is disabled until the effect is shown, and enabled after. Changing a form value disables it again.
8. Web: a looser unit row shows "The institution already requires more."

## Phase 3 — the builder is told why, and what would change it

1. Every answer that returns a proposal, on the web and in the tools, carries `authority`: the tier, the path as a sentence, the people who can decide by name with `via`, and the reasons. A reason whose row or signal names a library rule carries that rule's address, when `policy-library.md` phase 4 has run. When it has not, the reason carries the OSCAL control id and title.
2. A remedy for each raised check, from a fixed table beside the sentences: what the builder could change to avoid raising it. For `page_sensitivity`: "Remove the sensitive field from the page's read list." A check with no honest remedy has none. Do not invent one.
3. The proposal tool from `mcp-apps.md` phase 4 gains a `dry_run` argument, if it has none. A dry run runs the checks and `required`, stores nothing, and returns the same `authority` object. An agent uses it to tell a person what a change will need before proposing it.
4. `GET /me/proposals/to-decide` lists the pending proposals the caller may approve under the current matrix, on every path, across workspaces. When `workflows.md` phase 5 has run, each is also a My Work item of kind `review`. When it has not, say so in the pull request.
5. Web: the proposal view shows a panel, **Who decides**, with the sentence, the names, and each reason as a line with its link. A proposal with `no_approver` says who to contact: the Org Unit Admins of the unit.

Scenarios, added to the suite from `agent-coverage.md`.

1. "Add a date of birth field to the advising page." The dry run answers Moderate, a second owner, and names the reason and the remedy.
2. The same request in a unit whose matrix sends Moderate to the privacy officer names that position and its holder.
3. A proposal waiting on a vacant position tells the agent it is unassigned and who to contact.
4. A person who may decide lists their proposals with the tool and sees the reasons.

Tests.

1. A dry run stores no proposal and writes no ledger entry. Count rows before and after.
2. The names in `authority` are the people who then succeed at deciding. One test drives both from the same fixture.
3. Every flagged check has a sentence, and a remedy or an explicit `None`. The test iterates the list.
4. `to-decide` for a privacy officer lists proposals from workspaces they are not a member of, and shows no record values.
5. Web: the panel renders the sentence, the names, and a link for a reason that has a library rule.

## Phase 4 — a large institution, simulated

The question this phase answers: with this model, in an institution of real size, how much lands on whom, and what gets stuck? It answers with numbers. It does not predict how long people take.

`crates/scaffoldry-core/tests/authority_scale_test.rs`. Pure. No database. It calls `required` and `resolve_approvers` only.

1. Build an institution from a fixed seed with a small linear congruential generator written in the test. Do not add a random number crate. 5,000 people. 200 units in four levels. 400 workspaces, each with one to four owners. Positions: a chair for each department, a dean for each college, a privacy officer and a records officer at the root.
2. Generate 10,000 proposals over 13 simulated weeks. 70% raise no check. 20% raise one flagged check. 10% raise two or more. Proposers are drawn from workspace members, and 30% of them are owners.
3. Run four matrices: the built-in defaults; Moderate to the department chair; High to the privacy officer at the root; and the second and third together.
4. Run each matrix three ways: everyone present; 5% of positions vacant; 10% of holders away with a delegate for two of the weeks.
5. For each of the twelve runs, compute: the share of proposals on each path; the share applied with no second person; the number with no approver; for each approver, items per week, with the median, the 95th percentile, and the largest; and the number of proposals where the only resolved approver is the proposer.

Write the results to `docs/measurements/authority-simulation.md` as one table per matrix, with the seed, the sizes, and the date. State at the top what the simulation does not measure: waiting time, and whether people find the screens clear.

Limits the test asserts.

1. All twelve runs finish in under 5 seconds in total. Record the time.
2. With everyone present, no proposal has no approver under any matrix.
3. Under the built-in defaults, at least 60% of proposals need no second person.
4. No run applies a proposal with a raised flagged check on the `Direct` path.
5. With High sent to one privacy officer, the test does not assert a limit on that person's load. It reports it. If the largest week is above 50 items, the report says in its first paragraph that one position is a bottleneck under that matrix, and names the delegation or the unit-level row that would relieve it.

Then the same question on real data. `GET /admin/authority/load` returns, for the last 90 days, the same measures from stored proposals and the current matrix: paths, items per approver per week, unassigned, and the median and 95th percentile of the time from proposal to decision. Platform Admin or `compliance`. One SQL statement for each measure. Add a **Load** tab to the Decision authority section.

Tests.

1. The five limits above.
2. The report file is produced, names its seed, and a second run with the same seed produces the same numbers.
3. The load route on 500 seeded decided proposals returns hand-computed medians.
4. The load route shows no record value and no proposal content, only counts, names of approvers, and times.

## How to prompt the build agent

```
Read docs/plans/authority.md, docs/plans/approvers.md, and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
required() is pure. It does no I/O.
With both tables empty, every existing test passes unchanged.
Nothing lowers the built-in tier or the built-in path. A raised flagged check always needs a person other than the proposer.
A proposal with no approver waits. It never falls back to a looser path.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| A department will configure its way out of review. | It cannot. The function takes the highest tier and the strictest path on the way to the root, and the built-in floor is a constant in the source. A looser unit row is stored and has no effect. Phase 1 test 4 proves it. |
| One compliance officer will become the bottleneck for the whole university. | Phase 4 measures exactly that before anyone is affected, on a simulated institution and then on the institution's own last 90 days. The impact screen shows each person's load before a change is proposed. Delegation from `approvers.md` and unit-level rows are the relief, and the report names them. |
| A risk tier computed by software is a guess. | It is not a score. It is the highest tier among named checks that either were raised or were not. Each check is deterministic and has a sentence. A reviewer can read why a change is Moderate. |
| Nobody will understand why their change is waiting. | The proposal says who decides, by name, why, with a link to the policy when the library has one, and what would avoid the check. An agent can ask before proposing, with a dry run. |
| A matrix change by one administrator could open everything up. | Nothing can go below the floor. Above it, every change needs a second person and is a ledger entry holding both names. |
| Tightening the matrix will strand proposals in flight. | The requirement is recomputed at the decision. A proposal that now needs an approver waits for one, and shows as Unassigned if none resolves. Nothing already approved is reopened. |
| A vacant position will silently approve, or silently block. | Neither. The proposal stays pending, is flagged `no_approver`, appears in the console, and the builder is told who to contact. |
| Three tiers are too coarse. | They are the levels the institution's security office already uses, from FIPS 199. A unit can raise any single check, so the grain is the check, not the tier. More tiers can be added to a closed list. Fewer cannot be taken back. |
| Committees and two signatures are missing. | They are out of scope here and named: they need the approvals table from `workflows.md` phase 1 and the votes from `process-v2.md` phase 4. The path list is closed and has room for one more. |
| The simulation is made up. | It is, and its report says so in the first line. It answers what load and what gaps the model produces at a stated size. The load route then answers the same question from real decisions, including how long they took. |
