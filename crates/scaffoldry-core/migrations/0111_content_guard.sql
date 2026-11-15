-- Sensitive content phase 3: a write, an import or a file is checked against the detectors. A finding may say how much of a field was read,
-- and an attachment says whether its text was read.
ALTER TABLE content_findings ADD COLUMN IF NOT EXISTS note TEXT;

-- `clean` and `found` say the text was read. `unsupported` is a file type the scan does not read. `skipped` is a file not read yet, or one no detector looked at.
ALTER TABLE file_refs ADD COLUMN IF NOT EXISTS content_scan_state VARCHAR(12) NOT NULL DEFAULT 'skipped';
