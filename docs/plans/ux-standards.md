# UX standards — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`.

This is an enterprise system. A department chair approves a student's admission in it. A compliance officer proves to an auditor what happened. A screen that looks unfinished, hides an error, or leaves a person guessing is a defect, not a polish item.

This brief sets one kit of components and a short list of rules. Every screen in `docs/plans/admin-console.md`, `guards.md`, and `approvers.md` is built from the kit. The existing desk is moved onto it in phase 3.

## Standard

| Standard | Use |
| --- | --- |
| WCAG 2.2 level AA | The target for every screen. Public institutions are held to a WCAG standard, and procurement asks for a VPAT |
| WAI-ARIA Authoring Practices | The keyboard and role pattern for dialog, tabs, table, and live regions |

Automated accessibility scanning needs `axe-core`, which is MPL-2.0. That is outside the license whitelist in `AGENTS.md`. It is an open decision in the README. Until it is settled, accessibility is tested with Testing Library role and name queries, keyboard tests, and the checklist in phase 1.

## What exists today

| Fact | Where |
| --- | --- |
| Components are written per screen. Tables, modals, toasts, and confirm steps are each different | `apps/web/src/*.tsx` |
| Failed API calls end in `.catch(() => {})`. The person sees nothing | `AdminDesk.tsx` |
| Errors reach the screen as a raw `error` string from the server. There is no code and no next step | `api.ts`, `service/mod.rs` |
| Hard-coded sample data is shown as if it were live | `AdminDesk.tsx`, `ManifestRenderer.tsx` |
| `@tanstack/react-table` is installed and unused | `package.json` |

## Out of scope

- A visual redesign, new colors, or a new logo. Use the existing Tailwind tokens and dark mode.
- A component library package, Storybook, or a design token pipeline.
- Internationalization. Write messages as plain English strings in one place per component, so a later pass can find them.
- Mobile layouts below 360 px. Tablets and up only. The desk already assumes a wide screen.

## The rules

1. **Nothing is silent.** Every asynchronous action has four visible states: working, done, empty, and failed. Failed always shows the reason and a way to retry or fix it.
2. **Nothing is faked.** No screen shows sample data as live data. If data cannot be loaded, say so.
3. **Say what will happen.** A button that changes something is a verb and a noun: `Hold Alex Smith`, not `Submit`. Before the change, a sentence states the effect: `Alex Smith will not be able to sign in. This is recorded in the audit ledger.`
4. **Reasons are asked for once, in place.** An action that needs a reason shows the field next to the button. It is never a browser prompt.
5. **No browser dialogs.** No `window.alert`, `window.confirm`, or `window.prompt`.
6. **Color is never the only signal.** A status has text, and an icon where space allows.
7. **Everything works from the keyboard.** Tab order follows reading order. Focus is always visible. A dialog traps focus, closes on Escape, and returns focus to what opened it.
8. **Names, not ids.** Show a person's name with their address, a unit's name with its code, a control's title with its id. An id alone is never the label.
9. **Plain words.** One idea per sentence. Name the actor and the action. No internal terms such as `eppn`, `affiliation`, or `Cedar` without a plain label beside them on first use in a panel.
10. **Consistent places.** A list is a table with search on the left and filters beside it. A row opens a drawer on the right. Actions live in the drawer header. Creation is a button above the table that opens a drawer.

## Phase 1 — the kit

Create `apps/web/src/ui/`. One file per component, one `index.ts`. Use Tailwind classes already in the project. Add no dependency.

