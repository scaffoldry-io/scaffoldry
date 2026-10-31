-- Import and export phase 4: approved templates. A template is a bundle that a second person reviewed.
-- It waits as `pending` until someone other than its publisher approves it. `retired` templates stay for the record.
CREATE TABLE IF NOT EXISTS app_templates (
    id UUID PRIMARY KEY,
    name VARCHAR(120) NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    category VARCHAR(60) NOT NULL DEFAULT 'General',
    bundle JSONB NOT NULL,
    bundle_sha256 CHAR(64) NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    status VARCHAR(8) NOT NULL DEFAULT 'pending',
    unit_id UUID,
    published_by VARCHAR(255) NOT NULL,
    published_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    decided_by VARCHAR(255),
    decided_at TIMESTAMPTZ,
    rationale TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_app_templates_status ON app_templates (status);
