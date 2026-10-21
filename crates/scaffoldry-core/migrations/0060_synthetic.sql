-- Access model phase 5: synthetic institution, isolation, and reference evaluator.

ALTER TABLE scim_users ADD COLUMN IF NOT EXISTS synthetic BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE persons ADD COLUMN IF NOT EXISTS synthetic BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE organizations ADD COLUMN IF NOT EXISTS synthetic BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE workspaces ADD COLUMN IF NOT EXISTS synthetic BOOLEAN NOT NULL DEFAULT FALSE;

CREATE OR REPLACE FUNCTION refuse_synthetic_downgrade()
RETURNS TRIGGER AS $$
BEGIN
    IF OLD.synthetic = TRUE AND NEW.synthetic = FALSE THEN
        RAISE EXCEPTION 'Cannot change synthetic from true to false';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_scim_users_synthetic ON scim_users;
CREATE TRIGGER trg_scim_users_synthetic
    BEFORE UPDATE ON scim_users
    FOR EACH ROW EXECUTE FUNCTION refuse_synthetic_downgrade();

DROP TRIGGER IF EXISTS trg_persons_synthetic ON persons;
CREATE TRIGGER trg_persons_synthetic
    BEFORE UPDATE ON persons
    FOR EACH ROW EXECUTE FUNCTION refuse_synthetic_downgrade();

DROP TRIGGER IF EXISTS trg_organizations_synthetic ON organizations;
CREATE TRIGGER trg_organizations_synthetic
    BEFORE UPDATE ON organizations
    FOR EACH ROW EXECUTE FUNCTION refuse_synthetic_downgrade();

DROP TRIGGER IF EXISTS trg_workspaces_synthetic ON workspaces;
CREATE TRIGGER trg_workspaces_synthetic
    BEFORE UPDATE ON workspaces
    FOR EACH ROW EXECUTE FUNCTION refuse_synthetic_downgrade();

INSERT INTO hierarchies (key, name, kind, source, created_by)
VALUES ('supervisory', 'Supervisory management', 'units', 'api', 'system')
ON CONFLICT (key) DO NOTHING;

