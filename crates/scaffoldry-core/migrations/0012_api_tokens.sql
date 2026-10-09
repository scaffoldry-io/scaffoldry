CREATE TABLE IF NOT EXISTS api_tokens (
    token_hash CHAR(64) PRIMARY KEY,
    id UUID NOT NULL UNIQUE,
    kind VARCHAR(16) NOT NULL,
    eppn VARCHAR(255) NOT NULL,
    label VARCHAR(255) NOT NULL DEFAULT '',
    original_admin VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    last_used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_api_tokens_eppn ON api_tokens(eppn);
