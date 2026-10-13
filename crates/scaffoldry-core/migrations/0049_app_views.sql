-- Views live apart from the governed manifest. A view is presentation: it cannot grant access, because
-- every read still goes through the row scope and the field visibility rules.
CREATE TABLE IF NOT EXISTS app_views (
    id UUID PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64),
    name VARCHAR(120) NOT NULL,
    kind VARCHAR(12) NOT NULL,
    ownership VARCHAR(13) NOT NULL DEFAULT 'collaborative',
    owner VARCHAR(255),
    config JSONB NOT NULL DEFAULT '{}',
    version INTEGER NOT NULL DEFAULT 1,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_by VARCHAR(255) NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_app_views_app ON app_views (app_slug) WHERE deleted_at IS NULL;
