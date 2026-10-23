-- Signing in to an outside app (external apps phase 2).
-- A code is single use and lives 60 seconds. It is stored as a hash.
CREATE TABLE IF NOT EXISTS oauth_codes (
    code_hash CHAR(64) PRIMARY KEY,
    client_id VARCHAR(64) NOT NULL,
    eppn VARCHAR(255) NOT NULL,
    redirect_uri TEXT NOT NULL,
    code_challenge VARCHAR(128) NOT NULL,
    ceiling_digest CHAR(64) NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at TIMESTAMPTZ
);

-- What a user agreed to, per client. Consent is asked again when the ceiling's digest changes.
CREATE TABLE IF NOT EXISTS oauth_consents (
    eppn VARCHAR(255) NOT NULL,
    client_id VARCHAR(64) NOT NULL,
    ceiling_digest CHAR(64) NOT NULL,
    granted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (eppn, client_id)
);

-- Which client, and which code, a token of kind `client` came from.
CREATE TABLE IF NOT EXISTS oauth_tokens (
    token_hash CHAR(64) PRIMARY KEY,
    client_id VARCHAR(64) NOT NULL,
    eppn VARCHAR(255) NOT NULL,
    code_hash CHAR(64) NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_oauth_tokens_client ON oauth_tokens (client_id, eppn);
CREATE INDEX IF NOT EXISTS idx_oauth_tokens_code ON oauth_tokens (code_hash);
