CREATE TABLE IF NOT EXISTS index_status (
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    field VARCHAR(64) NOT NULL,
    state VARCHAR(10) NOT NULL,
    job_id UUID,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (app_slug, table_id, field)
);
