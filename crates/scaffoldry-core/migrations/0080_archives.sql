-- Lifecycle phase 2. An archived app or workspace is read-only, hidden from the default lists, and kept. A row here is
-- the archive. Unarchiving deletes the row, and the ledger keeps both decisions. Destruction is only by retention.
CREATE TABLE IF NOT EXISTS archives (
    kind VARCHAR(9) NOT NULL CHECK (kind IN ('app', 'workspace')),
    id VARCHAR(64) NOT NULL,
    reason TEXT NOT NULL,
    archived_by VARCHAR(255) NOT NULL,
    archived_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (kind, id)
);

-- Every instance holds the list, so each write can be refused without a query. The counter tells the others.
INSERT INTO cache_versions (name) VALUES ('archives') ON CONFLICT DO NOTHING;
