-- Guests phase 1: people outside the directory who prove they own an email address and may use one form.
-- A guest's identity inside the system is `guest:{id}@guest.invalid`. It is never a directory person.
CREATE TABLE IF NOT EXISTS guests (
    id UUID PRIMARY KEY,
    email VARCHAR(320) NOT NULL UNIQUE,
    email_verified_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ,
    disabled_at TIMESTAMPTZ
);

-- A sign-in link, good once for 15 minutes. Only the hash of its token is stored.
CREATE TABLE IF NOT EXISTS guest_links (
    token_hash CHAR(64) PRIMARY KEY,
    guest_id UUID NOT NULL REFERENCES guests(id),
    app_slug VARCHAR(64) NOT NULL,
    form_id UUID NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_guest_links_guest ON guest_links (guest_id);
