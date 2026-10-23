-- An outside app is a registered client with a ceiling a person approved (external apps phase 1).
CREATE TABLE IF NOT EXISTS app_clients (
    client_id VARCHAR(64) PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    operator VARCHAR(255) NOT NULL,
    kind VARCHAR(16) NOT NULL,
    secret_hash CHAR(64),
    redirect_uris TEXT[] NOT NULL,
    ceiling JSONB NOT NULL,
    status VARCHAR(10) NOT NULL DEFAULT 'Pending',
    proposed_by VARCHAR(255) NOT NULL,
    proposed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    decided_by VARCHAR(255),
    decided_at TIMESTAMPTZ,
    disabled_by VARCHAR(255),
    disabled_at TIMESTAMPTZ
);
