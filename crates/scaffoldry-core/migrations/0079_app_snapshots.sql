-- Import and export phase 5: snapshots. A snapshot is an app's definition and every record, as compressed JSON Lines in
-- the file store. It holds everything the app's data holds, so it is as sensitive as the app.
CREATE TABLE IF NOT EXISTS app_snapshots (
    id UUID PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    name VARCHAR(120) NOT NULL,
    kind VARCHAR(10) NOT NULL,
    state VARCHAR(8) NOT NULL DEFAULT 'pending',
    file_sha256 CHAR(64),
    record_count BIGINT,
    size BIGINT,
    error TEXT,
    -- The definition as a bundle, taken when the snapshot was asked for. Restore proposes from it.
    definition JSONB NOT NULL,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_app_snapshots_app ON app_snapshots (app_slug, created_at DESC);
