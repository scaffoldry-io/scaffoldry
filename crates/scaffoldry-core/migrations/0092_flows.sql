-- Workflows phase 1: flows run with approved grants and reach records by address.
ALTER TABLE workflow_automations ALTER COLUMN app_slug DROP NOT NULL;
ALTER TABLE workflow_automations ADD COLUMN IF NOT EXISTS workspace_id VARCHAR(64);
ALTER TABLE workflow_rule_versions ALTER COLUMN app_slug DROP NOT NULL;
-- The person whose action started the flow that made this write. The actor is the flow, `flow:{rule_id}`.
ALTER TABLE record_history ADD COLUMN IF NOT EXISTS on_behalf_of VARCHAR(255);
ALTER TABLE app_proposals ADD COLUMN IF NOT EXISTS parties JSONB;

CREATE TABLE IF NOT EXISTS proposal_approvals (
    proposal_id UUID NOT NULL,
    workspace_id VARCHAR(64) NOT NULL,
    decided_by VARCHAR(255),
    decision VARCHAR(8),
    rationale TEXT,
    at TIMESTAMPTZ,
    PRIMARY KEY (proposal_id, workspace_id)
);

-- One row per effect a flow run made, written in the same transaction as the write. A retry finds the key and skips.
CREATE TABLE IF NOT EXISTS flow_effects (
    dedupe_key VARCHAR(200) PRIMARY KEY,
    instance_id VARCHAR(128) NOT NULL,
    at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_flow_effects_instance ON flow_effects (instance_id);
