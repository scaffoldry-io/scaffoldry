-- Workflows phase 4: while a user task is open, each assignee holds a grant on that one record: read of `read_fields`,
-- write of `write_fields`. The grant closes in the transaction that closes, decides, escalates away, reassigns or
-- cancels the step. `state` is open or closed.
CREATE TABLE IF NOT EXISTS task_grants (
    instance_id VARCHAR(128) NOT NULL,
    step_id VARCHAR(64) NOT NULL,
    person VARCHAR(255) NOT NULL,
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    record_id VARCHAR(64) NOT NULL,
    read_fields TEXT[] NOT NULL,
    write_fields TEXT[] NOT NULL,
    state VARCHAR(8) NOT NULL DEFAULT 'open',
    closed_at TIMESTAMPTZ,
    PRIMARY KEY (instance_id, step_id, person)
);
CREATE INDEX IF NOT EXISTS idx_task_grants_open ON task_grants (lower(person), app_slug, record_id) WHERE state = 'open';
