# Jobs — implementation brief

Hand this file to Gemini Flash. Do one phase per session. Stop when that phase's tests pass. Do not start the next phase.

Run order is `docs/plans/README.md`. Phase 1 needs `foundation.md` phase 4 (several database workers). Phase 3 needs `row-scale.md` phase 2. Its item 4 (recomputing stored flags after a label change) is done when `admin-console.md` phase 4 runs, and until then labels do not exist.

Some work takes longer than a request should. Building an index over a million rows. Recalculating a formula column. Importing a spreadsheet. Writing an export. Sending mail. Delivering a webhook. Purging records past retention. Today each of these either runs inside the request or is not planned. A request that runs for minutes fails when a proxy gives up, holds a database connection, and leaves nothing to show a person what happened.

This brief adds one durable queue that every later brief uses.

## Decisions already made

| Question | Answer |
| --- | --- |
| Where does the queue live? | A PostgreSQL table. Workers claim rows with `FOR UPDATE SKIP LOCKED`. No Redis, no second service |
| Where do workers run? | Threads in the server process. Default two. Each owns one database connection |
| What if the server stops mid-job? | The job's lock expires and another worker takes it. Every job handler is written to be resumed |
| Does a job result carry data? | A small summary only. A produced file is stored by `import-export.md` and the job names it |
| Who may see a job? | Its creator, and a Platform Admin |

## What exists today

| Fact | Where |
| --- | --- |
| One connection per worker thread, with a channel of closures | `repository.rs` `PostgresRepository` |
| After `foundation.md` phase 4, eight such workers serve requests | `foundation.md` |
| An approved manifest change that adds an index runs the copy inside the approval transaction | `row-scale.md` phase 2, with a `ponytail` note |
| A label change recomputes record flags inside its transaction | `admin-console.md` phase 4 |

## Out of scope

- A second queue technology.
- Jobs that run on another machine.
- A priority system beyond a per-kind concurrency cap.
- A workflow of jobs that depend on jobs. A handler may enqueue another job.
- Exactly-once execution. Handlers are at-least-once and idempotent.

## Rules for every phase

1. Write the failing test first. Run it. Confirm it fails because the behavior is missing. Then write the code.
2. Do not add a dependency.
3. A handler never holds a database transaction open across batches. It commits each batch and records a checkpoint.
4. A handler checks for cancellation between batches.
5. A job never stores a secret, a record value, or a token in its payload or result. It stores ids and counts.

## Phase 1 — the queue

Migration `crates/scaffoldry-core/migrations/0026_jobs.sql`:

```sql
CREATE TABLE IF NOT EXISTS jobs (
    id UUID PRIMARY KEY,
    kind VARCHAR(64) NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}',
    state VARCHAR(10) NOT NULL DEFAULT 'queued',
    progress_done BIGINT NOT NULL DEFAULT 0,
    progress_total BIGINT,
    checkpoint JSONB,
    result JSONB,
    error TEXT,
    log JSONB NOT NULL DEFAULT '[]',
    attempts INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 3,
    run_after TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    locked_by VARCHAR(64),
    locked_until TIMESTAMPTZ,
    cancel_requested BOOLEAN NOT NULL DEFAULT FALSE,
    dedupe_key VARCHAR(128),
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_jobs_claim ON jobs (run_after, created_at) WHERE state = 'queued';
CREATE UNIQUE INDEX IF NOT EXISTS idx_jobs_dedupe ON jobs (dedupe_key) WHERE dedupe_key IS NOT NULL AND state IN ('queued', 'running');
CREATE INDEX IF NOT EXISTS idx_jobs_owner ON jobs (created_by, created_at DESC);
```

`state` is `queued`, `running`, `done`, `failed`, or `cancelled`.

Registry, in a new `crates/scaffoldry-server/src/jobs.rs`:

```rust
pub struct JobKind {
    pub name: &'static str,
    pub max_attempts: i32,
    pub max_concurrent: usize,
    pub run: fn(&JobCtx, serde_json::Value) -> Result<serde_json::Value, JobError>,
}
pub static JOB_KINDS: &[JobKind] = &[ /* one per kind */ ];
```

`JobCtx` offers `progress(done, total)`, `checkpoint(json)`, `load_checkpoint()`, `cancelled() -> bool`, `log(line)` (keeps the last 200 lines), and the repository.

Claim, in one transaction:

