-- Access model phase 2: Hierarchies as data, one closure, one notion of place.
-- Standard search: NIST SP 800-162, SP 800-178, NGAC, INCITS 359, RFC 7643.

CREATE TABLE IF NOT EXISTS hierarchies (
    key VARCHAR(32) PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    kind VARCHAR(8) NOT NULL,
    source VARCHAR(16) NOT NULL,
    status VARCHAR(8) NOT NULL DEFAULT 'Active',
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    retired_at TIMESTAMPTZ,
    CHECK (kind IN ('units', 'people')),
    CHECK (source IN ('api', 'scim', 'import', 'connection')),
    CHECK (status IN ('Active', 'Retired'))
);

INSERT INTO hierarchies (key, name, kind, source, created_by)
VALUES ('academic', 'Academic and administrative', 'units', 'api', 'system')
ON CONFLICT DO NOTHING;

ALTER TABLE organizations ADD COLUMN IF NOT EXISTS hierarchy_key VARCHAR(32) NOT NULL DEFAULT 'academic' REFERENCES hierarchies(key);
CREATE UNIQUE INDEX IF NOT EXISTS idx_one_root_per_hierarchy ON organizations (hierarchy_key) WHERE parent_id IS NULL;

-- Trigger enforcing that a unit's parent belongs to the same hierarchy
CREATE OR REPLACE FUNCTION check_org_parent_hierarchy() RETURNS TRIGGER AS $$
BEGIN
    IF NEW.parent_id IS NOT NULL THEN
        IF (SELECT hierarchy_key FROM organizations WHERE id = NEW.parent_id) <> NEW.hierarchy_key THEN
            RAISE EXCEPTION 'Cross-hierarchy parent: parent % does not belong to hierarchy %', NEW.parent_id, NEW.hierarchy_key;
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_check_org_parent_hierarchy ON organizations;
CREATE TRIGGER trg_check_org_parent_hierarchy
    BEFORE INSERT OR UPDATE OF parent_id, hierarchy_key ON organizations
    FOR EACH ROW
    EXECUTE FUNCTION check_org_parent_hierarchy();

CREATE TABLE IF NOT EXISTS org_closure (
    hierarchy_key VARCHAR(32) NOT NULL REFERENCES hierarchies(key),
    ancestor UUID NOT NULL REFERENCES organizations(id),
    descendant UUID NOT NULL REFERENCES organizations(id),
    depth SMALLINT NOT NULL,
    PRIMARY KEY (ancestor, descendant)
);
CREATE INDEX IF NOT EXISTS idx_org_closure_descendant ON org_closure (descendant, depth);

-- Fill org_closure from existing organizations
-- 1. Self-rows at depth 0
INSERT INTO org_closure (hierarchy_key, ancestor, descendant, depth)
SELECT hierarchy_key, id, id, 0
FROM organizations
ON CONFLICT (ancestor, descendant) DO NOTHING;

-- 2. Ancestor rows via recursive CTE
WITH RECURSIVE org_tree AS (
    SELECT
        parent_id AS ancestor,
        id AS descendant,
        1::smallint AS depth,
        hierarchy_key
    FROM organizations
    WHERE parent_id IS NOT NULL

    UNION ALL

    SELECT
        o.parent_id AS ancestor,
        t.descendant,
        (t.depth + 1)::smallint AS depth,
        t.hierarchy_key
    FROM org_tree t
    JOIN organizations o ON o.id = t.ancestor
    WHERE o.parent_id IS NOT NULL
)
INSERT INTO org_closure (hierarchy_key, ancestor, descendant, depth)
SELECT hierarchy_key, ancestor, descendant, depth
FROM org_tree
ON CONFLICT (ancestor, descendant) DO NOTHING;
