-- Data layer phase 10: a row share (docs/plans/data-layer.md, "Sharing a row").
--
-- A share names rows of one table, one person, what the person may do (read, or read and write), and the fields it covers. A share
-- that goes beyond what the grantee could already see needs a justification and an expiry, and a person approves it. A share inside
-- what the grantee could already see is not stored: it needs nothing.
--
-- `kind` is `beyond` (beyond the grantee's rows or role) or `restricted` (a row carries a category the grantee does not meet, so an
-- authorized steward must approve). `assessment` is the server's deterministic pre-check, with a slot for a narrative that an agent or a
-- person adds later (`row-share-assessment.schema.json`). It holds counts, field names and category ids, never a record value. `status`:
--   Requested  waiting for a person to decide
--   Approved   in force until `expires_at`
--   Declined   closed with a reason
--   Withdrawn  ended early by the requester, an owner or a steward
--   Ended      reached its date: the job wrote the ledger entry and told the owners
--   Lapsed     its date came before anybody decided it
-- Access never waits for the job. A read asks `status = 'Approved' AND expires_at > NOW()` directly.
-- Both tables name rows, so they belong to the data crate (`scaffoldry_data::shares`).
CREATE TABLE IF NOT EXISTS row_shares (
    id UUID PRIMARY KEY,
    dataset_id VARCHAR(64) NOT NULL REFERENCES datasets(id),
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    records TEXT[] NOT NULL,
    grantee VARCHAR(255) NOT NULL,
    access VARCHAR(5) NOT NULL CHECK (access IN ('read', 'write')),
    fields TEXT[] NOT NULL,
    kind VARCHAR(10) NOT NULL CHECK (kind IN ('beyond', 'restricted')),
    categories TEXT[] NOT NULL DEFAULT '{}',
    justification TEXT NOT NULL,
    requested_by VARCHAR(255) NOT NULL,
    requested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    assessment JSONB NOT NULL,
    status VARCHAR(10) NOT NULL DEFAULT 'Requested' CHECK (status IN ('Requested', 'Approved', 'Declined', 'Withdrawn', 'Ended', 'Lapsed')),
    decided_by VARCHAR(255),
    decided_at TIMESTAMPTZ,
    decision_note TEXT,
    ended_at TIMESTAMPTZ,
    end_notified_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_row_shares_dataset ON row_shares (dataset_id, status, requested_at);
CREATE INDEX IF NOT EXISTS idx_row_shares_due ON row_shares (expires_at) WHERE status = 'Approved';
CREATE INDEX IF NOT EXISTS idx_row_shares_unnotified ON row_shares (ended_at) WHERE status = 'Ended' AND end_notified_at IS NULL;

-- The lookup the read path uses: one row for each row an approved share covers, keyed by the grantee and the row. It holds rows only
-- while the share is approved, so it stays small. The header decides whether the share is live now.
CREATE TABLE IF NOT EXISTS row_share_rows (
    share_id UUID NOT NULL REFERENCES row_shares(id) ON DELETE CASCADE,
    grantee VARCHAR(255) NOT NULL,
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    record_id VARCHAR(64) NOT NULL,
    PRIMARY KEY (share_id, record_id)
);
CREATE INDEX IF NOT EXISTS idx_row_share_rows_grantee ON row_share_rows (grantee, app_slug, table_id, record_id);
