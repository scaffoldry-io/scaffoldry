-- Notifications phase 2: mail waiting to be sent, and what became of it.
-- A notice that should be mailed adds a row here in the same transaction that makes the notice. The send_email job
-- claims due rows with FOR UPDATE SKIP LOCKED, so any number of servers may run it.
CREATE TABLE IF NOT EXISTS email_outbox (
    id UUID PRIMARY KEY,
    to_address VARCHAR(320) NOT NULL,
    subject VARCHAR(200) NOT NULL,
    body TEXT NOT NULL,
    state VARCHAR(10) NOT NULL DEFAULT 'queued',
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_error TEXT,
    notification_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    sent_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_email_due ON email_outbox (next_attempt_at) WHERE state = 'queued';
-- The hourly limit counts a person's mail by address.
CREATE INDEX IF NOT EXISTS idx_email_recipient ON email_outbox (lower(to_address), created_at);
