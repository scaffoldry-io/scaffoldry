-- Decision authority phase 1: what a change needs. A unit may raise a check's tier and tighten what a tier requires.
-- Standard search: FIPS 199 impact levels (adopted for tier names); NIST SP 800-53 CM-3, CM-4, AC-5, RA-3.
CREATE TABLE IF NOT EXISTS authority_signals (
    organization_id UUID NOT NULL REFERENCES organizations(id),
    check_name VARCHAR(64) NOT NULL,
    tier VARCHAR(8) NOT NULL CHECK (tier IN ('Low', 'Moderate', 'High')),
    PRIMARY KEY (organization_id, check_name)
);

CREATE TABLE IF NOT EXISTS authority_rows (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id),
    tier VARCHAR(8) NOT NULL CHECK (tier IN ('Low', 'Moderate', 'High')),
    path VARCHAR(16) NOT NULL CHECK (path IN ('direct', 'second_person', 'approver')),
    approver JSONB,
    control_id VARCHAR(32),
    library_policy_key VARCHAR(64),
    library_rule_key VARCHAR(64),
    UNIQUE (organization_id, tier),
    CHECK ((path = 'approver') = (approver IS NOT NULL))
);

CREATE TABLE IF NOT EXISTS authority_changes (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id),
    change JSONB NOT NULL,
    reason TEXT NOT NULL,
    status VARCHAR(10) NOT NULL DEFAULT 'Pending' CHECK (status IN ('Pending', 'Confirmed', 'Rejected', 'Withdrawn')),
    proposed_by VARCHAR(255) NOT NULL,
    proposed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    decided_by VARCHAR(255),
    decided_at TIMESTAMPTZ
);
