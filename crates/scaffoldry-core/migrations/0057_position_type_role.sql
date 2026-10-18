-- A position type may grant a workspace role to its holders, for workspaces at and below the holder's unit.
-- Null means it grants nothing, which is how every existing type stays.
ALTER TABLE position_types ADD COLUMN IF NOT EXISTS workspace_role VARCHAR(16);
