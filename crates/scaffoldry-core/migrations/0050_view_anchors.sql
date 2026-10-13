-- Anchors let a view jump to any row of a very large table (views phase 3). Skipping 500,000 rows with OFFSET
-- over 1,000,000 took 3.0 s with the join and 0.4 s on the index alone, against a limit of 0.3 s, so a job
-- samples the sort key of every 1,000th row. A jump starts at the nearest anchor at or before the row it wants
-- and skips at most 999 rows from there. `cursor` is the key of the row just before the anchored row, in the form
-- a page cursor takes, and is null for the first anchor.
CREATE TABLE IF NOT EXISTS view_anchors (
    view_id UUID NOT NULL,
    position INTEGER NOT NULL,
    cursor JSONB,
    PRIMARY KEY (view_id, position)
);

-- What the anchors were built for. A different `spec_hash` means the view's order or filter changed since.
CREATE TABLE IF NOT EXISTS view_anchor_builds (
    view_id UUID PRIMARY KEY,
    spec_hash VARCHAR(64) NOT NULL,
    total BIGINT NOT NULL,
    built_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
