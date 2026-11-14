-- Guests phase 3: the link in each mail to a guest that stops these mails. Only the hash of the token is stored. The choice
-- itself is a row of `notification_prefs` for the guest's address.
CREATE TABLE IF NOT EXISTS guest_stops (
    token_hash CHAR(64) PRIMARY KEY,
    guest_id UUID NOT NULL REFERENCES guests(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
