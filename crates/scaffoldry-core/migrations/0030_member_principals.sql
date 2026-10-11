ALTER TABLE workspace_collaborators ADD COLUMN IF NOT EXISTS principal_type VARCHAR(8) NOT NULL DEFAULT 'user';
ALTER TABLE workspace_collaborators ADD COLUMN IF NOT EXISTS principal_id VARCHAR(255);
UPDATE workspace_collaborators SET principal_id = eppn WHERE principal_id IS NULL;