```sql
UPDATE jobs SET state = 'running', locked_by = $1, locked_until = NOW() + INTERVAL '60 seconds',
       attempts = attempts + 1, started_at = COALESCE(started_at, NOW())
WHERE id = (
    SELECT id FROM jobs WHERE state = 'queued' AND run_after <= NOW()
      AND NOT (kind = ANY($2))
    ORDER BY run_after, created_at FOR UPDATE SKIP LOCKED LIMIT 1)
RETURNING id, kind, payload, checkpoint, attempts
```

`$2` is the list of kinds whose running count has reached `max_concurrent`. Compute it in the same transaction.

A running job extends `locked_until` every 20 seconds. A recovery pass, run every 30 seconds by one worker, returns a `running` job whose lock has expired to `queued` with `run_after` set by backoff, or marks it `failed` when `attempts` has reached `max_attempts`. A handler error retries with delays of 30 seconds, 5 minutes, and 30 minutes, then fails. A `JobError::Permanent` fails at once.

`SCAFFOLDRY_JOB_WORKERS` sets the thread count, default 2. It is a process fact, so it stays an environment variable like the database worker count. Workers start after migrations and stop on shutdown after finishing the current batch.

API, all signed in:

| Method | Path | Behavior |
| --- | --- | --- |
| GET | `/api/v1/jobs/{id}` | State, progress, result, error, and the log. Only the creator and a Platform Admin. Others get 404 |
| POST | `/api/v1/jobs/{id}/cancel` | Sets `cancel_requested`. A queued job becomes `cancelled` at once |
| GET | `/api/v1/jobs` | The caller's jobs, newest first, paged |

Add error code `job_failed` to the closed list in `ux-standards.md`. Add `enqueue(kind, payload, created_by, dedupe_key) -> job_id` as the one way to create a job.

Tests, in `crates/scaffoldry-server/tests/jobs_test.rs`:

1. Eight threads claiming from 100 queued jobs run each job exactly once.
2. A job whose worker is killed (drop its connection) after claiming returns to `queued` after the lock expires and runs again, with `attempts` of 2.
3. A handler that errors three times ends `failed` with the last error. One that returns `Permanent` ends `failed` after one attempt.
4. A resumed job sees the checkpoint the first run wrote.
5. A kind with `max_concurrent` of 1 never runs two at once with ten queued.
6. Enqueue with a `dedupe_key` while one is queued returns the existing id. After it finishes, the key can be used again.
7. Cancelling a running job makes `cancelled()` true, and the job ends `cancelled` at its next batch.
8. A user who did not create the job gets 404. A Platform Admin gets 200.
9. A payload or result containing a string key named `token` or `secret` is refused by `enqueue` and by `JobCtx::result`.

## Phase 2 — a schedule, and the Jobs panel

Migration `0044_job_schedules.sql`:

```sql
CREATE TABLE IF NOT EXISTS job_schedules (
    kind VARCHAR(64) PRIMARY KEY,
    every_seconds INTEGER NOT NULL,
    next_run_at TIMESTAMPTZ NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE
);
```

One scheduler thread wakes every 15 seconds. Under `pg_try_advisory_lock`, so only one process acts, it inserts a job for each due schedule with `dedupe_key = kind` and advances `next_run_at` by `every_seconds`, counted from the old `next_run_at` so a slow tick does not drift. Schedules are registered in code with `register_schedule(kind, every_seconds)`, inserted at boot if missing, and never edited from a request. A schedule can be switched off only by the setting `jobs.schedules_disabled` (an array of kinds), added to the closed list in `foundation.md` phase 8.

Admin panel and routes, all Platform Admin and all in `ADMIN_ROUTES`, built from the kit:

| Method | Path | Behavior |
| --- | --- | --- |
| GET | `/admin/jobs` | Query `state`, `kind`, `owner`, `cursor`, `limit`. Row: id, kind, owner, state, progress as a percent when the total is known, attempts, created, age, last log line |
| GET | `/admin/jobs/{id}` | Full detail and the log |
| POST | `/admin/jobs/{id}/cancel` | Body `reason`. Ledger entry `JobCancelled`, a new variant added to `DecisionType::ALL` |
| POST | `/admin/jobs/{id}/retry` | A `failed` job becomes `queued` with `attempts` reset. Body `reason` |

The Overview from `admin-console.md` gains queue depth, the age of the oldest queued job, and the number of jobs that failed in the last 24 hours.

MCP tools, both `SignedIn` scope with an owner check inside: `get_job` and `cancel_job`. Add them to the tool registry. Any tool that starts a long job returns `{ "job_id": ... }` and tells the agent to poll `get_job`.

Tests.

