-- Data layer phase 2c: the row's unit column (docs/plans/data-layer.md, "The row's organization").
-- A table names one of its Unit fields as the unit column. The name and the field's hierarchy are copied to the dataset, so a
-- write can read them in its own transaction. The manifest stays the source.
ALTER TABLE datasets ADD COLUMN IF NOT EXISTS unit_spec JSONB;

-- The unit of a row, copied from the unit field when the row is written. A row with no unit is the dataset's own. The value
-- is a unit id and nothing more: a unit that is retired later keeps its rows, and nothing here follows it.
ALTER TABLE dataset_records ADD COLUMN IF NOT EXISTS unit_id UUID;

-- A list filtered by unit reads this index in the order of the page. It holds only rows that have a unit, so a table that
-- names no unit column adds nothing to it.
CREATE INDEX IF NOT EXISTS idx_dataset_records_unit
    ON dataset_records (app_slug, table_id, unit_id, created_at, id)
    WHERE deleted_at IS NULL AND unit_id IS NOT NULL;

-- Sets the unit of a record, or clears it when the value is blank. A value that is not a unit of the field's hierarchy is
-- refused as a check violation of `unit_not_on_list`. The message is the value as it was sent, the column is the field, and
-- the detail is the hierarchy. It names no unit.
CREATE OR REPLACE FUNCTION set_record_unit(p_record TEXT, p_unit UUID, p_raw TEXT, p_hierarchy TEXT, p_field TEXT) RETURNS void AS $$
BEGIN
    IF p_raw IS NULL THEN
        UPDATE dataset_records SET unit_id = NULL WHERE id = p_record AND unit_id IS NOT NULL;
        RETURN;
    END IF;
    IF p_unit IS NULL OR NOT EXISTS (SELECT 1 FROM organizations o WHERE o.id = p_unit AND o.hierarchy_key = p_hierarchy) THEN
        RAISE EXCEPTION '%', left(p_raw, 100) USING ERRCODE = '23514', CONSTRAINT = 'unit_not_on_list', COLUMN = p_field, DETAIL = p_hierarchy;
    END IF;
    UPDATE dataset_records SET unit_id = p_unit WHERE id = p_record AND unit_id IS DISTINCT FROM p_unit;
END
$$ LANGUAGE plpgsql;
