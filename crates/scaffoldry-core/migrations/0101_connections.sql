-- Connections phase 1: scheduled, read-only copies of external data. A connection names a source and the dataset it
-- fills. Each run of the sync is a row of sync_runs, with its counts.
CREATE TABLE IF NOT EXISTS connections (
    id UUID PRIMARY KEY,
    name VARCHAR(120) NOT NULL UNIQUE,
    kind VARCHAR(12) NOT NULL,
    config JSONB NOT NULL,
    secret VARCHAR(64),
    dataset_app VARCHAR(64) NOT NULL,
    dataset_table VARCHAR(64) NOT NULL,
    key_field VARCHAR(64) NOT NULL,
    column_map JSONB NOT NULL,
    sensitivity VARCHAR(32) NOT NULL,
    every_minutes INTEGER,
    at_time VARCHAR(5),
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_connections_dataset ON connections (dataset_app, dataset_table);

CREATE TABLE IF NOT EXISTS sync_runs (
    id UUID PRIMARY KEY,
    connection_id UUID NOT NULL REFERENCES connections(id),
    state VARCHAR(10) NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finished_at TIMESTAMPTZ,
    added BIGINT NOT NULL DEFAULT 0,
    changed BIGINT NOT NULL DEFAULT 0,
    removed BIGINT NOT NULL DEFAULT 0,
    unchanged BIGINT NOT NULL DEFAULT 0,
    error TEXT,
    source_hash CHAR(64)
);
CREATE INDEX IF NOT EXISTS idx_sync_runs_connection ON sync_runs (connection_id, started_at DESC);
-- One run of a connection at a time, on any number of servers.
CREATE UNIQUE INDEX IF NOT EXISTS idx_sync_runs_one_running ON sync_runs (connection_id) WHERE state = 'running';

-- Only the sync changes a dataset's records. The services refuse first, with a plain error. These triggers are behind
-- them, so a writer that skipped the check is still stopped. The sync sets `scaffoldry.sync_writer` for its own
-- transaction. A purge of a record already in trash, and a change that touches neither the data nor the trash flag,
-- are not writes of the source's data.
--
-- They run once for each statement, over the rows it touched, not once for each row. A bulk insert of 300,000 rows
-- costs about 0.1 second more, where a row trigger cost about 1.5 seconds more (measured in the PR).
CREATE OR REPLACE FUNCTION guard_dataset_insert() RETURNS trigger AS $$
BEGIN
    IF current_setting('scaffoldry.sync_writer', true) = 'on' THEN
        RETURN NULL;
    END IF;
    IF EXISTS (SELECT 1 FROM new_rows n JOIN connections c ON c.dataset_app = n.app_slug) THEN
        RAISE EXCEPTION 'read_only_source: a dataset''s records are changed only by its sync' USING ERRCODE = 'P0409';
    END IF;
    RETURN NULL;
END
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION guard_dataset_update() RETURNS trigger AS $$
BEGIN
    IF current_setting('scaffoldry.sync_writer', true) = 'on' THEN
        RETURN NULL;
    END IF;
    IF EXISTS (
        SELECT 1 FROM old_rows o JOIN new_rows n ON n.id = o.id JOIN connections c ON c.dataset_app = n.app_slug
        WHERE n.data IS DISTINCT FROM o.data OR n.deleted_at IS DISTINCT FROM o.deleted_at
    ) THEN
        RAISE EXCEPTION 'read_only_source: a dataset''s records are changed only by its sync' USING ERRCODE = 'P0409';
    END IF;
    RETURN NULL;
END
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION guard_dataset_delete() RETURNS trigger AS $$
BEGIN
    IF current_setting('scaffoldry.sync_writer', true) = 'on' THEN
        RETURN NULL;
    END IF;
    IF EXISTS (SELECT 1 FROM old_rows o JOIN connections c ON c.dataset_app = o.app_slug WHERE o.deleted_at IS NULL) THEN
        RAISE EXCEPTION 'read_only_source: a dataset''s records are changed only by its sync' USING ERRCODE = 'P0409';
    END IF;
    RETURN NULL;
END
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS guard_dataset_records ON dataset_records;
DROP TRIGGER IF EXISTS guard_dataset_insert ON dataset_records;
DROP TRIGGER IF EXISTS guard_dataset_update ON dataset_records;
DROP TRIGGER IF EXISTS guard_dataset_delete ON dataset_records;
CREATE TRIGGER guard_dataset_insert AFTER INSERT ON dataset_records
    REFERENCING NEW TABLE AS new_rows FOR EACH STATEMENT EXECUTE FUNCTION guard_dataset_insert();
CREATE TRIGGER guard_dataset_update AFTER UPDATE ON dataset_records
    REFERENCING OLD TABLE AS old_rows NEW TABLE AS new_rows FOR EACH STATEMENT EXECUTE FUNCTION guard_dataset_update();
CREATE TRIGGER guard_dataset_delete AFTER DELETE ON dataset_records
    REFERENCING OLD TABLE AS old_rows FOR EACH STATEMENT EXECUTE FUNCTION guard_dataset_delete();
DROP FUNCTION IF EXISTS guard_dataset_records();

-- The catalog no longer holds sample rows typed into it. A dataset's rows come from its sync.
UPDATE published_datasets SET payload = payload - 'sample_data' WHERE payload ? 'sample_data';
