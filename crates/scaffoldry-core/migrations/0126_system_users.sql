-- Data layer phase 9: system users for automations (docs/plans/data-layer.md, "Automations are system users").
--
-- An automation is a flow. It already acts as the identity `flow:{rule_id}`. A system user is that identity with an owner and grants.
-- A flow with no row here runs as it did before. A flow with a row here reads and writes only what an approved, live grant names.
--
-- `owner` is stored the way a workspace membership is: a person's address, `group:{id}`, or `position:{key}@{unit}`. The people it
-- means are read when they are needed, so a position owner follows the seat.
--
-- A grant names a dataset, an action (`read`, `write` or `create`) and the fields it covers (none for `create`). The approver's reason is
-- kept here and in the ledger. `review_by` is the date the approver chose. A grant is live while `status = 'Active' AND review_by > NOW()`.
-- A run asks that directly, so no job is needed for access. `lapse_notified_at` is set by the run that first meets the lapse, in the same
-- statement that finds it, so the owner is told once.
CREATE TABLE IF NOT EXISTS system_users (
    flow_id VARCHAR(128) PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    name VARCHAR(200) NOT NULL,
    notes TEXT NOT NULL DEFAULT '',
    owner TEXT NOT NULL,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_by VARCHAR(255) NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS system_grants (
    id UUID PRIMARY KEY,
    flow_id VARCHAR(128) NOT NULL REFERENCES system_users(flow_id) ON DELETE CASCADE,
    dataset_id VARCHAR(64) NOT NULL REFERENCES datasets(id),
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    action VARCHAR(6) NOT NULL CHECK (action IN ('read', 'write', 'create')),
    fields TEXT[] NOT NULL DEFAULT '{}',
    reason TEXT NOT NULL,
    approved_by VARCHAR(255) NOT NULL,
    approved_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    review_by TIMESTAMPTZ NOT NULL,
    status VARCHAR(7) NOT NULL DEFAULT 'Active' CHECK (status IN ('Active', 'Revoked')),
    revoked_by VARCHAR(255),
    revoked_at TIMESTAMPTZ,
    revoke_reason TEXT,
    lapse_notified_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_system_grants_flow ON system_grants (flow_id, status, review_by);
