-- Policy library phase 5: fetching web sources and noticing change.
-- A url source is checked at most once a day. Three failures in a row mark it unreachable.
ALTER TABLE library_sources ADD COLUMN IF NOT EXISTS last_checked_at TIMESTAMPTZ;
ALTER TABLE library_sources ADD COLUMN IF NOT EXISTS last_status VARCHAR(16);
ALTER TABLE library_sources ADD COLUMN IF NOT EXISTS consecutive_failures INTEGER NOT NULL DEFAULT 0;
ALTER TABLE library_sources ADD COLUMN IF NOT EXISTS next_check_at TIMESTAMPTZ;
-- When a rule's cited passage left its source. The rule keeps its status; the date is shown beside it.
ALTER TABLE library_rules ADD COLUMN IF NOT EXISTS source_changed_at TIMESTAMPTZ;
-- Who marked a policy reviewed last. The next person to mark it must be someone else.
ALTER TABLE library_policies ADD COLUMN IF NOT EXISTS last_reviewed_by VARCHAR(255);
CREATE INDEX IF NOT EXISTS idx_library_sources_due ON library_sources (next_check_at) WHERE kind = 'url';
