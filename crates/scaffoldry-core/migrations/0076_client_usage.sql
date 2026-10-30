-- External apps phase 6: what each outside app did, by day. The Outside apps panel reads thirty days of it.
CREATE TABLE IF NOT EXISTS client_usage (
    client_id VARCHAR(64) NOT NULL,
    day DATE NOT NULL,
    calls BIGINT NOT NULL DEFAULT 0,
    records_read BIGINT NOT NULL DEFAULT 0,
    records_written BIGINT NOT NULL DEFAULT 0,
    last_call_at TIMESTAMPTZ,
    PRIMARY KEY (client_id, day)
);
