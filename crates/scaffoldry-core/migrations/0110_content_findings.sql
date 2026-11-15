-- Sensitive content phase 2: findings of the scan, the places a category may live, and what the scan expected to find.
-- A finding holds the field, the detector, counts and up to five record ids. It never holds a matched value, because a value
-- would put the sensitive data in a second place.
CREATE TABLE IF NOT EXISTS content_findings (
    id UUID PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    field VARCHAR(64) NOT NULL,
    detector_id VARCHAR(64) NOT NULL,
    category VARCHAR(64) NOT NULL,
    source VARCHAR(12) NOT NULL,
    match_count BIGINT NOT NULL,
    record_count BIGINT NOT NULL,
    sample_record_ids TEXT[] NOT NULL DEFAULT '{}',
    status VARCHAR(12) NOT NULL DEFAULT 'open',
    decided_by VARCHAR(255),
    decided_at TIMESTAMPTZ,
    reason TEXT,
    first_seen TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (app_slug, table_id, field, detector_id)
);
CREATE INDEX IF NOT EXISTS idx_content_findings_status ON content_findings (status, app_slug);

CREATE TABLE IF NOT EXISTS authorized_stores (
    category VARCHAR(64) NOT NULL,
    scope VARCHAR(12) NOT NULL,
    app_slug VARCHAR(64) NOT NULL DEFAULT '',
    table_id VARCHAR(64) NOT NULL DEFAULT '',
    field VARCHAR(64) NOT NULL DEFAULT '',
    workspace_id VARCHAR(64) NOT NULL DEFAULT '',
    reason TEXT NOT NULL,
    set_by VARCHAR(255) NOT NULL,
    set_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (category, scope, workspace_id, app_slug, table_id, field)
);

-- What a scan read inside an authorized store and so did not report. It is a count and nothing else. A scan of a table replaces its rows.
CREATE TABLE IF NOT EXISTS content_expected (
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    field VARCHAR(64) NOT NULL,
    category VARCHAR(64) NOT NULL,
    matches BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (app_slug, table_id, field, category)
);

-- A label can carry categories as well as the flag. A label with categories is protected only if one of them is protected.
ALTER TABLE data_labels ADD COLUMN IF NOT EXISTS categories TEXT[] NOT NULL DEFAULT '{}';
