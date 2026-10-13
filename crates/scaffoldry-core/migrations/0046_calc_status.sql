CREATE TABLE IF NOT EXISTS calc_status (
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    field VARCHAR(64) NOT NULL,
    state VARCHAR(12) NOT NULL DEFAULT 'calculating',
    job_id UUID,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (app_slug, table_id, field)
);
