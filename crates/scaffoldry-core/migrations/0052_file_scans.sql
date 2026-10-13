-- Attachments phase 2. When a scan finished, so the Overview can show the last result.
ALTER TABLE file_refs ADD COLUMN IF NOT EXISTS scanned_at TIMESTAMPTZ;
