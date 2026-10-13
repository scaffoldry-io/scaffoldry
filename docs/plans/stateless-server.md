# One source of truth — implementation brief

> **Status: Open**

Hand this file to the build agent. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs nothing that is still open. Each later phase needs the one before it.

An institution must be able to run two copies of the server at once. That is what makes a restart, an upgrade, or a failed machine invisible to the people using it. It is also what a hosted offering needs.

Today it cannot. The running process keeps its own copy of almost everything: users, organizations, appointments, workspaces, guards, app definitions, process instances, the ledger, tokens, and settings. It loads them at boot and answers from memory. PostgreSQL is written to, and read back only on the next boot. A second copy of the server would never see what the first one wrote.

This brief makes PostgreSQL the only source of truth. The server keeps nothing it cannot rebuild from a query. It adds no feature.

## Standard

No standard defines this. Record the search in the first commit message: the Twelve-Factor App, factor VI ("processes are stateless and share nothing") states the rule this brief adopts. NIST SP 800-53 CP-10 (system recovery) and SC-24 (fail in known state) are the controls it serves.

## Decisions already made

| Question | Answer |
| --- | --- |
| What is the source of truth? | PostgreSQL. Always. A value in memory is a cache of rows, never the record itself |
| May the server cache anything? | Only a collection on the closed list in phase 1, and only through the one mechanism in phase 1. Nothing else is held between requests |
| How does one instance learn that another wrote? | A counter row per cached collection, bumped in the writer's transaction. Every request reads the counters in one statement before it does anything else |
| Is `LISTEN` and `NOTIFY` used? | No. A missed notice leaves a stale cache with no error. `realtime.md` may use it to tell browsers. It is never how the server decides what is true |
| What is never cached? | Sessions, tokens, people, groups, appointments, process instances, the ledger, proposals, jobs, and records. They are large, or they change in bursts, or a stale read is a security fault |
| Does the server start without PostgreSQL? | No. It also does not start when any boot read fails |
| Is there a memory-only mode for tests? | No. A test uses a real database of its own |
| Is a second instance supported for writes? | Yes. Every write that depends on what it read takes a row lock or a version check in PostgreSQL. No lock lives in the process |

## What exists today

| Fact | Where |
| --- | --- |
| `ServerState` holds twenty collections behind `RwLock`: `users`, `groups`, `workspaces`, `collaborators`, `guards`, `guard_policy_cache`, `datasets`, `relationships`, `automations`, `process_instances`, `ledger`, `engine`, `sessions`, `api_tokens`, `organizations`, `roles`, `position_types`, `labels`, `settings`, and `memory_records` | `state.rs` |
| Boot loads each with `if let Ok(...)`. A read that fails is skipped. The server starts with that collection empty and reports nothing | `state.rs` `ServerState::new` |
| A write changes memory first and PostgreSQL second. When the second fails, memory keeps a value that was never stored | `state.rs` `persist_organization`, `persist_role`, and the functions beside them |
| An approved proposal is stored, then the new app definition is put into this process's `engine`. Another instance keeps serving the old one | `service/proposals.rs` `decide` |
| The whole ledger is held as a `Vec` and extended after each append | `state.rs`, `service/proposals.rs` |
| Two writes are serialized by a lock in the process: `WRITE_LOCK` and `PRESET_LOCK` | `service/positions.rs`, `routes/settings.rs` |
| `ServerState::in_memory()` builds a state with no repository. Nothing calls it. Twenty-six branches still handle `repository: None` | `state.rs` |
| The integration tests share one database and change `DATABASE_URL` in the process environment. CI runs them one at a time | `tests/`, `.github/workflows/ci.yml` |
| Records are already read from PostgreSQL. Jobs already claim rows with `FOR UPDATE SKIP LOCKED`, and the scheduler already takes an advisory lock | `live-data.md`, `jobs.rs` |
| The only health route is `/healthz`. It does not check the database. The server does not drain requests when it stops | `routes/mod.rs`, `main.rs` |

