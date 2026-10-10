ALTER TABLE dataset_records ADD COLUMN IF NOT EXISTS table_id VARCHAR(64) NOT NULL DEFAULT '';
UPDATE dataset_records SET table_id = COALESCE(data->>'_table_id', '') WHERE table_id = '';
CREATE INDEX IF NOT EXISTS idx_dataset_records_page
    ON dataset_records (app_slug, table_id, created_at, id);
