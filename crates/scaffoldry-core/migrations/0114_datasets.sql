-- Data layer phase 1: an app table is a dataset with an id.
-- The row holds identity only. The name and fields stay in the app manifest, so there is one source for them.
-- Nothing reads this table to decide access. The app path still decides.

CREATE TABLE IF NOT EXISTS datasets (
    id VARCHAR(64) PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_datasets_table UNIQUE (app_slug, table_id)
);

-- One dataset for each table of each stored app. Safe to run again: a table that has one keeps it.
INSERT INTO datasets (id, app_slug, table_id)
SELECT 'ds_' || replace(gen_random_uuid()::text, '-', ''), m.slug, t->>'id'
FROM app_manifests m, jsonb_array_elements(COALESCE(m.manifest->'tables', '[]'::jsonb)) t
WHERE t->>'id' IS NOT NULL
ON CONFLICT (app_slug, table_id) DO NOTHING;
