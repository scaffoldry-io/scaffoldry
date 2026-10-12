-- The disclosure log: a record of each release of sensitive fields, so a student's request for their
-- record of disclosures can be answered. It records candidate events and does not decide what is a
-- disclosure. Only a release that includes a sensitive field is written.
CREATE TABLE IF NOT EXISTS disclosure_log (
    id UUID PRIMARY KEY,
    at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    actor VARCHAR(255) NOT NULL,
    kind VARCHAR(20) NOT NULL,
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64),
    record_count BIGINT NOT NULL,
    fields TEXT[] NOT NULL,
    recipient VARCHAR(255),
    note TEXT NOT NULL DEFAULT '',
    records_truncated BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX IF NOT EXISTS idx_disclosure_app ON disclosure_log (app_slug, at DESC);

CREATE TABLE IF NOT EXISTS disclosure_records (
    disclosure_id UUID NOT NULL REFERENCES disclosure_log(id),
    record_id VARCHAR(64) NOT NULL,
    PRIMARY KEY (disclosure_id, record_id)
);
CREATE INDEX IF NOT EXISTS idx_disclosure_record ON disclosure_records (record_id);
