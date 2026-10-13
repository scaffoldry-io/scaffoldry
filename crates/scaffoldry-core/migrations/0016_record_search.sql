ALTER TABLE dataset_records
    ADD COLUMN IF NOT EXISTS search tsvector
    GENERATED ALWAYS AS (jsonb_to_tsvector('simple', data, '["string", "numeric"]')) STORED;
CREATE INDEX IF NOT EXISTS idx_dataset_records_search ON dataset_records USING gin (search);
DROP INDEX IF EXISTS idx_dataset_records_data_gin;
