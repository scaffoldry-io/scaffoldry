-- What a custom page remembers for one person. A setting is the person's own and is never shown to anyone else.
-- The service keeps one person's settings for one page to 64 KB.
CREATE TABLE IF NOT EXISTS page_settings (
    app_slug VARCHAR(64) NOT NULL,
    page_id VARCHAR(64) NOT NULL,
    eppn VARCHAR(255) NOT NULL,
    key VARCHAR(512) NOT NULL,
    value JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (app_slug, page_id, eppn, key)
);
