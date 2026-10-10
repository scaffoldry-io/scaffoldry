CREATE TABLE IF NOT EXISTS workflow_rule_versions (
    rule_id VARCHAR(64) NOT NULL,
    version INTEGER NOT NULL,
    app_slug VARCHAR(64) NOT NULL,
    rule_json JSONB NOT NULL,
    summary TEXT NOT NULL,
    proposal_id UUID,
    changed_by VARCHAR(255) NOT NULL,
    changed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (rule_id, version)
);
CREATE INDEX IF NOT EXISTS idx_rule_versions_app ON workflow_rule_versions(app_slug);
