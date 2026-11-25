-- MCP toolsets phase 2 (docs/plans/mcp-toolsets.md).
-- The toolsets beyond core that a person ticked at consent. An agent code carries them to the token endpoint, and the
-- session keeps them so a refreshed token has the same ones. NULL means every toolset: rows made before this stay NULL.
ALTER TABLE oauth_codes ADD COLUMN IF NOT EXISTS toolsets TEXT[];
ALTER TABLE oauth_sessions ADD COLUMN IF NOT EXISTS toolsets TEXT[];
