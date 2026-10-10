CREATE TABLE IF NOT EXISTS process_instances (
    id VARCHAR(128) PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    rule_id VARCHAR(64) NOT NULL,
    status VARCHAR(16) NOT NULL,
    instance_json JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_process_instances_app ON process_instances(app_slug, status);
