CREATE TABLE IF NOT EXISTS record_comments (
    id UUID PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    record_id VARCHAR(64) NOT NULL,
    parent_id UUID,
    author VARCHAR(255) NOT NULL,
    body TEXT NOT NULL,
    mentions TEXT[] NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    edited_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_comments_record ON record_comments (record_id, created_at);
