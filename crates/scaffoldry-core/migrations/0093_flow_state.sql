-- Workflows phase 3: each flow's circuit breaker and rate window. A flow that fails 20 runs in a row is turned off here
-- until a new version is saved. A flow starts at most `flows.max_runs_per_minute` runs in one window.
CREATE TABLE IF NOT EXISTS flow_state (
    flow_id VARCHAR(64) PRIMARY KEY,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    disabled_at TIMESTAMPTZ,
    window_start TIMESTAMPTZ,
    window_runs INTEGER NOT NULL DEFAULT 0
);
