-- Access model phase 14: AI tools draft, people decide. A change an AI tool drafted records the person it was drafted for,
-- the tool and the task, and ends seven days after it is stored.
ALTER TABLE access_changes ADD COLUMN IF NOT EXISTS proposed_via VARCHAR(8) NOT NULL DEFAULT 'person' CHECK (proposed_via IN ('person', 'agent'));
ALTER TABLE access_changes ADD COLUMN IF NOT EXISTS drafted_for VARCHAR(255);
ALTER TABLE access_changes ADD COLUMN IF NOT EXISTS via_client VARCHAR(64);
ALTER TABLE access_changes ADD COLUMN IF NOT EXISTS task VARCHAR(200);
ALTER TABLE access_changes ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ;
