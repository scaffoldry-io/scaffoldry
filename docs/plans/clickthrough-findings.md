# Click-through findings — 2026-10-14

A short manual pass through the web desk, against a fresh database with `seed-demo`. Chromium drove the screens. The goal was to see whether a person can use the product. This is not a full test. It covers sign-in, the workspace list, one published app, Datasets, and the admin console's first load.

## What worked

- Sign-in with a pasted token works. The 401s before sign-in are expected.
- The workspace list and the left rail come from the API and render with no console errors.
- The API allows the dev origin (CORS preflight returns 200).
- `seed-demo` and `setup-token` both run and produce a usable state.

## What does not work

These are gaps between what the screen says and what the system does. Items 1 and 2 are the most serious.

| # | Finding | Evidence | Where |
| --- | --- | --- | --- |
| 1 | The published-app screen is a mock-up. Its four records are a hardcoded array. "New Proposal" and Kanban moves change React state only. Nothing is sent to the API, and a reload loses them | `PublishedAppView.tsx`: `useState<RecordItem[]>([...APP-101...])`, `handleCreateRecord`, `handleCardStageMove`. No write call seen in the browser. The backend has no app `cs-ai-benchmarks` | `apps/web/src/PublishedAppView.tsx` |
| 2 | After "New Proposal" the screen says "Record APP-105 committed to sovereign ledger (SHA-256 block attestation)". That is false. No ledger entry is written | `handleCreateRecord` sets that message after a local state change | same file |
| 3 | Dashboard figures are made up. "+12% this term", "88% target met", and "Target: 7 days" are fixed strings | `PublishedAppView.tsx`, stat-metric `trend_text` | same file |
| 4 | "Approved Funding $2,290,000" is the sum of every budget, including the two proposals still under review. It has no status filter | The tile config has `aggregation: sum`, `target_field: budget`, and no filter | same file |
| 5 | The header always shows "MC Prof. Curie, faculty", whoever is signed in | Hardcoded in the header JSX. I was signed in as the setup identity | same file |
| 6 | The budget field has `min=1000 step=5000`, so 100,000 is refused as invalid. The valid values are 96,000 and 101,000 | Browser validation message | same file |
| 7 | The Datasets screen is static. Its datasets, row counts ("Total Live Rows: 840"), and sample rows are written in the component. It made no API call, and the API's `/datasets` returns different data | `DatasetExplorer.tsx` has no API import. A click made no `/datasets` call | `apps/web/src/DatasetExplorer.tsx` |
| 8 | The README's two-command dev setup cannot connect. `vite.config.ts` has no proxy, and the app calls `/api/v1` on its own origin. It works only with `VITE_API_BASE_URL` set | Vite returned its HTML for `/api/v1/me`. Setting the variable fixed it | `README.md`, `apps/web/vite.config.ts` |

## Fixed since

Items 2, 3, and 5 are fixed. The published-app screen no longer claims a ledger entry. It says the record was "added to this preview only. It is not saved." The made-up trend figures and the hardcoded "Prof. Curie" header are gone. The "Live" badge and the "Live Dataset" chip now read "Sample data". Two tests had locked in the old claims ("committed to sovereign ledger" and "Live"). They now assert the honest wording. The web suite passes (245 tests) and the typecheck is clean.

Items 6 and 8 are fixed too. The budget field's step is now 1,000, so 100,000 is valid. Vite forwards `/api` to `http://127.0.0.1:8080`, so the README's two commands connect without `VITE_API_BASE_URL`. Both have tests. I checked the proxy rule in the config only. I did not run the two servers together after the change.

The `/admin` identity question is resolved. The desk asked the server who a token belongs to only at sign-in. After a reload the stored token came back, but the person shown reset to the built-in "Dr. Sarah Connor, faculty". The screen then displayed and gated as that person while API calls carried the real token. The desk now asks the server on load and shows that person. A 401 ends the session. A network failure does not. Three tests cover it. The "403 Forbidden" I saw is consistent with the desk gating on the wrong person. I did not confirm that separately in a browser.

Item 1 has its foundation. `api.ts` now has list, get, create, update, and delete calls for an app's records, and `useAppRecords` loads them, adds created rows, saves with the version it read, and on a 409 reloads the row, shows "Someone else saved this record. Reloaded.", and does not send the typed value again. It invents no rows: before a load finishes or when it fails there are none. Five tests cover it. No screen uses it yet, so the published-app screen is still the mock-up. Switching that screen, and `MultiViewWorkspace`, to the hook is the next slice. It rewrites tests that assert the mock-up's behaviour, and it needs a decision on what the published-app screen shows for an app of any shape.

