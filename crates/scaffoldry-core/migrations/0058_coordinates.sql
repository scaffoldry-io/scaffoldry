-- Access model phase 3: every object has coordinates.
--
-- A workspace, an app, a table and a field each sit at a unit in each hierarchy, in a domain, and at a sensitivity.
-- Apps, tables and fields carry theirs in the manifest, so they change only by proposal. A workspace's are rows.

CREATE TABLE IF NOT EXISTS access_domains (
    key VARCHAR(64) PRIMARY KEY,
    parent_key VARCHAR(64) REFERENCES access_domains(key),
    name VARCHAR(255) NOT NULL,
    status VARCHAR(8) NOT NULL DEFAULT 'Active'
);

-- One root, then the HERM domains, then the HERM capabilities under their domain. The domains and capabilities are the
-- ones in crates/scaffoldry-core/src/standards/herm.rs. A test fails if that file holds one this seed does not.
INSERT INTO access_domains (key, parent_key, name) VALUES
    ('institution', NULL, 'Institution'),
    ('learning-and-teaching', 'institution', 'Learning and Teaching'),
    ('research', 'institution', 'Research'),
    ('engagement', 'institution', 'Engagement'),
    ('strategy-and-governance', 'institution', 'Strategy and Governance'),
    ('enabling-capabilities', 'institution', 'Enabling Capabilities'),
    ('herm:1.2.3', 'learning-and-teaching', 'Curriculum Management'),
    ('herm:2.2.3', 'research', 'Research Grant Administration'),
    ('herm:5.3.1', 'enabling-capabilities', 'Space and Facilities Management')
ON CONFLICT DO NOTHING;

-- `axis` is `unit:<hierarchy key>`, `domain`, or `sensitivity`. A workspace's `academic` unit stays in
-- workspaces.organization_id.
CREATE TABLE IF NOT EXISTS workspace_coordinates (
    workspace_id VARCHAR(64) NOT NULL REFERENCES workspaces(id),
    axis VARCHAR(48) NOT NULL,
    value VARCHAR(64) NOT NULL,
    PRIMARY KEY (workspace_id, axis)
);

-- A change to a workspace's coordinates that widens them waits here until a different person confirms it.
CREATE TABLE IF NOT EXISTS workspace_coordinate_changes (
    id UUID PRIMARY KEY,
    workspace_id VARCHAR(64) NOT NULL REFERENCES workspaces(id),
    coordinates JSONB NOT NULL,
    reason TEXT NOT NULL,
    proposed_by VARCHAR(255) NOT NULL,
    proposed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    status VARCHAR(9) NOT NULL DEFAULT 'Pending',
    decided_by VARCHAR(255),
    decided_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_coordinate_changes_workspace ON workspace_coordinate_changes (workspace_id, status);

-- allowed_affiliations was stored and returned and never read by a decision. A guard (deny_write_for_affiliation and
-- the others) is how a workspace limits who may do what, so the list is removed instead of enforced.
ALTER TABLE workspaces DROP COLUMN IF EXISTS allowed_affiliations;