## Out of scope

- A cache server, a message broker, or a second data store. Do not add Redis.
- An asynchronous database driver or a connection pool crate. The worker threads from `foundation.md` phase 4 stay.
- Read replicas, sharding, or more than one institution in one database.
- A load balancer, a container orchestrator, or deployment files. `foundation.md` phase 11 owns the image.
- Metrics, tracing, and log format. They are a separate brief.
- Presence in `realtime.md`. Who has a stream open is a fact about one instance and may stay in that instance's memory. It is never used to decide access.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. Every phase adds its collections to the two-instance test from phase 1. A phase is not done until a write through instance A is read through instance B on the next request.
4. A write commits in PostgreSQL before memory changes. When the commit fails, memory is not touched and the request fails.
5. Do not put `let _ =` or `if let Ok` in front of a repository call. A failed read is an error the caller sees.
6. Do not add a `static` lock, a `static` map, or a new `RwLock` field to `ServerState`. The scan test in phase 1 fails when one appears.
7. Paste the output of the `grep` named in the phase. It must show no remaining use.

## Phase 1 — the mechanism, an honest boot, and a database per test

Migration `crates/scaffoldry-core/migrations/0062_cache_versions.sql`:

```sql
CREATE TABLE IF NOT EXISTS cache_versions (
    name VARCHAR(32) PRIMARY KEY,
    version BIGINT NOT NULL DEFAULT 0
);
INSERT INTO cache_versions (name) VALUES
    ('settings'), ('labels'), ('position_types'), ('organizations'), ('workspaces'),
    ('guards'), ('datasets'), ('relationships'), ('manifests'), ('rules'), ('policy')
ON CONFLICT DO NOTHING;
```

That list is the closed list of cached collections. A collection that is not on it is read from PostgreSQL every time.

The mechanism, in a new file `crates/scaffoldry-server/src/cache.rs`:

1. `bump(tx, name)` runs `UPDATE cache_versions SET version = version + 1 WHERE name = $1` inside the writer's transaction. It returns an error when no row changed. Every repository function that writes a cached collection calls it before it commits.
2. `ServerState::refresh()` runs `SELECT name, version FROM cache_versions` once. For each name whose version differs from the one this instance holds, it reloads that collection with its existing `list_*` function and then stores the version it read before the reload. A reload that fails returns the error and keeps the old version number, so the next request tries again.
3. One axum middleware calls `refresh()` at the start of every request, before `guard::require_session`. A failure is `503` with the code `storage_unavailable`, which is on the closed list in `ux-standards.md`. Add its row to `explainError`. The job runner calls `refresh()` before each job.
4. `ponytail:` the `UPDATE` holds a row lock until commit, so two writers of one collection run one after the other. That is acceptable for configuration. It is the reason nothing that changes often is on the list.

Move `settings` to the mechanism in this phase, as the proof. `set_platform_setting` writes the row and bumps `settings` in one transaction, then calls `refresh()`. Replace `PRESET_LOCK` in `routes/settings.rs` with `SELECT ... FOR UPDATE` on the settings rows the preset changes.

An honest boot.

1. Replace every `if let Ok(x) = repo.list_...()` in `ServerState::new` with `?`. A boot read that fails returns the error and the server does not start.
2. Delete `ServerState::in_memory()`. Make `repository` a plain `Arc<PostgresRepository>`, not an `Option`. Delete every `None` branch. Run `grep -n "repository: None\|Some(ref repo)\|None =>" crates/scaffoldry-server/src/state.rs` and paste the output.
3. Add `ServerState::connect(database_url: &str)`. `ServerState::new()` reads `DATABASE_URL` and calls it. No test changes the process environment after this phase.

A database per test, in `crates/scaffoldry-server/tests/common/mod.rs`:

