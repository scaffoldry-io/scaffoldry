-- Data layer phase 2d: the catalog (docs/plans/data-layer.md, "The catalog: find what exists and ask for it").
-- A catalog entry is a row of `datasets`. The name, the fields and the owner stay in the manifest, or in `published_datasets`,
-- so there is one source for each. This migration adds the policy an entry carries.

-- Who may discover the dataset. A JSON string, "everyone" (the default) or "hidden", or an object with a list of unit ids and
-- a list of position keys.
ALTER TABLE datasets ADD COLUMN IF NOT EXISTS discoverability JSONB NOT NULL DEFAULT '"everyone"';

-- Fields shown as "a restricted field": no name, no type, no category, no sample.
ALTER TABLE datasets ADD COLUMN IF NOT EXISTS restricted_fields TEXT[] NOT NULL DEFAULT '{}';

-- The steward, as a position key. A seat, so it survives people changing. Null until one is set.
ALTER TABLE datasets ADD COLUMN IF NOT EXISTS steward_position VARCHAR(64);

-- A published dataset is a dataset too. Its row here holds its id (`published_id` is the id in `published_datasets`), so it
-- has a dataset id and a policy like any other. Its `app_slug` is a name no app can have.
ALTER TABLE datasets ADD COLUMN IF NOT EXISTS published_id VARCHAR(64);
CREATE UNIQUE INDEX IF NOT EXISTS uq_datasets_published ON datasets (published_id) WHERE published_id IS NOT NULL;

INSERT INTO datasets (id, app_slug, table_id, published_id)
SELECT 'ds_' || replace(gen_random_uuid()::text, '-', ''), '~published', p.id, p.id FROM published_datasets p
ON CONFLICT (app_slug, table_id) DO NOTHING;
