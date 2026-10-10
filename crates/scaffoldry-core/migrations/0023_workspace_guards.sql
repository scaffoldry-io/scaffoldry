CREATE TABLE IF NOT EXISTS workspace_guards (
    workspace_id VARCHAR(64) NOT NULL REFERENCES workspaces(id),
    version INTEGER NOT NULL,
    rules JSONB NOT NULL,
    compiled TEXT NOT NULL,
    reason TEXT NOT NULL,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (workspace_id, version)
);

CREATE TABLE IF NOT EXISTS workspace_guards_legacy (
    workspace_id VARCHAR(64) PRIMARY KEY,
    cedar_policy_guard TEXT NOT NULL,
    archived_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
INSERT INTO workspace_guards_legacy (workspace_id, cedar_policy_guard)
    SELECT id, cedar_policy_guard FROM workspaces
    WHERE cedar_policy_guard IS NOT NULL AND cedar_policy_guard <> ''
ON CONFLICT DO NOTHING;
ALTER TABLE workspaces DROP COLUMN IF EXISTS cedar_policy_guard;