Items 1 and 4 are done, by replacing the screen. The published-app screen no longer holds a record array. It is now a thin wrapper around `MultiViewWorkspace`, which builds its columns from the app's own tables and fields, so it works for an app of any shape. `MultiViewWorkspace` no longer seeds or generates rows. It shows what the server holds, says "Loading records…" while it loads, shows an error with Retry when the load fails, adds a row or a form submission as a real record, and saves a cell with the version it read. On a 409 it reloads the row and says "Someone else saved this record. Reloaded." The proposals mock-up, with its hardcoded investigators, fake trend figures, and the "Approved Funding" tile, is deleted. That settles item 4, because the tile no longer exists. Tests that asserted the mock-up were removed. New tests, with a stubbed server, cover stored rows with none invented, a load failure, adding a row, a cell edit with its version, the conflict notice, and the wrapper. I kept the Public Intake Form button, because a test guards that real feature.

Not verified: I did not click through this in a browser. The tests use a stubbed server. The kanban, calendar, and gallery views were not tested against real rows, and the kanban view still moves cards by writing a field named `status`, which an app may not have. Deleting a record has no button on this screen. Link and lookup fields were not checked.

Item 7 is fixed. The Datasets screen reads `GET /datasets` and each dataset's relationships. It shows a loading line, an error with Retry, or "No datasets are published yet". The department filter lists the server's departments. Three tests cover it, and the UI smoke check opens the screen against a real server. The app studio's dataset picker (`CoBuilderStudioModal.tsx`) reads the server too, and the built-in samples are deleted. A new guard test (`no-built-in-sample-data.test.ts`) found one more: the app builder started every app with an invented automation rule, "Honors Research Grant Notification". It now starts empty. Automation rules in the builder are still held in the browser only.

Still open: the rest of the desk's built-in sample data, such as the seeded app and workspace lists in `AdminDesk.tsx`. Item 1 is the real fix. The label only stops the screen from misleading people.

## Found in passing

- **The setup identity is hardcoded as `central_admin`.** `resolve_user` returns `affiliation: "central_admin"` for `setup@scaffoldry.local`. This answers the first-boot question in `position-roles.md` phase 1. That phase must give the setup identity a role, or exempt it, or first boot loses its admin.
- **The Postgres clusters stopped between turns, twice.** Tests need `pg_ctlcluster 16 scaffoldry start` (port 5433). The development docs should say so.

## Not verified

- **A fresh load of `/admin` showed "Dr. Sarah Connor, Faculty" and a "403 Forbidden", though I had signed in as the setup identity.** I did not find the cause. It may be a session that does not survive a full page load, or a fallback to a default persona. If it falls back to a persona, that is worse than a login screen. A reload test would settle it. I stopped before running it.
- Everything not named above: the Builder, Kanban, forms, workflows, proposals, and every admin tab after the first load. I did not open them.
- Whether the backend routes that these screens should call do what the tests say, from the browser. The API suite is green (588 tests). The screens above do not use it.

## What this changes in the plan

- The core loop, "open an app, add a record, see it saved and in the ledger", has a working backend and no working screen. The brief that fixes it is `live-data.md` (row 159, "The desk reads records and the directory from the API"). It sits in Wave 4, which has 0 of 36 done. It should move to the front.
- Items 2, 3, 4, and 5 are honesty bugs, not missing features. Remove the false ledger message and the made-up figures now, before the real wiring lands. A governance product must not claim a ledger entry it did not write.
- Add to `live-data.md`, or to a new brief: a rule that no screen may show sample data without a visible "sample" label, and a test that fails on a hardcoded record array in a screen component.
- Add item 8 to `foundation.md` or the README fix list: a Vite proxy for `/api` in development.
- Resolve the `/admin` identity question before any release.

The earlier schedule estimate (6 to 10 weeks for the whole plan) is unchanged. It assumed the screens were wired. For the core loop, they are not, so the estimate for a usable first pilot should start from `live-data.md`, not from the plan's count of completed briefs.

## Fixed after the audit existed

The audit found two faults that no one had listed. `ProcessDesk` sent its requests with no token, so the Decisions panel always got 401 and said "No decisions waiting". It now sends the signed-in token on the list and on decide, with a test for each. The header "+ New App" drew its plus twice, and is fixed with a guard test that runs with the normal suite. The audit's remaining findings are in `docs/plans/README.md` as rows 1c-7 to 1c-9.
