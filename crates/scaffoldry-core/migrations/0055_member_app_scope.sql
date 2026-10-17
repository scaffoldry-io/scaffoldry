-- A membership may be limited to one app. An empty app means the whole workspace.
ALTER TABLE workspace_collaborators ADD COLUMN IF NOT EXISTS app_slug VARCHAR(64) NOT NULL DEFAULT '';
ALTER TABLE workspace_collaborators DROP CONSTRAINT IF EXISTS uq_workspace_collab;
ALTER TABLE workspace_collaborators ADD CONSTRAINT uq_workspace_collab UNIQUE (workspace_id, eppn, app_slug);
