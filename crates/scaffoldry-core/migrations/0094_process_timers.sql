-- Process v2 phase 3: the timers of waiting user tasks. Entering a step writes its timers in the same transaction. The
-- `fire_timers` job claims the due ones with FOR UPDATE SKIP LOCKED and marks them fired. A decision cancels the
-- instance's pending timers in its own transaction. `state` is pending, fired or canceled.
CREATE TABLE IF NOT EXISTS process_timers (
    id UUID PRIMARY KEY,
    instance_id VARCHAR(128) NOT NULL,
    step_id VARCHAR(64) NOT NULL,
    action JSONB NOT NULL,
    fire_at TIMESTAMPTZ NOT NULL,
    state VARCHAR(8) NOT NULL DEFAULT 'pending',
    fired_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_process_timers_due ON process_timers (fire_at) WHERE state = 'pending';
CREATE INDEX IF NOT EXISTS idx_process_timers_instance ON process_timers (instance_id) WHERE state = 'pending';
