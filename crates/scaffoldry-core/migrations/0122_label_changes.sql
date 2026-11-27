-- Data layer phase 6: a label change is proposed, evaluated on request, and decided by a steward (docs/plans/data-layer.md,
-- "Changing a label", steps 1 to 3). A proposal stores what is asked and applies nothing. Only a decision applies it.
--
-- `kind` is `tags` (add or remove tags on named rows) or `floor` (set a dataset's floor). `spec` holds the ask: for tags
-- `{ add, remove, records }`, for a floor `{ floor }`. A change may come from a review finding (`source = 'review'`) or a person.
-- `approvals` is the list of people who approved so far, `[{ by, at, note }]`. A removal of protection needs two. `status`:
--   Pending   waiting for approvals
--   Approved  approved and held: the steward chose "apply for new work only", so it applies when the open work has ended
--   Applied   done, with its ledger entry
--   Declined  closed with a reason
-- The table belongs to the data crate (`scaffoldry_data::changes`), because it names rows.
CREATE TABLE IF NOT EXISTS label_changes (
    id UUID PRIMARY KEY,
    dataset_id VARCHAR(64) NOT NULL REFERENCES datasets(id),
    kind VARCHAR(8) NOT NULL CHECK (kind IN ('tags', 'floor')),
    spec JSONB NOT NULL,
    reason TEXT NOT NULL,
    source VARCHAR(8) NOT NULL CHECK (source IN ('person', 'review')),
    finding_id UUID,
    proposed_by VARCHAR(255) NOT NULL,
    proposed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    status VARCHAR(10) NOT NULL DEFAULT 'Pending' CHECK (status IN ('Pending', 'Approved', 'Applied', 'Declined')),
    approvals JSONB NOT NULL DEFAULT '[]',
    amended JSONB NOT NULL DEFAULT '[]',
    open_work VARCHAR(16) CHECK (open_work IN ('now', 'new_work_only')),
    decided_by VARCHAR(255),
    decided_at TIMESTAMPTZ,
    decision_note TEXT
);
CREATE INDEX IF NOT EXISTS idx_label_changes_dataset ON label_changes (dataset_id, status, proposed_at);
-- A finding makes one proposal that is still alive.
CREATE UNIQUE INDEX IF NOT EXISTS uq_label_changes_finding ON label_changes (finding_id) WHERE finding_id IS NOT NULL AND status IN ('Pending', 'Approved');
