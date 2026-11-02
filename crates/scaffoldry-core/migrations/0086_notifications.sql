-- Notifications phase 1: the inbox. A notice carries names and links, never a record value.
CREATE TABLE IF NOT EXISTS notifications (
    id UUID PRIMARY KEY,
    recipient VARCHAR(255) NOT NULL,
    kind VARCHAR(32) NOT NULL,
    app_slug VARCHAR(64),
    title VARCHAR(200) NOT NULL,
    body VARCHAR(500) NOT NULL,
    link JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    read_at TIMESTAMPTZ,
    email_state VARCHAR(10) NOT NULL DEFAULT 'none'
);
CREATE INDEX IF NOT EXISTS idx_notifications_recipient ON notifications (recipient, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_notifications_unread ON notifications (recipient) WHERE read_at IS NULL;
