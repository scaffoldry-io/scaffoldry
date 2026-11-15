-- Admin console phase 4: data governance. A label raises one field's sensitivity at once, without a structural change to the
-- manifest, so no proposal is needed. A label can only add protection: a `false` never lowers a flag the manifest sets.
-- The table holds exceptions only, so it stays small.
CREATE TABLE IF NOT EXISTS data_labels (
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    field VARCHAR(64) NOT NULL,
    ferpa_sensitive BOOLEAN NOT NULL,
    note TEXT NOT NULL,
    set_by VARCHAR(255) NOT NULL,
    set_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (app_slug, table_id, field)
);
