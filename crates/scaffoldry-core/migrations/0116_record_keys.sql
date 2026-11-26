-- Data layer phase 2b: a dataset declares a key (docs/plans/data-layer.md, "Keys and local columns").
-- The key is a copy of the manifest's table key, kept on the dataset row so a write can read it in its own transaction.
-- The manifest stays the source. Both are written together when a proposal that sets the key is approved.
ALTER TABLE datasets ADD COLUMN IF NOT EXISTS key_spec JSONB;

-- One row for each live record of a table that declares a key. The unique constraint is what refuses a second record
-- with the same key text. A trashed record holds no row. The row goes with its record.
CREATE TABLE IF NOT EXISTS record_keys (
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    key_text TEXT NOT NULL,
    record_id VARCHAR(64) NOT NULL REFERENCES dataset_records(id) ON DELETE CASCADE,
    CONSTRAINT uq_record_keys UNIQUE (app_slug, table_id, key_text),
    CONSTRAINT uq_record_keys_record UNIQUE (record_id)
);
