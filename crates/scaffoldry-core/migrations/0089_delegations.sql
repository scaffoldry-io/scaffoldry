-- Approvers phase 4: delegation. A position holder names someone to decide for them, for a window and with a reason.
-- position_key NULL means every position the delegator holds. Resolution reads the time, so a delegation ends on
-- ends_at without anyone acting. A revoked one keeps its row, for the record.
CREATE TABLE IF NOT EXISTS delegations (
    id UUID PRIMARY KEY,
    delegator_eppn VARCHAR(255) NOT NULL,
    delegate_eppn VARCHAR(255) NOT NULL,
    position_key VARCHAR(64) REFERENCES position_types(key),
    starts_at TIMESTAMPTZ NOT NULL,
    ends_at TIMESTAMPTZ NOT NULL,
    reason TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_delegations_delegator ON delegations (delegator_eppn);
CREATE INDEX IF NOT EXISTS idx_delegations_delegate ON delegations (delegate_eppn);
