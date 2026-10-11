# Handoff for the parallel agent (Gemini in Antigravity)

Written by Claude, 2026-10-11. Coordination is only through git, GitHub, and `python3 governance/scripts/next-phase.py`.

## State of main
Everything below is merged to `main` (PRs #61 to #76), each squash-merged after CI went green:
live-data 1-2, admin-console 1-4 (first part), approvers 1-3, guards 2-3, sensitive-content 1, mcp-apps 2-5 incl. 4a,
record-history 1, agent-coverage 1, row-scale 1-2, jobs 1-3, links 1-3.

## Claims right now
- `feat/row-scale-phase-3` is claimed by Claude and in progress. No other claims. No open PRs.
- Your last pushes (admin-console 1 and 3, guards 2, jobs 2, ux-standards 1) are all merged. Nothing of yours is outstanding.

## What can be taken in parallel
`next-phase.py available` lists one phase besides mine: **calc-graph 1** (row 44, one formula evaluator in Rust plus a preview route).
Everything else is blocked on run order. calc-graph 2 needs row-scale 3, which is the phase I hold.

## Overlap to watch if you take calc-graph 1
- Row-scale 3 (mine) stores formulas at write time by calling `scaffoldry_engine::evaluate_formula(expression, &data)` from
  `apply_formulas` in `crates/scaffoldry-server/src/repository.rs`. If calc-graph 1 changes that signature or replaces the
  evaluator, keep `evaluate_formula` (or tell me) so the call still compiles.
- I do not edit `crates/scaffoldry-engine/src/lib.rs`'s formula code in row-scale 3. Files I touch: `repository.rs`,
  `repository_index.rs`, `service/records.rs`, `service/history.rs`, and new test `row_scale_computed_test.rs`.
- Please claim with `next-phase.py claim calc-graph 1` before starting, and branch from the claim.

## Things that changed under you (read before editing these)
- `FieldSpec` gained `indexed`, `link_target_table`, `link_target_app`, `link_cardinality`, `link_related_field`,
  `link_on_delete`, `computed_via`. Build new ones with `FieldSpec::simple`.
- `Lookup`, `Rollup`, `Count` now require `computed_via` naming a `Link` field. The by-value rules are gone.
- Every record write goes through `WriteCtx` (index fields, link specs, computed specs, stored formulas).
  Only `repository.rs` may run `INSERT/UPDATE/DELETE` on `dataset_records` (a scan test enforces this).
- Approving a manifest that adds an indexed field queues a `build_index` job. It no longer copies rows.
- Postgres on port 5433 dies in this container now and then. Run cargo tests one at a time, never concurrently.

## Open items for Johann (not for you to settle)
1. Admin-console phase 0 was marked COMPLETED while its work was not done (issue #51).
2. Jobs: nothing enqueues `recompute_flags` yet. It waits on the rest of admin-console phase 4 (labels).
3. `lifecycle.md` phase 1 owns trash. Links phase 2 added `trash_links` and `restore_record_links`; lifecycle's restore should call it.
4. `link_target_app` is refused until `connections.md` creates dataset apps.
5. Builders (WorkflowBuilder, app builder) are not wired to the server's `submitRule` yet.
6. `get_record` does not fill lookups, because the update tool merges its output back into a write.
7. The web app still computes lookups, rollups and formulas in `computedFields.ts`. The server now stores them.
