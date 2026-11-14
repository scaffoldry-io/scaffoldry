-- Guests phase 4: operating guests. An erased guest keeps its row, so the records it made keep a submitter, but its address is a hash.
ALTER TABLE guests ADD COLUMN IF NOT EXISTS erased_at TIMESTAMPTZ;

-- Records made through a guest form carry `_submitter` and `_form`. Only those records are in these indexes, so no other write
-- pays for them. The guest routes, the console and the 90 day purge all look records up by them.
CREATE INDEX IF NOT EXISTS idx_dataset_records_guest_submitter ON dataset_records ((data->>'_submitter'), (data->>'_form')) WHERE data ? '_submitter';
CREATE INDEX IF NOT EXISTS idx_dataset_records_guest_form ON dataset_records (app_slug, (data->>'_form')) WHERE data ? '_form';
