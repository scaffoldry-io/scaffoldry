-- Lifecycle phase 4. A legal hold overrides every destruction path. A held record may go to trash and come back. It is
-- never destroyed. NIST SP 800-53 AU-11(1) and SI-12.
CREATE TABLE IF NOT EXISTS legal_holds (
    id UUID PRIMARY KEY,
    name VARCHAR(120) NOT NULL,
    case_ref VARCHAR(120) NOT NULL DEFAULT '',
    reason TEXT NOT NULL,
    scope_kind VARCHAR(10) NOT NULL CHECK (scope_kind IN ('workspace', 'app', 'table', 'query')),
    workspace_id VARCHAR(64),
    app_slug VARCHAR(64),
    table_id VARCHAR(64),
    -- A query hold: field = value pairs a record's data must contain.
    filter JSONB,
    include_future BOOLEAN NOT NULL DEFAULT FALSE,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    released_by VARCHAR(255),
    released_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS hold_records (
    hold_id UUID NOT NULL REFERENCES legal_holds(id),
    record_id VARCHAR(64) NOT NULL,
    PRIMARY KEY (hold_id, record_id)
);
CREATE INDEX IF NOT EXISTS idx_hold_records_record ON hold_records (record_id);

-- The one answer to "may this record be destroyed?". Every destruction path in `purge.rs` asks it.
CREATE OR REPLACE FUNCTION is_held(p_app VARCHAR, p_record VARCHAR) RETURNS BOOLEAN AS $$
    SELECT EXISTS (
        SELECT 1 FROM legal_holds h
        LEFT JOIN dataset_records r ON r.id = p_record AND r.app_slug = p_app
        WHERE h.released_at IS NULL AND (
            (h.scope_kind = 'workspace' AND h.workspace_id = (SELECT m.manifest->>'workspace_id' FROM app_manifests m WHERE m.slug = p_app))
            OR (h.scope_kind = 'app' AND h.app_slug = p_app)
            OR (h.scope_kind = 'table' AND h.include_future AND h.app_slug = p_app AND h.table_id = r.table_id)
            OR (h.scope_kind = 'query' AND h.include_future AND h.app_slug = p_app AND (h.table_id IS NULL OR h.table_id = r.table_id) AND r.data @> h.filter)
            OR EXISTS (SELECT 1 FROM hold_records hr WHERE hr.hold_id = h.id AND hr.record_id = p_record)
        )
    )
$$ LANGUAGE sql STABLE;
