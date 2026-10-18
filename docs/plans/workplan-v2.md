# Workplan v2 — proposed

```
Title:  One workplan that joins the new design notes to the existing briefs and the architecture.
Risk:   notable
Ask:    Approve the order below, or change it. Then I fold it into the README and the architecture document.
IfYes:  The next sessions each build one thing, in an order where every step has what it needs. The new design is not a second plan beside the first.
IfNo:   The README and the new notes keep drifting apart. Some new work repeats old briefs, and some old briefs wait on decisions the new notes already made.
Checks: None. This is a plan. It was built from the README rows and the design notes, not from reading every brief in full.
```

## Where we are

Counted from `docs/plans/README.md` on 2026-10-19.

| Wave | Done | Meaning |
| --- | --- | --- |
| 0 Honest and safe | 15 of 15 | Done |
| 1 The governed core | 42 of 42 | Done |
| 1b Proof, one source of truth, who decides | 4 of 16 | Stateless server, the policy library, and the authority matrix are open |
| 1c Make the desk real | 6 of 10 | The rest is small |
| 2 The data platform | 14 of 28 | Import, export, links, lifecycle, and attachments are open |
| 3 Collaboration and process | 0 of 28 | Not started |
| 4 Ecosystem and reach | 0 of 36 | Not started |
| Total | 81 of 175 | |

Two import and export pull requests (rows 86 and 90) are green and waiting for your merge. One branch, `claude/sharp-newton-gj0ivi`, holds the desk and MCP work since the last merge and has no pull request.

## What the new design builds on

A lot is built already. The new notes extend it. They do not replace it.

| Built | Rows | What the new design does with it |
| --- | --- | --- |
| Row rules as data, one `row_scope`, a SQL and Rust equivalence test, no filter or sort on a hidden column, column rules, view as another user | 52 to 57 | This is the single place access is decided for rows. Audience fields and label requirements extend it. They do not add a second evaluator |
| Positions, holders, vacancies, approver resolution | 21, 23, 24 | Stewards are these positions |
| Sensitivity categories, presets, and detectors as settings | 33 | Label requirements attach to these categories |
| Groups, units, and positions as members, one `member_role` | 52 | Phase 2b of the position note is built on it |
| A job runner, the ledger, caches keyed by version | Wave 1 | The change process and the scheduled notices run on these |

## Where the new notes overlap existing briefs

This is the main reason the plan is not yet cohesive. I wrote parts of the new design without checking them against these briefs.

| New idea | Existing brief | What to do |
| --- | --- | --- |
| Policy as data, an agent proposes and a person reviews, the AI never applies | `policy-library.md` (66 to 68, 71) | **Reuse.** That brief already stores sources, checks citations, and routes review. The policy schemas in the data layer should be documents in that library. Read it fully before building either |
| Who decides a change, a matrix of tiers, a preview of its effect | `authority.md` (69, 70, 72, 73) | **Reuse.** The steward model and the label change process should sit in this matrix. Check that its tiers can express "function steward for definition, unit steward for rows" |
| Findings, a reviewer, authorized stores | `sensitive-content.md` (151 to 153) | **Extend.** The AI review is the detector in that brief. Build those rows before the evaluation page |
| Inbox, email, reminders | `notifications.md` (107 to 113) | **Needed first.** Request access, notice periods, and expiry reminders all need it. None of it is built |
| Real datasets from outside systems | `connections.md` (135 to 140) | **Reconcile.** That brief's dataset is a read-only synced copy. The data layer's dataset is broader. Use one definition: a governed table with an owning function and unit, from a sync or from local entry |
| Guests | `guests.md` (146 to 149) | **Separate.** A guest submits a form. An external collaborator reads a shared dataset. Different feature, and the collaborator is not planned anywhere yet |
| Error hints, scenarios, a tool budget | `agent-coverage.md` (131) | **Same work.** `mcp-guidance.md` phases 2 and 7 are this row, widened |
| Department strings in Cedar | `position-roles.md` phase 4 | **Replaced** by the data layer. Mark it |
| Rows say who may see them | `access-rules.md` "a rule narrows and never grants" | **A decision was made to depart** for audience fields. Record the change in that brief |