1. `TestDb::new()` connects to the server in `DATABASE_URL`, runs `CREATE DATABASE scaffoldry_test_<uuid>`, and returns its URL. `Drop` runs `DROP DATABASE ... WITH (FORCE)`.
2. `TestDb::app()` returns a router on a fresh `ServerState::connect`. `TestDb::second_app()` returns another router, on another `ServerState`, on the same database. That pair is the two-instance test.
3. Move every integration test to `TestDb`. Delete the shared `build_app()` helpers and every `std::env::set_var("DATABASE_URL", ...)`. The test for an unreachable database calls `ServerState::connect` with a bad URL.
4. Name the struct `TestDb` and keep it in `tests/common/`. CI looks for it there and runs `cargo test` without `--test-threads=1` once it exists. Paste three consecutive passing runs of plain `cargo test`.

The scan test, `crates/scaffoldry-server/tests/stateless_scan_test.rs`. It reads the source files as text.

1. Every `RwLock<` or `Mutex<` field of `ServerState` is on an allowed list held in the test. The list starts as today's fields and shrinks in each phase. A field that is not on it fails the test.
2. No file under `crates/scaffoldry-server/src` declares a `static` holding a `Mutex`, an `RwLock`, a `OnceLock`, or a `LazyLock`, outside an allowed list that names `WRITE_LOCK` until phase 2 removes it.

Tests.

1. Two instances: a setting changed through A is returned by B on the next request.
2. `refresh()` runs one statement when nothing changed. Count the statements with a counter on the repository in the test build.
3. A `bump` for a name that is not in the table is an error and the transaction rolls back.
4. A database where `organizations` cannot be read makes `ServerState::connect` return an error. Rename the table in the test, then restore it.
5. Two tests that each create the organization code `PHYS` pass when run at once.
6. Measure and record in the pull request: the added time per request from `refresh()`, as the median of 1,000 calls against the CI database. If it is above 2 ms, stop and report. Do not tune it.

## Phase 2 — people, groups, appointments, sessions, and tokens are queries

These are never cached. A university has tens of thousands of people and a registry sync changes them in bursts. A revoked token must stop working on the next request, on every instance.

1. Add to the repository, each one indexed query: `user_by_eppn`, `users_by_eppns`, `search_users(query, cursor, limit)`, `groups_of(eppn)`, `roles_of(eppn)`, `roles_at(unit_ids)`, `holders_of(unit_ids, position_key)`, `session_by_token_hash`, and `api_token_by_hash`. Reuse a function that already exists. Do not write a second one beside it.
2. Replace every read of `state.users`, `state.groups`, `state.roles`, `state.sessions`, and `state.api_tokens` with a call to one of them. Remove the five fields. Run `grep -rn "\.users\.\|\.groups\.\|\.roles\.\|\.sessions\.\|\.api_tokens\." crates/scaffoldry-server/src` and paste the output.
3. `resolve_approvers` stays a pure function. The code that builds its `ResolveCtx` loads the role rows and users for the units on the path from the record's unit to the root, with `roles_at` and `users_by_eppns`. It does not load every person.
4. The People list in the console pages in SQL with `search_users`. Remove the `ponytail` note in `admin-console.md` phase 3 that says the list is built in memory, and say why in the pull request.
5. `update_token_last_used` stays a single `UPDATE`. It bumps nothing.
6. Replace `WRITE_LOCK` in `service/positions.rs`. An appointment that must not exceed a position type's holder limit runs `SELECT ... FOR UPDATE` on the `position_types` row, counts the holders, and inserts, in one transaction.
7. Add an index only where `EXPLAIN` on a table of 50,000 people shows a sequential scan. Put it in `crates/scaffoldry-core/migrations/0063_people_indexes.sql`. If no index is needed, do not create the file, and say so in the pull request.

Tests.

1. Two instances: a token revoked through A is `401` on B's next request.
2. Two instances: a user put on hold through A cannot sign in through B.
3. Two instances: an appointment made through A makes that person an approver in B's `resolve` answer.
4. Two requests at once, on two instances, each appoint a different person to a position with a limit of one. One succeeds and one is `409`. The table holds one holder.
5. With 50,000 seeded users, `user_by_eppn` and a People page of 50 each run one statement and return in under 20 ms. Record the times.
6. With 50,000 seeded users and 400 units, building a `ResolveCtx` for one record loads fewer than 500 role rows.
7. The scan test's allowed list no longer holds the five fields or `WRITE_LOCK`.

