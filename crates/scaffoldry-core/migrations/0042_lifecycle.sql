ALTER TABLE dataset_records ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
ALTER TABLE dataset_records ADD COLUMN IF NOT EXISTS deleted_by VARCHAR(255);
ALTER TABLE dataset_records ADD COLUMN IF NOT EXISTS delete_batch UUID;

DROP INDEX IF EXISTS idx_dataset_records_page;
CREATE INDEX IF NOT EXISTS idx_dataset_records_live
    ON dataset_records (app_slug, table_id, created_at, id) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_dataset_records_trash
    ON dataset_records (app_slug, deleted_at) WHERE deleted_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS trash_links (
    record_id VARCHAR(64) NOT NULL,
    field VARCHAR(64) NOT NULL,
    other_record VARCHAR(64) NOT NULL,
    direction VARCHAR(4) NOT NULL,
    PRIMARY KEY (record_id, field, other_record, direction)
);
