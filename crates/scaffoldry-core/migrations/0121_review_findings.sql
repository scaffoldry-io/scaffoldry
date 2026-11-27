-- Data layer phase 5: the AI review writes findings in the same table the content scan uses, with the source `review`.
-- A review finding is one for each field and category, so its detector id is `review:` and the category id, which can be 64
-- characters. The column is widened to hold it. Widening a varchar changes no stored value and rewrites no row.
ALTER TABLE content_findings ALTER COLUMN detector_id TYPE VARCHAR(80);