## Phase 3 — workspaces, guards, labels, datasets, and the org tree

These are small and read on almost every request. They are cached through the mechanism.

1. `organizations`, `position_types`, `workspaces` (with their collaborators), `guards`, `labels`, `datasets`, and `relationships`: every repository function that writes one of them calls `bump` with its name before it commits. Every service function that writes one calls the repository first, then `refresh()`. Delete the code that changes the map directly.
2. `collaborators` is reloaded with `workspaces`. Collaborator and member changes bump `workspaces`.
3. `guard_policy_cache` is cleared whenever `guards` reloads. It stays keyed by workspace and guard version, so it is correct without a counter of its own.
4. A write that depends on a value it read uses a version or a row lock in PostgreSQL. Moving an organization unit locks the unit row and checks for a cycle in the same transaction.
5. Run `grep -rn "\.write()" crates/scaffoldry-server/src --include=*.rs` and paste the output. The only writers left for these fields are inside `refresh()`.

Tests.

1. Two instances, one case for each of the seven collections: a change through A is read through B on the next request.
2. Two instances: a guard added through A denies the next matching request through B.
3. Two instances: a collaborator removed through A loses access through B on the next request.
4. A repository write that fails leaves memory unchanged. Make the write fail with a constraint violation, then read through the same instance.
5. Two requests at once move unit X under Y and unit Y under X. One is `409`. The tree has no cycle.

## Phase 4 — app definitions, process definitions, and policy

1. `engine` holds the live app definitions. `approve_proposal`, and every other function that changes `app_manifests`, bumps `manifests` in its transaction. `decide` no longer calls `register_manifest`. It calls `refresh()`, which rebuilds the engine entries whose stored version changed. Do not rebuild every app when one changes: compare the version each definition carries.
2. `automations` holds the live process definitions. Rule changes bump `rules`. Delete `reload_rules`.
3. When `admin-console.md` phase 5 has run, policy activation bumps `policy` in the activating transaction and the engine is swapped by `refresh()`. That phase says "after commit, swap the engine". Change it to use this mechanism and delete the direct swap. When that phase has not run, add a test marked for it and say so in the pull request.
4. An app definition is validated when it is loaded. One that no longer parses fails the request that needs it with `storage_unavailable` and names the app in the server log. It does not stop the other apps.

Tests.

1. Two instances: a proposal approved through A makes the new field visible through B on the next request.
2. Two instances: a process definition changed through A is the one B starts on the next record change.
3. Two instances approve the same proposal at once. One answer is `200` and one is `409`. There is one `AppPublished` ledger entry.
4. With 500 apps, approving one reloads one. Count the definitions parsed.
5. An app whose stored definition does not parse returns `storage_unavailable` for that app. Another app on the same instance still answers.

## Phase 5 — process instances and the ledger

Neither is cached.

1. Remove `process_instances` from `ServerState`. A decision reads the instance with `SELECT ... FOR UPDATE`, checks that it is still `Waiting` at the step the caller named, writes the new state and the ledger entry, and commits. A second decision on the same step is `409` with the code `version_conflict`.
2. Remove `ledger` from `ServerState`. An append runs in one transaction: lock the head, read its hash, insert the entry. Use `SELECT ... FOR UPDATE` on the last row, or the advisory lock the repository already takes. Do not hold the chain in memory to compute the next hash.
3. Ledger reads page in SQL, newest first, with a cursor. `GET /governance/ledger` returns one page and `total_entries` from `COUNT(*)`. The chain check runs in SQL batches of 1,000 and does not load the whole chain.
4. Run `grep -rn "\.ledger\.\|\.process_instances\." crates/scaffoldry-server/src` and paste the output.

