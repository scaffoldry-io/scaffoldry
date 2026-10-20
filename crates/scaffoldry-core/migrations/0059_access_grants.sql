-- Access model phase 4: one grant shape. A grant says a holder has a role at an anchor, optionally limited to a domain
-- and a clearance, for a window of time. Grants are never deleted: a revoke sets revoked_at.
CREATE TABLE IF NOT EXISTS access_grants (
    id BIGSERIAL PRIMARY KEY,
    holder_kind VARCHAR(8) NOT NULL CHECK (holder_kind IN ('person', 'position', 'unit', 'group')),
    holder_id VARCHAR(255) NOT NULL,
    role VARCHAR(16) NOT NULL CHECK (role IN ('viewer', 'commenter', 'editor', 'admin', 'owner')),
    hierarchy_key VARCHAR(32) REFERENCES hierarchies(key),
    anchor_kind VARCHAR(16) NOT NULL CHECK (anchor_kind IN ('container', 'unit', 'holder_unit')),
    anchor_id VARCHAR(255),
    depth SMALLINT,
    domain VARCHAR(64) REFERENCES access_domains(key),
    clearance VARCHAR(16),
    valid_from TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    valid_until TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    revoked_by VARCHAR(255),
    source VARCHAR(16) NOT NULL CHECK (source IN ('membership', 'appointment', 'rule', 'request', 'task', 'flow')),
    reason TEXT NOT NULL,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_access_grants_holder ON access_grants (holder_kind, holder_id) WHERE revoked_at IS NULL;

-- A change waits here until a different person confirms it. Every new grant widens access, so every new grant waits.
CREATE TABLE IF NOT EXISTS access_changes (
    id UUID PRIMARY KEY,
    change JSONB NOT NULL,
    widens BOOLEAN NOT NULL,
    reason TEXT NOT NULL,
    status VARCHAR(10) NOT NULL DEFAULT 'Pending' CHECK (status IN ('Pending', 'Confirmed', 'Rejected', 'Withdrawn')),
    proposed_by VARCHAR(255) NOT NULL,
    proposed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    decided_by VARCHAR(255),
    decided_at TIMESTAMPTZ,
    grant_id BIGINT REFERENCES access_grants(id)
);
CREATE INDEX IF NOT EXISTS idx_access_changes_status ON access_changes (status, proposed_at);
