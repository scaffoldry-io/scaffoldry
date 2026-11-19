-- Admin console phase 7: the audit explorer filters the ledger by fixed platform columns only.
-- Each index ends in `sequence DESC`, so a filtered page is read newest first without a sort.
CREATE INDEX IF NOT EXISTS idx_ledger_decision ON governance_ledger (decision_type, sequence DESC);
CREATE INDEX IF NOT EXISTS idx_ledger_app ON governance_ledger (app_slug, sequence DESC);
CREATE INDEX IF NOT EXISTS idx_ledger_control ON governance_ledger (oscal_control_id, sequence DESC);
