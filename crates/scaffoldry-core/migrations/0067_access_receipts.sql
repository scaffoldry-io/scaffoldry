-- Access model phase 8: a receipt for every denial, every privileged action and every read that returns a protected
-- field. A receipt holds ids, never record values. Batches seal receipts with a hash chain.
CREATE TABLE IF NOT EXISTS access_receipts (
    id BIGSERIAL PRIMARY KEY,
    at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    actor VARCHAR(255) NOT NULL,
    acting_as VARCHAR(255),
    action VARCHAR(48) NOT NULL,
    object_kind VARCHAR(16) NOT NULL,
    object_id VARCHAR(255) NOT NULL,
    outcome VARCHAR(8) NOT NULL,
    grant_ids BIGINT[] NOT NULL DEFAULT '{}',
    policy_ids TEXT[] NOT NULL DEFAULT '{}',
    narrowed_by TEXT[] NOT NULL DEFAULT '{}',
    counters JSONB NOT NULL,
    batch_id BIGINT
);
CREATE INDEX IF NOT EXISTS idx_access_receipts_object ON access_receipts (object_kind, object_id, at);
CREATE INDEX IF NOT EXISTS idx_access_receipts_actor ON access_receipts (actor, at);
CREATE INDEX IF NOT EXISTS idx_access_receipts_unsealed ON access_receipts (id) WHERE batch_id IS NULL;

CREATE TABLE IF NOT EXISTS access_receipt_batches (
    id BIGSERIAL PRIMARY KEY,
    first_id BIGINT NOT NULL,
    last_id BIGINT NOT NULL,
    hash CHAR(64) NOT NULL,
    previous_hash CHAR(64) NOT NULL,
    ledger_sequence BIGINT
);
