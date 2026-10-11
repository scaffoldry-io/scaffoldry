CREATE TABLE IF NOT EXISTS record_links (
    app_slug VARCHAR(64) NOT NULL,
    field VARCHAR(64) NOT NULL,
    from_record VARCHAR(64) NOT NULL,
    to_record VARCHAR(64) NOT NULL,
    ord SMALLINT NOT NULL DEFAULT 0,
    PRIMARY KEY (from_record, field, to_record)
);
CREATE INDEX IF NOT EXISTS idx_record_links_reverse ON record_links (to_record, field);
