CREATE TABLE IF NOT EXISTS record_index (
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    field VARCHAR(64) NOT NULL,
    record_id VARCHAR(64) NOT NULL REFERENCES dataset_records(id) ON DELETE CASCADE,
    num DOUBLE PRECISION,
    txt VARCHAR(128),
    ts TIMESTAMPTZ,
    ord SMALLINT NOT NULL DEFAULT 0,
    PRIMARY KEY (record_id, field, ord)
);
CREATE INDEX IF NOT EXISTS idx_record_index_num ON record_index (app_slug, table_id, field, num, record_id);
CREATE INDEX IF NOT EXISTS idx_record_index_txt ON record_index (app_slug, table_id, field, txt, record_id);
CREATE INDEX IF NOT EXISTS idx_record_index_ts  ON record_index (app_slug, table_id, field, ts, record_id);
