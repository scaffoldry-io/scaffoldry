-- Integrations phase 4: webhooks. A delivery is queued in the transaction of the write it reports, and sent by the
-- deliver_webhooks job. A webhook that fails 20 times in a row is turned off until an owner turns it on again.
CREATE TABLE IF NOT EXISTS webhook_deliveries (
    id UUID PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    webhook_id VARCHAR(64) NOT NULL,
    event_id UUID NOT NULL,
    payload JSONB NOT NULL,
    state VARCHAR(10) NOT NULL DEFAULT 'queued',
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_status INTEGER,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    delivered_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_webhook_due ON webhook_deliveries (next_attempt_at) WHERE state = 'queued';
-- The call log reads one webhook's newest deliveries; the purge keeps 200 of them.
CREATE INDEX IF NOT EXISTS idx_webhook_log ON webhook_deliveries (app_slug, webhook_id, created_at DESC);

CREATE TABLE IF NOT EXISTS webhook_state (
    app_slug VARCHAR(64) NOT NULL,
    webhook_id VARCHAR(64) NOT NULL,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    disabled_at TIMESTAMPTZ,
    PRIMARY KEY (app_slug, webhook_id)
);
