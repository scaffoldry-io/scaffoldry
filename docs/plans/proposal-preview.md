# Proposal preview — implementation brief

> **Status: Open**

Hand this file to the build agent. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `mcp-apps.md` phase 4a. Phase 2 needs `mcp-apps.md` phase 7. Phase 3 needs `access-model.md` phase 5.

A reviewer approves a change to an app from a list of what changes and the checks it raised. For a new field that is enough. For a new page, a new form, or a changed layout it is not. The reviewer is asked to approve something they cannot see.

This brief lets the proposer and the people who may decide **open the proposed app before it is live**, and makes an approval apply **only to exactly what was reviewed**.

## Standard

NIST SP 800-53 CM-3 (configuration change control) and CM-4 (impact analysis): a change is reviewed and its effect understood before it is approved. CM-3(2): changes are tested before they are put into operation. Record these in the first commit message. No standard defines how an application change is previewed.

## Decisions already made

| Question | Answer |
| --- | --- |
| What does an approval cover? | One digest: the SHA-256 of the proposal's canonical content, which is the manifest, every page source, and every rule. The approver's request must name the digest they saw. What goes live is the content with that digest |
| Who may open a preview? | The proposer, the people who may decide the proposal at that moment, and a Platform Admin |
| What data does a preview show? | Live records, read-only, through the stricter of the live rules and the proposed rules. A field the proposal adds is empty. A field it removes is hidden |
| Can a preview write? | Not in phases 1 and 2. Phase 3 adds a separate trial copy filled with synthetic records, which can be written and is thrown away |
| How long does a preview last? | A preview session lasts 30 minutes, as view-as does. It ends early when the proposal is decided, withdrawn or superseded |
| Is opening a preview recorded? | Yes. A ledger entry `ProposalPreviewed` with the proposal id, the digest and the person. When `access-model.md` phase 8 has run, the reads also write receipts |

## What exists today

| Fact | Where |
| --- | --- |
| A proposal stores a manifest, page sources and rules, with checks. Approval writes a ledger entry holding the SHA-256 of the manifest | `mcp-apps.md` phases 4 and 4a, `service/proposals.rs` |
| A proposal made against an older live version is superseded at approval | `repository_proposals.rs` `approve_proposal` |
| View-as gives a read-only, 30-minute, one-app session with a banner | `access-rules.md` phase 6, `service/view_as.rs` |
| Custom pages render in a sandboxed frame from stored source | `mcp-apps.md` phases 6 and 7, planned |
| The proposal view lists changes and checks as text | `apps/web/src` proposal screens |

## Out of scope

- A second environment, a copy of the database, or a staging server.
- Previewing a change to institutional policy. `admin-console.md` phase 5 has test cases and a simulator for that.
- Writes to live records from a preview, at any phase.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. A preview never writes a live record and never changes a live app. A test in every phase tries.
4. The rules a preview applies are never looser than the live rules for the same person.
5. Add every new error code to the closed list in `ux-standards.md` and to `explainError`.
6. Use the next free migration number on `main`.

## Phase 1 — an approval applies to exactly what was reviewed

1. `proposal_digest(proposal)`: the SHA-256 of a canonical JSON object with the manifest, each page source keyed by page id, and the rules, with keys sorted and no insignificant white space. One function. Store it on the proposal when it is made.
2. `GET /proposals/{id}` and the proposal tools return `digest`.
3. `decide` with `approve` requires `digest`. When it differs from the stored digest, the answer is `409 proposal_changed` and nothing is applied. A proposal's content cannot change after it is made, so this also catches a client that reviewed something else.
4. The ledger entry `AppPublished`, or the entry for a rule change, holds the full digest beside the manifest hash it holds today.
5. `submit_direct`, the direct path for an owner's clean change, computes the digest and passes it to `decide` itself.

Tests.

1. Approving with the right digest applies the change. Approving with a different digest is `409 proposal_changed` and the live app is unchanged.
2. The digest is the same for the same content in a different key order, and different when one character of a page source changes.
3. The ledger entry holds the digest.
4. The scenario suite's approval scenarios pass with the digest added.

## Phase 2 — open the proposed app, read-only

1. `POST /proposals/{id}/preview` by a permitted person mints a `preview` token: one app, 30 minutes, read-only, bound to the proposal and its digest, with `original_admin` set to the caller. It writes `ProposalPreviewed` to the ledger before it returns. Any other person gets `403`.
2. Under a `preview` token, every read of the app uses the proposed manifest, views and pages, and these rules:
   - Rows: the live row scope and the proposed row scope together, with `RowScope::and`.
   - Fields: hidden if the live rules or the proposed rules hide them. A field only the proposal adds is shown with no values.
   - The caller's own rights apply as always.
3. Every write is refused with `preview_read_only`, as the view-as gate refuses writes today. Reuse that gate. Do not write a second one.
4. Custom pages in the proposal render from the proposal's page sources. Their `page_call` reads go through the same rules. Their writes are refused.
5. The token stops working when the proposal is decided, withdrawn or superseded.
6. Desk: the proposal view gains `Preview`. It opens the proposed app with a banner: "Previewing a proposed change. Read-only. Nothing here is live." The banner names the proposal and has `Stop preview`.
7. MCP: the tool `preview_proposal` returns the preview's app description and, for each custom page, the rendered page, so an agent can show the person what they are about to propose.

Tests.

1. The proposer and a person who may decide can preview. Another workspace member gets `403`.
2. A preview of a proposal that adds a field shows the field empty. One that removes a field hides it.
3. A preview of a proposal that loosens a row rule shows no row the person could not see live.
4. Every write under a preview token is refused and no record changes.
5. A preview token stops working after the proposal is approved, and after 30 minutes.
6. Opening a preview writes one ledger entry.

## Phase 3 — a trial copy to try writes

A reviewer sometimes needs to try the form, not only look at it.

1. `POST /proposals/{id}/trial` creates a trial app: the proposed manifest, in a synthetic workspace under the synthetic subtree from `access-model.md` phase 5, filled with synthetic records generated for the proposed tables. It never holds a live record. The caller is its owner.
2. Limits: at most three open trials per person, 500 records per trial, and each trial is destroyed through `purge::destroy` seven days after it was last opened, or when its proposal is decided. Put the numbers in settings `preview.trials_per_person`, `preview.trial_records` and `preview.trial_days`.
3. A trial app is marked synthetic. The access model keeps its people and grants inside the synthetic subtree.
4. The proposal view shows `Try it` when a trial is allowed.

Tests.

1. A trial app has the proposed tables and synthetic records, and no live record.
2. Writing in a trial changes nothing outside it.
3. A fourth trial for the same person is refused with `429`.
4. A trial is destroyed when its proposal is approved.

## How to prompt the build agent

```
Read docs/plans/proposal-preview.md, docs/plans/mcp-apps.md, docs/plans/ux-standards.md, and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
A preview never writes a live record. Its rules are never looser than the live rules.
An approval names the digest it reviewed.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| A reviewer could approve something other than what they saw. | The approval names the digest of what was reviewed. Anything else is refused and nothing is applied. |
| A preview will show data a person should not see. | It uses the stricter of the live rules and the proposed rules for that person. A loosened rule shows nothing new until it is approved. |
| A preview will change live data. | Every write under a preview token is refused by the same gate as view-as. Phase 3's trial copy holds only synthetic records. |
| Trial copies will pile up. | Three per person, 500 records each, destroyed seven days after last use or when the proposal is decided. |
| This needs a staging environment. | It does not. App definitions are data, separate from records, so the proposed definition is rendered against the live records with stricter rules. |
