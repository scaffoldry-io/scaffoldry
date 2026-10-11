CREATE TABLE IF NOT EXISTS trash_links (
    record_id VARCHAR(64) NOT NULL,
    field VARCHAR(64) NOT NULL,
    other_record VARCHAR(64) NOT NULL,
    direction VARCHAR(4) NOT NULL,
    PRIMARY KEY (record_id, field, other_record, direction)
);
