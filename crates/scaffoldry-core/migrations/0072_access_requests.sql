-- Access model phase 13: asking for access. A person who is refused can ask. A rule approves the routine asks alone and a
-- person decides the rest. `session_id` is the AI tool session that filed a request, so a `self` approval widens that
-- session and no other.
CREATE TABLE IF NOT EXISTS access_requests (
    id UUID PRIMARY KEY,
    requester VARCHAR(255) NOT NULL,
    filed_by_client VARCHAR(64),
    session_id UUID,
    task VARCHAR(200),
    object_kind VARCHAR(16) NOT NULL CHECK (object_kind IN ('workspace', 'app')),
    object_id VARCHAR(255) NOT NULL,
    role VARCHAR(16) NOT NULL CHECK (role IN ('viewer', 'commenter', 'editor', 'admin', 'owner')),
    clearance VARCHAR(16),
    reason TEXT NOT NULL,
    days SMALLINT NOT NULL CHECK (days BETWEEN 1 AND 365),
    route VARCHAR(10) NOT NULL CHECK (route IN ('auto', 'self', 'approver')),
    rule_id UUID,
    status VARCHAR(20) NOT NULL CHECK (status IN ('Waiting for person', 'Pending', 'Approved', 'Rejected', 'Withdrawn', 'Expired')),
    decided_by VARCHAR(255),
    decided_at TIMESTAMPTZ,
    grant_id BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_access_requests_requester ON access_requests (requester, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_access_requests_open ON access_requests (status, created_at) WHERE status IN ('Waiting for person', 'Pending');

CREATE TABLE IF NOT EXISTS access_request_rules (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id),
    rule JSONB NOT NULL,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    retired_at TIMESTAMPTZ
);

-- The default rule, at the institution's root. It has a fixed id, so creating it again changes nothing, and retiring it is
-- a retirement that stays.
INSERT INTO access_request_rules (id, organization_id, rule, created_by)
SELECT '00000000-0000-0000-0000-0000000000a1', id,
       '{"max_role":"viewer","max_sensitivity":"Internal","visibilities":["departmental","institutional"],"own_unit":true,"max_days":30}'::jsonb,
       'migration'
FROM organizations WHERE id = '00000000-0000-0000-0000-000000000001'
ON CONFLICT (id) DO NOTHING;
