ALTER TABLE workspaces ADD COLUMN IF NOT EXISTS organization_id UUID REFERENCES organizations(id);
CREATE INDEX IF NOT EXISTS idx_workspaces_organization ON workspaces(organization_id);

INSERT INTO organizations (id, name, code, org_type, parent_id)
VALUES ('00000000-0000-0000-0000-000000000001', 'Institution', 'INST', 'Institution', NULL)
ON CONFLICT (code) DO NOTHING;

UPDATE workspaces
SET organization_id = (SELECT id FROM organizations WHERE code = 'INST')
WHERE organization_id IS NULL;
