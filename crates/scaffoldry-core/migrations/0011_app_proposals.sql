CREATE TABLE IF NOT EXISTS app_proposals (
    id UUID PRIMARY KEY,
    workspace_id VARCHAR(64) NOT NULL REFERENCES workspaces(id),
    app_slug VARCHAR(64) NOT NULL,
    base_version INTEGER,
    manifest JSONB NOT NULL,
    checks JSONB NOT NULL,
    summary TEXT NOT NULL,
    status VARCHAR(16) NOT NULL DEFAULT 'Pending',
    proposed_by VARCHAR(255) NOT NULL,
    decided_by VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    decided_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_app_proposals_workspace ON app_proposals(workspace_id, status);

-- A page's source, named by its hash. A row is never updated. A changed page is a new row.
CREATE TABLE IF NOT EXISTS app_page_sources (
    sha256 CHAR(64) PRIMARY KEY,
    source TEXT NOT NULL,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