Tests.

1. Two instances append to the ledger at once, 200 entries each. The chain has 400 more entries, the sequence has no gap and no repeat, and the chain check passes.
2. Two instances decide the same waiting step at once. One is `200` and one is `409`. The ledger has one decision.
3. A ledger of 100,000 entries: the first page returns in under 50 ms, and the chain check finishes without holding more than 1,000 entries at a time. Record the times.
4. An instance started while another is appending verifies the chain and starts.

## Phase 6 — the whole surface, readiness, and a clean stop

1. The scan test's allowed list for `ServerState` is now the closed list from phase 1 and nothing else. `memory_records` is gone.
2. `tests/two_instance_test.rs`: for every route in the router and every tool in `TOOLS` that writes, one case that writes through A and reads the result through B. The test builds its list from the router and the registry. A write route with no case fails the test by name.
3. `GET /readyz` returns `200` when `refresh()` succeeds and `503` otherwise. `/healthz` stays as it is and checks nothing. Add `/readyz` to the routes that need no session.
4. On `SIGTERM` the server stops accepting connections, finishes the requests it holds for up to 30 seconds, lets the job runner finish the job it is on, and exits. Use `axum::serve(...).with_graceful_shutdown` and `tokio::signal`. Both are already dependencies.
5. Update `docs/ARCHITECTURE.md`: one paragraph that says PostgreSQL is the only source of truth, names the cached collections, and says that any number of instances may serve one database.

Tests.

1. The two-instance suite passes, and it fails when a write route is added without a case. Prove the second part with a route added and removed in the test.
2. Instance A is stopped while B serves. Every request sent to B succeeds, and a request in flight on A completes.
3. `/readyz` is `503` while the database is unreachable and `200` after it returns, with no restart.
4. A restarted instance serves the same answers as before the restart for one read of every cached collection.

## How to prompt the build agent

```
Read docs/plans/stateless-server.md and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency. Do not add Redis, a broker, or LISTEN and NOTIFY.
PostgreSQL commits before memory changes. A failed read is an error, never an empty list.
Do not add a static lock or a new field to ServerState.
Add your collections to the two-instance test.
Stop when the tests listed for phase N pass, and paste the command output and the grep output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| One query on every request is a tax on everything. | It is one read of an eleven-row table by primary key. Phase 1 measures it and stops the work if the median is above 2 ms. The alternative is a cache that can be wrong, which costs more. |
| A cache invalidated by a counter can still serve a stale read. | Only inside one request that started before the write committed. The next request on any instance reads the counter first. Nothing that decides access for a person or a token is cached at all. |
| Reloading a collection on every change will not scale. | The cached collections are configuration: settings, the org tree, workspaces, guards, labels, and definitions. People, appointments, tokens, records, instances, and the ledger are never cached. App definitions reload one at a time. Phase 4 proves it with 500 apps. |
| `LISTEN` and `NOTIFY` is the usual way to do this. | A notice sent while a connection is down is lost and nothing reports it. A counter read at the start of the request cannot be missed. |
| Two instances will corrupt the ledger chain. | An append locks the head of the chain in PostgreSQL. Phase 5 appends 400 entries from two instances at once and checks every sequence number and hash. |
| Two approvers on two instances will both decide. | The decision locks the instance row and checks its state. One wins and the other gets `409`. The same holds for approving a proposal. |
| The server will start half-loaded after a database fault. | It does not start. Every boot read is checked. Phase 1 tests it by breaking a table. |
| This is a rewrite hidden in a brief. | It removes fields and branches. It adds one table, one middleware, and one small module. No route changes its contract. Every existing test passes at every phase. |
| The tests will get slower with a database each. | Creating an empty database takes tens of milliseconds, and the tests then run at once on every core. Today they run one at a time. |
| A hosted service needs more than this. | Yes. This brief removes the one thing that makes a second instance impossible. Provisioning, backup, monitoring, and a full export are separate work and are listed in `capability-roadmap.md`. |
