-- AI tools sign in as themselves (external apps phase 8).
-- A session is one sign-in of an agent client by one person. It has an end, a level the person chose, and the one sentence the
-- tool says it is doing. Its refresh tokens rotate: each is used once, and a second use ends the session.
ALTER TABLE oauth_codes ADD COLUMN IF NOT EXISTS access VARCHAR(8);

CREATE TABLE IF NOT EXISTS oauth_sessions (
    session_id UUID PRIMARY KEY,
    client_id VARCHAR(64) NOT NULL,
    eppn VARCHAR(255) NOT NULL,
    access VARCHAR(8) NOT NULL,
    task VARCHAR(200),
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_oauth_sessions_person ON oauth_sessions (eppn, client_id);

ALTER TABLE oauth_tokens ADD COLUMN IF NOT EXISTS session_id UUID;
CREATE INDEX IF NOT EXISTS idx_oauth_tokens_session ON oauth_tokens (session_id);

CREATE TABLE IF NOT EXISTS oauth_refresh (
    refresh_hash CHAR(64) PRIMARY KEY,
    session_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    used_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_oauth_refresh_session ON oauth_refresh (session_id);
