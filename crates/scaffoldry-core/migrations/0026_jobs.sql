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
