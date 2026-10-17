-- Stateless server phase 2. People, groups and appointments are read from the database one question at a time.
-- These indexes answer those questions without a scan.
CREATE INDEX IF NOT EXISTS idx_scim_users_name ON scim_users (lower(user_name));
CREATE INDEX IF NOT EXISTS idx_scim_users_order ON scim_users ((lower(user_name) COLLATE "C"), id);
CREATE INDEX IF NOT EXISTS idx_persons_eppn_lower ON persons (lower(eppn));
CREATE INDEX IF NOT EXISTS idx_roles_person ON roles (person_id);
CREATE INDEX IF NOT EXISTS idx_roles_organization ON roles (organization_id);
