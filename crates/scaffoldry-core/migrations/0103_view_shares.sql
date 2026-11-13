-- Views phase 6: shared links. A share is a link to one view. Only the hash of its token is stored, and the link is shown once.
CREATE TABLE IF NOT EXISTS view_shares (
    id UUID PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    view_id UUID NOT NULL,
    token_hash CHAR(64) NOT NULL UNIQUE,
    scope VARCHAR(12) NOT NULL,
    allow_download BOOLEAN NOT NULL DEFAULT FALSE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ,
    last_used_at TIMESTAMPTZ,
    use_count BIGINT NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_view_shares_view ON view_shares (view_id);
CREATE INDEX IF NOT EXISTS idx_view_shares_app ON view_shares (app_slug);

-- A count of requests for each key in each whole minute. A key is `share:<token hash>` or `addr:<address>`.
CREATE TABLE IF NOT EXISTS rate_limits (
    key VARCHAR(200) NOT NULL,
    window_start TIMESTAMPTZ NOT NULL,
    count INTEGER NOT NULL,
    PRIMARY KEY (key, window_start)
);
CREATE INDEX IF NOT EXISTS idx_rate_limits_window ON rate_limits (window_start);
