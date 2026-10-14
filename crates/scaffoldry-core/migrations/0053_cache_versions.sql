-- Stateless server phase 1. One counter for each collection a server instance may hold between requests.
-- A writer bumps the counter in the transaction that writes the collection. Every request reads all the
-- counters in one statement, and an instance that holds an older number reloads that collection.
-- The rows are the closed list of cached collections. A collection that is not here is read every time.
CREATE TABLE IF NOT EXISTS cache_versions (
    name VARCHAR(32) PRIMARY KEY,
    version BIGINT NOT NULL DEFAULT 0
);
INSERT INTO cache_versions (name) VALUES
    ('settings'), ('labels'), ('position_types'), ('organizations'), ('workspaces'),
    ('guards'), ('datasets'), ('relationships'), ('manifests'), ('rules'), ('policy')
ON CONFLICT DO NOTHING;
