-- The catalog's by-value dataset relationships are gone. A link to a dataset record is a link field of an app
-- (`links.md`, `connections.md` phase 5), and a value join in an app converts to one.
DROP TABLE IF EXISTS dataset_relationships;
DELETE FROM cache_versions WHERE name = 'relationships';
