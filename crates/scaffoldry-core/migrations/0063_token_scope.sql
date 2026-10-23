-- A token may be narrower than its owner. Null means the owner's full rights.
ALTER TABLE api_tokens ADD COLUMN IF NOT EXISTS scope JSONB;