1. A registered schedule with `every_seconds` of 60 creates one job per minute across two simulated scheduler threads, never two.
2. A tick delayed by 90 seconds creates one job, not two, and the next run is on the original cadence.
3. Setting `jobs.schedules_disabled` to a kind stops it.
4. The admin routes are in `ADMIN_ROUTES`. A faculty caller is 403.
5. `retry` on a `failed` job runs it again. `retry` on a `done` job is 409.
6. `get_job` through MCP returns another user's job as not found.
7. Web: the panel shows progress as text and a bar, a `failed` job shows its error, `Cancel` asks for a reason.

## Phase 3 — move the heavy work off the request

Needs `row-scale.md` phase 2 and `admin-console.md` phase 4.

Migration `0045_index_status.sql`:

```sql
CREATE TABLE IF NOT EXISTS index_status (
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    field VARCHAR(64) NOT NULL,
    state VARCHAR(10) NOT NULL,
    job_id UUID,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (app_slug, table_id, field)
);
```

`state` is `building` or `ready`.

1. `rebuild_index` becomes the job kind `build_index`, with payload `app_slug`, `table_id`, and the fields. It works in keyset batches of 2,000 records, checkpoints the last id, and sets `ready` at the end.
2. The approval transaction in `row-scale.md` phase 2 now only writes the manifest, writes `index_status` rows as `building`, and enqueues one `build_index` job with `dedupe_key` of the app, table, and field set. It no longer copies values.
3. A list that sorts or filters on a `building` field is refused with error code `index_building` and the text `This field is still being prepared for sorting. It is {percent} done.` Add `index_building` to the closed list in `ux-standards.md`.
4. Label recomputation from `admin-console.md` phase 4 becomes the job kind `recompute_flags`, in batches. The label change commits first. Until the job finishes, reads apply the label at read time through `effective_ferpa_sensitive`, so protection is immediate even while stored flags catch up.
5. Writes that arrive during a build still write index rows for the field, so nothing is missed. The job skips rows whose index row already holds the current version.

Tests.

1. Approving a manifest that adds an indexed field on a 200,000-row table returns in under one second and leaves `building` rows. After the job runs, `index_status` is `ready` and every record has an index row.
2. A sort on a `building` field returns `index_building` with a percent. After the job, it works.
3. A record written during the build has an index row, and the job leaves it unchanged.
4. Kill the worker halfway. After recovery the job resumes from its checkpoint and the final index is complete with no duplicate rows.
5. A raised label protects a read immediately, before `recompute_flags` has run.

## Phase 4 — limits and visibility

1. A user may have at most 5 queued or running jobs, and the whole server at most 500 queued. Past either, `enqueue` fails with error code `quota_exceeded`. Add that code to the closed list.
2. Finished jobs older than 30 days are deleted by a scheduled job, `purge_jobs`. Failed ones are kept for 90 days.
3. The desk shows a small `Background work` indicator for the signed-in user's running jobs, with progress, using polling every five seconds until `realtime.md` phase 3 replaces it. It is a kit `Banner`-style region with `role="status"`.
4. Every job kind in `JOB_KINDS` has a one-sentence plain description and a plain name, shown in the panel and the indicator.

Tests.

1. The sixth job from one user is refused with `quota_exceeded`.
2. `purge_jobs` removes a 31-day-old `done` job and keeps a 31-day-old `failed` one.
3. Every `JOB_KINDS` entry has a non-empty description. A test iterates the registry.
4. Web: the indicator appears while a job runs and disappears when it finishes.

## How to prompt Gemini

```
Read docs/plans/jobs.md and docs/plans/README.md.
Implement phase N only.
Write the failing test first and run it.
Do not add a dependency.
Do not hold a transaction across batches.
Do not put a record value, secret, or token in a job payload or result.
Do not read, copy, or adapt another product's source code.
Stop when the tests listed for phase N pass, and paste the command output.
```

## Enterprise objections

| Objection | Answer |
| --- | --- |
| A queue in the database will slow the database. | Claiming is one indexed statement. Workers poll every second at most, and the table is trimmed. The queue shares a server a pilot already runs. |
| A job will run twice. | Yes, rarely. Handlers are resumable and idempotent, and phase 1 tests resume. Exactly-once would need a coordinator this plan refuses to add. |
| A job will leak data in its log. | `enqueue` and the result writer refuse keys that look like secrets, and the rule says ids and counts only. |
| Anyone can flood the queue. | A per-user and a global cap, with a clear error. |
| It takes minutes to see a new sort work. | The screen says it is preparing and shows the percent. Until then it refuses to sort, so it never returns a wrong order. |
