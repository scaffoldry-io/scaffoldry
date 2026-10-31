-- Proposal preview phase 3: a trial copy of a proposed app, in a synthetic workspace, filled with synthetic records.
CREATE TABLE IF NOT EXISTS proposal_trials (
    id UUID PRIMARY KEY,
    proposal_id UUID NOT NULL,
    owner VARCHAR(255) NOT NULL,
    app_slug VARCHAR(64) NOT NULL,
    workspace_id VARCHAR(64) NOT NULL,
    records INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_opened_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    destroyed_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_proposal_trials_owner ON proposal_trials (owner) WHERE destroyed_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_proposal_trials_proposal ON proposal_trials (proposal_id) WHERE destroyed_at IS NULL;

-- Trials that have gone unopened for the number of days in the settings are destroyed. The job looks hourly.
INSERT INTO job_schedules (kind, every_seconds, next_run_at, enabled)
VALUES ('expire_trials', 3600, NOW() + INTERVAL '1 hour', TRUE)
ON CONFLICT (kind) DO NOTHING;
