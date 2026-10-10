CREATE TABLE IF NOT EXISTS job_schedules (
    kind VARCHAR(64) PRIMARY KEY,
    every_seconds INTEGER NOT NULL,
    next_run_at TIMESTAMPTZ NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE
);
