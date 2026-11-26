-- Data layer phase 2e: request access from a catalog entry (docs/plans/data-layer.md, "Requesting access").
-- A catalog request is an access model phase 13 request. These columns add what it carries and how it is followed.

-- The fields the dataset's request form lists, from a fixed list: reason, project, duration, agreement_reference.
ALTER TABLE datasets ADD COLUMN IF NOT EXISTS request_fields TEXT[] NOT NULL DEFAULT '{reason}';

-- The dataset a request was made on, the answers to its form, when the steward was last reminded, and the reason a person
-- gave when they decided it.
ALTER TABLE access_requests ADD COLUMN IF NOT EXISTS dataset_id VARCHAR(64);
ALTER TABLE access_requests ADD COLUMN IF NOT EXISTS answers JSONB NOT NULL DEFAULT '{}';
ALTER TABLE access_requests ADD COLUMN IF NOT EXISTS reminded_at TIMESTAMPTZ;
ALTER TABLE access_requests ADD COLUMN IF NOT EXISTS decision_note TEXT;
CREATE INDEX IF NOT EXISTS idx_access_requests_dataset_open ON access_requests (created_at) WHERE dataset_id IS NOT NULL AND status = 'Pending';
