-- Lifecycle phase 3. Delete is trash. Destruction happens only by policy, after the ledger records it, and leaves a
-- certificate. NIST SP 800-53 SI-12, MP-6 and AU-11.
CREATE TABLE IF NOT EXISTS retention_policies (
    id UUID PRIMARY KEY,
    name VARCHAR(120) NOT NULL,
    scope_kind VARCHAR(14) NOT NULL CHECK (scope_kind IN ('classification', 'workspace')),
    classification VARCHAR(32),
    workspace_id VARCHAR(64),
    keep_trash_days INTEGER NOT NULL CHECK (keep_trash_days >= 0),
    keep_history_days INTEGER CHECK (keep_history_days >= 0),
    keep_retired_days INTEGER NOT NULL CHECK (keep_retired_days >= 0),
    max_record_age_days INTEGER CHECK (max_record_age_days >= 0),
    on_expiry VARCHAR(5) NOT NULL DEFAULT 'trash' CHECK (on_expiry IN ('trash', 'purge')),
    basis TEXT NOT NULL,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK ((scope_kind = 'classification' AND classification IS NOT NULL) OR (scope_kind = 'workspace' AND workspace_id IS NOT NULL))
);
-- One active policy per classification and per workspace, so which one applies is never a choice.
CREATE UNIQUE INDEX IF NOT EXISTS idx_retention_one_per_classification ON retention_policies (classification) WHERE active AND scope_kind = 'classification';
CREATE UNIQUE INDEX IF NOT EXISTS idx_retention_one_per_workspace ON retention_policies (workspace_id) WHERE active AND scope_kind = 'workspace';

CREATE TABLE IF NOT EXISTS purge_certificates (
    id UUID PRIMARY KEY,
    run_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    policy_id UUID,
    app_slug VARCHAR(64) NOT NULL,
    counts JSONB NOT NULL,
    ids_sha256 CHAR(64) NOT NULL,
    held_skipped INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_purge_certificates_run ON purge_certificates (run_at DESC);

-- A run in progress, so a run that stops resumes where it was. The ids are kept only until the run's certificate is
-- written, in the same transaction that deletes them.
CREATE TABLE IF NOT EXISTS purge_runs (
    id UUID PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    policy_id UUID,
    counts JSONB NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE TABLE IF NOT EXISTS purge_pending (
    run_id UUID NOT NULL REFERENCES purge_runs(id) ON DELETE CASCADE,
    record_id VARCHAR(64) NOT NULL,
    done BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (run_id, record_id)
);
