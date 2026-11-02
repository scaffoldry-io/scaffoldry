-- Integrations phase 1: the secrets store. A secret is AES-256-GCM ciphertext under the master key the operator keeps
-- in the file SCAFFOLDRY_MASTER_KEY_FILE names. The key is never in the database. Each write uses a fresh 96-bit
-- nonce, and the row's id is the associated data, so a ciphertext copied to another row does not decrypt.
CREATE TABLE IF NOT EXISTS secrets (
    id UUID PRIMARY KEY,
    name VARCHAR(64) NOT NULL UNIQUE,
    ciphertext BYTEA NOT NULL,
    nonce BYTEA NOT NULL,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    rotated_at TIMESTAMPTZ
);
