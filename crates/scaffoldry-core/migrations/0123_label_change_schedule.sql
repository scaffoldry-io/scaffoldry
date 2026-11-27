-- Data layer phase 6b: a label change may run on a date (docs/plans/data-layer.md, "Changing a label", steps 4 to 6).
--
-- A decided change is approved for a date and moves to `Scheduled`. It holds the effective date and its communications plan. The plan is
-- approved apart from the change, and nothing is sent before then. A job applies the change when its date comes, through the phase 6
-- apply path. The steward may withdraw it first (`Withdrawn`).
ALTER TABLE label_changes DROP CONSTRAINT IF EXISTS label_changes_status_check;
ALTER TABLE label_changes ADD CONSTRAINT label_changes_status_check CHECK (status IN ('Pending', 'Approved', 'Scheduled', 'Applied', 'Declined', 'Withdrawn'));
ALTER TABLE label_changes DROP CONSTRAINT IF EXISTS label_changes_open_work_check;
ALTER TABLE label_changes ADD CONSTRAINT label_changes_open_work_check CHECK (open_work IN ('now', 'new_work_only', 'date'));
ALTER TABLE label_changes ADD COLUMN IF NOT EXISTS effective_at TIMESTAMPTZ;
ALTER TABLE label_changes ADD COLUMN IF NOT EXISTS plan JSONB;
ALTER TABLE label_changes ADD COLUMN IF NOT EXISTS plan_approved_by VARCHAR(255);
ALTER TABLE label_changes ADD COLUMN IF NOT EXISTS plan_approved_at TIMESTAMPTZ;
CREATE INDEX IF NOT EXISTS idx_label_changes_due ON label_changes (effective_at) WHERE status = 'Scheduled';

-- The people on a plan, one row each, with the notice recorded for them. A row is written when the change is scheduled and holds no
-- notice. `notified_at` is set when the steward approves the plan and the notice is sent. `delivered` is true when the inbox took it.
-- `withdrawn_notified_at` is set when the change is withdrawn after they were told. `audiences` lists why the person is on the plan:
-- lose_access, owners, stewards or compliance. The table names no row and no value. The app and the table are kept here so that a read of
-- the dataset's records finds its upcoming changes with one indexed lookup.
CREATE TABLE IF NOT EXISTS label_change_audience (
    change_id UUID NOT NULL REFERENCES label_changes(id),
    person VARCHAR(255) NOT NULL,
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    audiences TEXT[] NOT NULL,
    notified_at TIMESTAMPTZ,
    delivered BOOLEAN NOT NULL DEFAULT FALSE,
    withdrawn_notified_at TIMESTAMPTZ,
    PRIMARY KEY (change_id, person)
);
CREATE INDEX IF NOT EXISTS idx_label_change_audience_person ON label_change_audience (person, app_slug, table_id);

-- A request to keep access after the date. The steward decides. An approved one holds the grant (`access_grants`) that carries its expiry,
-- and `adds` names the categories the change adds, which the grant waives for this person on this dataset. `in_force` is set when the
-- change is applied, so an exception waives nothing before then.
CREATE TABLE IF NOT EXISTS label_change_exceptions (
    id UUID PRIMARY KEY,
    change_id UUID NOT NULL REFERENCES label_changes(id),
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    person VARCHAR(255) NOT NULL,
    justification TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    adds TEXT[] NOT NULL,
    status VARCHAR(10) NOT NULL DEFAULT 'Requested' CHECK (status IN ('Requested', 'Approved', 'Declined', 'Lapsed', 'Withdrawn')),
    requested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    decided_by VARCHAR(255),
    decided_at TIMESTAMPTZ,
    decision_note TEXT,
    grant_id BIGINT REFERENCES access_grants(id),
    in_force BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE UNIQUE INDEX IF NOT EXISTS uq_label_change_exceptions_live ON label_change_exceptions (change_id, person) WHERE status IN ('Requested', 'Approved');
CREATE INDEX IF NOT EXISTS idx_label_change_exceptions_gate ON label_change_exceptions (person, app_slug, table_id) WHERE in_force;
