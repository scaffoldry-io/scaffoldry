-- Access model phase 10: a grant is reviewed now and then. The clock runs from the last review, or from creation.
ALTER TABLE access_grants ADD COLUMN IF NOT EXISTS reviewed_at TIMESTAMPTZ;
ALTER TABLE access_grants ADD COLUMN IF NOT EXISTS reviewed_by VARCHAR(255);
