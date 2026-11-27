-- Data layer phase 3: row tags, the unit a row is read by, and dataset labels (docs/plans/data-layer.md, "Row tags",
-- "The row's organization", "Who may read a row").
--
-- A row's effective label is the largest of its dataset's floor, its own tags, and the categories of its fields. A category may
-- carry a requirement in settings: a position and a reach. A reader meets it when they hold the position at a unit that covers
-- the row's unit. The row's unit is `dataset_records.unit_id`, or the dataset's own unit when that is empty.

-- The dataset's floor: category ids that every row of the dataset carries, tagged or not. Only a governed change sets it.
ALTER TABLE datasets ADD COLUMN IF NOT EXISTS label_floor TEXT[] NOT NULL DEFAULT '{}';

-- The row's own tags: category ids. The JSON of the row (`_tags`) is the source, so a write sets them with the rest of the data
-- and history shows a change. This column is a copy a list can read in SQL. A trigger keeps the copy exact on every write,
-- through any door, so the copy and the JSON cannot disagree.
ALTER TABLE dataset_records ADD COLUMN IF NOT EXISTS tags TEXT[] NOT NULL DEFAULT '{}';

CREATE OR REPLACE FUNCTION record_tags_of(d JSONB) RETURNS TEXT[] AS $$
    SELECT COALESCE(array_agg(DISTINCT t ORDER BY t), '{}')
    FROM jsonb_array_elements_text(CASE WHEN jsonb_typeof(d -> '_tags') = 'array' THEN d -> '_tags' ELSE '[]'::jsonb END) AS t
$$ LANGUAGE sql IMMUTABLE;

-- A write that leaves `_tags` out keeps the row's tags. Only a write that names `_tags` changes them, so a sync, an import or an
-- update that replaces the JSON cannot lower a row by forgetting its tags.
CREATE OR REPLACE FUNCTION dataset_records_copy_tags() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'UPDATE' AND NOT (NEW.data ? '_tags') AND cardinality(OLD.tags) > 0 THEN
        NEW.data := jsonb_set(NEW.data, '{_tags}', to_jsonb(OLD.tags));
    END IF;
    NEW.tags := record_tags_of(NEW.data);
    RETURN NEW;
END
$$ LANGUAGE plpgsql;

-- A list finds the tagged rows of a table through this index. It holds only tagged rows, so a table with no tags adds nothing.
CREATE INDEX IF NOT EXISTS idx_dataset_records_tagged ON dataset_records (app_slug, table_id) WHERE deleted_at IS NULL AND tags <> '{}';

DROP TRIGGER IF EXISTS trg_dataset_records_tags ON dataset_records;
CREATE TRIGGER trg_dataset_records_tags BEFORE INSERT OR UPDATE OF data ON dataset_records
    FOR EACH ROW EXECUTE FUNCTION dataset_records_copy_tags();

-- A table that declares its unit column may also require it: a row that names no unit is refused. The older function stays,
-- and calls this one with the flag off, so a server that has not been updated yet keeps working.
CREATE OR REPLACE FUNCTION set_record_unit_checked(p_record TEXT, p_unit UUID, p_raw TEXT, p_hierarchy TEXT, p_field TEXT, p_required BOOLEAN) RETURNS void AS $$
BEGIN
    IF p_raw IS NULL THEN
        IF p_required THEN
            RAISE EXCEPTION 'a unit is required' USING ERRCODE = '23514', CONSTRAINT = 'unit_required', COLUMN = p_field, DETAIL = p_hierarchy;
        END IF;
        UPDATE dataset_records SET unit_id = NULL WHERE id = p_record AND unit_id IS NOT NULL;
        RETURN;
    END IF;
    IF p_unit IS NULL OR NOT EXISTS (SELECT 1 FROM organizations o WHERE o.id = p_unit AND o.hierarchy_key = p_hierarchy) THEN
        RAISE EXCEPTION '%', left(p_raw, 100) USING ERRCODE = '23514', CONSTRAINT = 'unit_not_on_list', COLUMN = p_field, DETAIL = p_hierarchy;
    END IF;
    UPDATE dataset_records SET unit_id = p_unit WHERE id = p_record AND unit_id IS DISTINCT FROM p_unit;
END
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION set_record_unit(p_record TEXT, p_unit UUID, p_raw TEXT, p_hierarchy TEXT, p_field TEXT) RETURNS void AS $$
    SELECT set_record_unit_checked(p_record, p_unit, p_raw, p_hierarchy, p_field, FALSE)
$$ LANGUAGE sql;