## The architecture

`docs/ARCHITECTURE.md` describes six layers and a modular monolith on one machine. The data layer fits that: it is a crate boundary inside the monolith, not a second service. Three things in the document must change when the design is approved:

1. A data layer section. It sits across the access, store, and ledger layers and is the single door to rows.
2. Authorization. The document says Cedar decides at the API boundary. Row rules already decide in Rust, and label and audience checks would too. The document should say which question each engine answers.
3. The MCP section. Add the guidance layers from `mcp-guidance.md`.

## The order

Each step ends with something a person can use or a test that proves it. I give no dates. The only estimate I made before assumed the screens were wired, and I would not repeat it.

| Step | What | Rows and notes | You can see |
| --- | --- | --- | --- |
| 0 | Merge the two import and export pull requests. Open a pull request for the current branch | 86, 90 | The work so far is on `main` |
| 1 | Reconcile the docs. Fold this plan into the README, update the architecture, mark replaced items | Docs only | One plan |
| 2 | Inbox and email | `notifications.md` phases 1 and 2 (107, 108) | A bell and a first email |
| 3 | The data layer foundation: dataset ids, a crate boundary and an audit for the single door, a declared key, the row's unit column | `data-layer.md` phases 1 and 2, and keys | No feature. Tests prove nothing reads rows outside the door |
| 4 | Catalog v1: entries, synthetic samples, search on the desk and in MCP, request access to the steward | `data-layer.md` catalog. MCP guidance phases 2 to 4 alongside | The first visible feature. A person finds a dataset and asks for it |
| 5 | Labels and grants: findings and review, row tags, effective labels, function-by-unit grants, row shares, the evaluation page, a scheduled change with notice | `sensitive-content.md` 151 to 153, `data-layer.md` phases 3, 5, 6, 6b, `position-roles.md` 5a | A steward changes a label safely |
| 6 | Audience fields and conditions, with the performance gate. System users for automations | `data-layer.md` phases 3b and 9 | Advisor notes and a cross-listed course work |
| 7 | Real datasets: file feeds, change proposals, and linking apps to datasets | `connections.md` 135, 136, 139 | A dataset from an outside system |
| 8 | Finish what the desk does not yet do with real rows | Wave 1c leftovers, Wave 2 import, links, and lifecycle | A department can run a real process |

Alongside, not in the path: the policy library and authority matrix (1b), the stateless server rows (62 to 65, needed before more than one instance runs), and the rest of MCP guidance.

**Not scheduled:** Wave 3 and Wave 4, apart from the rows pulled in above. Extra unit tags, run-level tracking of what an automation read, an expedited change path, and standing share policies stay deferred, as in the version 1 cut.

## What I need from you to start

1. **Approve the order**, or move steps. The one I would weigh most is whether the catalog (step 4) should come before labels and grants (step 5). I think it should: it needs less, and it shows pilots something.
2. **The policy library.** Should the data layer's policy documents live in it? I think so, but I have not read the whole brief. If it does not fit, I would rather learn that before building either.
3. **A first pilot.** The first real use should be named: one unit, one dataset, one request path. It decides what step 4 must do first. I do not know the answer.
4. **Docs first.** Step 1 is a half-day of reconciliation with no code. I would do it before step 2. It is the cheapest way to stop the drift.

## Not verified

- I read the README rows and the headers of the briefs named above. I did not read all 41 briefs. The overlaps may be larger than I list.
- I did not check that each older brief's "needs" still hold after the recent changes.
- I have not measured any performance claim in the new notes.
- Order and dependencies come from the README text. A step may need something I missed.
