-- Process v2, phase 1: one row for each transition of a process instance, written in the transaction that makes it.
-- An event holds ids, step ids and counts (the decision, how many approvers), never a record value.
CREATE TABLE IF NOT EXISTS process_events (
    id BIGSERIAL PRIMARY KEY,
    instance_id VARCHAR(128) NOT NULL,
    app_slug VARCHAR(64) NOT NULL,
    rule_id VARCHAR(64) NOT NULL,
    rule_version INTEGER NOT NULL,
    step_id VARCHAR(64),
    kind VARCHAR(20) NOT NULL,
    actor VARCHAR(255),
    detail JSONB NOT NULL DEFAULT '{}',
    at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_process_events_instance ON process_events (instance_id, id);
CREATE INDEX IF NOT EXISTS idx_process_events_rule ON process_events (rule_id, kind, at);
-- The analytics read one app's events.
CREATE INDEX IF NOT EXISTS idx_process_events_app ON process_events (app_slug, rule_id, step_id);
