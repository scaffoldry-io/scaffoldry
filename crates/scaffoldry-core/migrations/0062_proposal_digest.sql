-- Proposal preview phase 1: a proposal carries the digest of its content, and an approval names it.
-- A proposal made before this migration has none stored. The service works it out from the stored content, which cannot change.
ALTER TABLE app_proposals ADD COLUMN IF NOT EXISTS digest CHAR(64);
