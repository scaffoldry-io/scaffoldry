ALTER TABLE app_manifests ADD COLUMN IF NOT EXISTS workspace_id VARCHAR(64) REFERENCES workspaces(id);
CREATE INDEX IF NOT EXISTS idx_app_manifests_workspace ON app_manifests(workspace_id);
