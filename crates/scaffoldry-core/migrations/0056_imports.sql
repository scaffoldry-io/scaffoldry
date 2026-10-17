-- Import phase 1: App imports table for uploaded spreadsheets/CSVs, mapping, dry runs, and execution.
CREATE TABLE IF NOT EXISTS app_imports (
    id UUID PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    file_ref_id UUID NOT NULL REFERENCES file_refs(id) ON DELETE CASCADE,
    target_table_id VARCHAR(64),
    status VARCHAR(32) NOT NULL DEFAULT 'uploaded',
    delimiter VARCHAR(8) NOT NULL DEFAULT ',',
    encoding VARCHAR(32) NOT NULL DEFAULT 'UTF-8',
    columns JSONB NOT NULL DEFAULT '[]',
    sample JSONB NOT NULL DEFAULT '[]',
    inferred_types JSONB NOT NULL DEFAULT '{}',
    mapping JSONB,
    dry_run_result JSONB,
    dry_run_at TIMESTAMPTZ,
    run_result JSONB,
    checkpoint JSONB,
    rejected_file_ref_id UUID REFERENCES file_refs(id) ON DELETE SET NULL,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT (NOW() + INTERVAL '24 hours')
);

CREATE INDEX IF NOT EXISTS idx_app_imports_slug ON app_imports(app_slug);
CREATE INDEX IF NOT EXISTS idx_app_imports_expires ON app_imports(expires_at);
