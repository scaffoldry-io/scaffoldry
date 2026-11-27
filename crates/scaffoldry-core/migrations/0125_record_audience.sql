-- Data layer phase 3b: audience fields and conditions (docs/plans/data-layer.md, "Audience fields" and "Conditions").
--
-- A table's manifest marks fields as audience fields. A copy of the policy sits on the dataset, as `key_spec` and `unit_spec` do, so a
-- write reads it in its own transaction. The manifest stays the source.
ALTER TABLE datasets ADD COLUMN IF NOT EXISTS audience_spec JSONB;

-- The names in the audience fields of each row, one row for each field and name, written in the same transaction as the record by the
-- data crate (`scaffoldry_data::audience`). `kind` is person, position, unit or group, read from the shape of the name. `principal` is
-- the name in lower case. A person's name is an address, a position is `key@unit-id`, a unit is its id, a group is `group:id`.
-- `accepted` is false for a unit the policy asks to accept first. Nothing reads a row that is not accepted. A trashed record keeps its
-- rows, and the read that joins the record's own table leaves it out.
CREATE TABLE IF NOT EXISTS record_audience (
    app_slug TEXT NOT NULL,
    table_id TEXT NOT NULL,
    record_id TEXT NOT NULL,
    field TEXT NOT NULL,
    kind VARCHAR(8) NOT NULL CHECK (kind IN ('person', 'position', 'unit', 'group')),
    principal TEXT NOT NULL,
    accepted BOOLEAN NOT NULL DEFAULT TRUE,
    accepted_by TEXT,
    accepted_at TIMESTAMPTZ,
    PRIMARY KEY (record_id, field, principal)
);

-- "Rows naming any of these principals": a list reads this in the order of the principal, and answers from the index alone.
CREATE INDEX IF NOT EXISTS idx_record_audience_principal ON record_audience (app_slug, table_id, principal, field, record_id) WHERE accepted;

-- The units a steward has yet to accept.
CREATE INDEX IF NOT EXISTS idx_record_audience_waiting ON record_audience (app_slug, table_id, record_id) WHERE NOT accepted;

-- Conditions on the person record (data layer phase 3b). `conditions` is a small map from a condition id to the last day it holds, as
-- YYYY-MM-DD. It is read once with the person, and a read compares the date. `condition_sources` holds, for each condition, the owner,
-- the source (`manual` or `sync`), who set it and when. Nothing here is a job: a lapse is a date that has passed.
ALTER TABLE persons ADD COLUMN IF NOT EXISTS conditions JSONB NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE persons ADD COLUMN IF NOT EXISTS condition_sources JSONB NOT NULL DEFAULT '{}'::jsonb;