| Component | Contract |
| --- | --- |
| `PageHeader` | Title, one-sentence description, and an actions slot. Renders the page's one `h1` |
| `DataTable` | Built on `@tanstack/react-table`. Props: `columns`, `load(cursor, search, filters) => Promise<{ rows, next_cursor }>`, `rowKey`, `onRowOpen`, `filters` slot, `emptyState`. Shows skeleton rows while loading, `ErrorState` on failure, `Load more` while `next_cursor` is set, a search box that reloads from the first page, and `Export CSV` of the loaded rows. It never sorts or filters rows itself. A column may declare `sortKey`. The header then shows a sort button that calls `load` with that key. Rows are reachable by keyboard and open on Enter |
| `Drawer` | Right-side panel. Focus trap, Escape to close, focus returns to the opener, `aria-labelledby` the title. Header has the title and an actions slot |
| `ConfirmAction` | Props: `verb`, `target`, `consequence`, `audited` (shows the ledger note), `requireReason`, `onConfirm`. Shows the consequence sentence, the reason field when required, and a primary button that stays disabled until the form is valid. While working it shows a spinner and disables itself. A failure shows `ErrorState` inline and keeps the form |
| `EmptyState` | A heading, one sentence, and at most one action |
| `ErrorState` | The message, what to do next when known, a `Retry` button when a retry is possible, and a `Details` disclosure with the status and code |
| `Skeleton` | Placeholder rows and blocks. Respects `prefers-reduced-motion` |
| `Banner` | `info`, `warning`, `error`. Icon plus text. `role="status"` or `role="alert"` by kind |
| `StatusBadge` | Text plus an icon. A fixed set of tones: `ok`, `warn`, `bad`, `neutral` |
| `FormField` | Label, hint, error, required marker. Wires `id`, `aria-describedby`, and `aria-invalid` |
| `Tabs` | `tablist` with arrow-key movement and `aria-selected` |
| `Toast` | A single live region. Success is polite. Failure is assertive and stays until dismissed |
| `ControlLabel`, `UnitLabel`, `PersonLabel` | One-line renderers for a control, a unit, a person. Text only. They take an id and a lookup map and fall back to the id plus the words `unknown` |
| `useAsync` | `data`, `error`, `loading`, `reload` for one promise-returning function |

`docs/ux-checklist.md`: one page, a checklist a reviewer ticks for every screen: the four states, keyboard path, focus return, reason and consequence text, names not ids, no sample data, no browser dialog, dark mode.

Tests, in `apps/web/src/test/ui-kit.test.tsx`, with Testing Library:

1. `DataTable`: shows skeleton rows, then rows. `Load more` calls `load` with the cursor. A rejected `load` shows `ErrorState`, and `Retry` calls it again. An empty result shows the `emptyState`.
2. `DataTable` does not reorder rows that `load` returned.
3. `Drawer`: opens with focus inside, Tab stays inside, Escape closes, focus returns to the button that opened it.
4. `ConfirmAction`: the button is disabled until the reason has text. A rejected `onConfirm` shows the error and keeps the reason. A resolved one calls the success handler once.
5. `Toast`: a failure has `role="alert"`. A success has `role="status"`.
6. `StatusBadge` text is present in the accessible name for every tone.
7. `FormField` error sets `aria-invalid` and is named by the input's `aria-describedby`.

## Phase 2 — errors that explain

Needs `guards.md` phase 1, because a denial names the policy that made it.

Backend. `ServiceError`'s JSON body becomes:

```json
{ "error": "plain sentence", "code": "forbidden", "policy": { "id": "dept-boundary", "description": "..." }, "fields": { "reason": "required" }, "reason": "why", "remedy": "what to do" }
```

`code` is always present and comes from a closed list: `unauthorized`, `forbidden`, `not_found`, `bad_request`, `version_conflict`, `no_approver`, `too_large`, `internal`, and the codes that later briefs add: `job_failed`, `index_building`, `calculating`, `quota_exceeded`, `rate_limited`, `undo_conflict`, `field_hidden`, `field_read_only`, `constraint_violated`, `file_too_large`, `file_type_blocked`, `scanner_unavailable`, `master_key_missing`, `token_scope`, `read_only_source`, `view_as_read_only`, `share_not_allowed`, `archived`. Each brief adds its code to this list in the same change that introduces it. `reason` and `remedy` are optional sentences from access rules. `policy` appears on a denial and holds the `@id` and `@description` of the policy that decided. `fields` appears on `bad_request` and names the bad inputs. Do the same for the handlers that build error JSON by hand in `routes/*.rs`. Search for `json!({"error"` and replace each with `ServiceError`.

