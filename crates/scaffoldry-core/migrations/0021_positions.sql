CREATE TABLE IF NOT EXISTS position_types (
    key VARCHAR(64) PRIMARY KEY,
    name VARCHAR(128) NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    org_types JSONB NOT NULL,
    max_holders INTEGER NOT NULL DEFAULT 1,
    retired_at TIMESTAMPTZ,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
ALTER TABLE roles ADD COLUMN IF NOT EXISTS position_key VARCHAR(64) REFERENCES position_types(key);
CREATE INDEX IF NOT EXISTS idx_roles_position ON roles (position_key, organization_id) WHERE position_key IS NOT NULL;
