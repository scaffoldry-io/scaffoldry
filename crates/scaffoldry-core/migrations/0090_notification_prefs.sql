-- Notifications phase 3: each person's channel per kind. A missing row means the institution's default
-- (the setting notifications.defaults), and with no default, both: the inbox and mail.
CREATE TABLE IF NOT EXISTS notification_prefs (
    eppn VARCHAR(255) NOT NULL,
    kind VARCHAR(32) NOT NULL,
    channel VARCHAR(12) NOT NULL,
    PRIMARY KEY (eppn, kind)
);
-- The digest reads each person's pending notices.
CREATE INDEX IF NOT EXISTS idx_notifications_digest ON notifications (recipient) WHERE email_state = 'digest' AND read_at IS NULL;