Frontend.

1. `ApiError` carries `status`, `code`, `policy`, and `fields`.
2. One function, `explainError(e: ApiError): { message: string; next?: string }`, in `apps/web/src/ui/explainError.ts`. Every `ErrorState` and `Toast` failure uses it. Messages:

   | `code` | message | next |
   | --- | --- | --- |
   | `unauthorized` | `Your session has ended.` | `Sign in again.` |
   | `forbidden` with `policy` | The policy's description. | `Ask a workspace owner or an administrator if you need access.` |
   | `forbidden` without `policy` | `You do not have permission to do this.` | The same |
   | `version_conflict` | `Someone else changed this first.` | `Your view has been refreshed. Review it and try again.` |
   | `no_approver` | `No one is assigned to decide this step.` | `Ask an administrator to assign a position holder.` |
   | `too_large` | The server's sentence. | `Narrow the filter.` |
   | status 0 | `The server is not reachable.` | `Check the connection and try again.` |
   | other | The server's sentence. | The body's `remedy` when present |

A code added by a later brief gets a row here in the same change. Until it has one it shows the server's sentence and the `remedy`.

Tests.

1. Every `ServiceError` variant serializes with a `code` from the closed list. A test iterates the variants.
2. A Cedar denial's body holds a `policy.id` and a non-empty `description`.
3. `grep -rn 'json!({"error"' crates/scaffoldry-server/src` prints nothing. Paste the output.
4. `explainError` table test: one case per row above.
5. A `DataTable` whose `load` rejects with a `forbidden` error carrying a policy shows that policy's description.

## Phase 3 — move the desk onto the kit, and keep it there

1. Replace every `window.alert`, `window.confirm`, and `window.prompt` in `apps/web/src` with `ConfirmAction` or `Toast`.
2. Replace every `.catch(() => {})` and empty `catch {}` in `apps/web/src` with a visible state through `useAsync`, `ErrorState`, or `Toast`. Where the old code kept a sample-data fallback, show a `Banner` that says the data is sample data, or remove the fallback.
3. Replace each hand-written modal in `WorkspaceSettingsModal.tsx`, `CoBuilderStudioModal.tsx`, `AddViewModal.tsx`, `CreateTableModal.tsx`, `LinkedFieldModal.tsx`, and `CsvImportModal.tsx` with `Drawer` or a centered `Modal` variant built on the same focus handling.
4. Extend `governance/scripts/audit-architecture.py` with two rules over `apps/web/src`, skipping `test/` and `ui/`: no call to `alert(`, `confirm(`, or `prompt(`, and no empty catch. The CI step already runs the script. Add the rules only after steps 1 and 2 leave zero violations.
5. Fill in `docs/ux-checklist.md` results for each migrated screen in the session output.

Tests.

1. Run the audit script. Paste that it passes.
2. For each migrated modal, one Testing Library test: it opens with focus inside and Escape closes it.
3. The existing web tests pass. If a test depended on `window.confirm`, change it to click `Confirm` and say so.

## How to prompt Gemini

```
Read docs/plans/ux-standards.md, docs/ux-checklist.md, and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
Do not use window.alert, window.confirm, or window.prompt.
Do not swallow an error. Every failure is shown with its reason and a way forward.
Do not show sample data as live data.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| We will not buy software that fails accessibility review. | The target is WCAG 2.2 AA. The kit builds the keyboard and focus behavior once and tests it. Automated scanning waits on a license decision, and the README says so. A VPAT needs a manual audit that this plan does not replace. |
| Every screen will drift from the kit again. | The audit script fails CI on browser dialogs and swallowed errors. Screens are reviewed against one checklist. |
| Error messages leak policy internals. | A denial shows the policy's description, written for people. The id is in a `Details` disclosure for support. |
| A shared table will not fit every list. | It is server-driven and takes any columns. It does not sort or filter in the browser, so it cannot disagree with the server at a million rows. |
| Asking for a reason on every change is friction. | It is one field, in place, and it is the sentence the auditor reads. Without it the ledger says who and when, and never why. |
