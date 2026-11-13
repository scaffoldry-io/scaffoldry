-- Connections phase 2: when a source changes shape, the sync stops and proposes the change to the dataset. The
-- connection remembers the proposal and the column map the change would give it. Approving the proposal puts that map in
-- place with the new definition, and the next run goes ahead.
ALTER TABLE connections ADD COLUMN IF NOT EXISTS pending_proposal UUID;
ALTER TABLE connections ADD COLUMN IF NOT EXISTS pending_column_map JSONB;
