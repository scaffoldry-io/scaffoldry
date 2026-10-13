-- Attachments phase 1. A file's bytes live on disk under its SHA-256. These tables hold what the disk cannot:
-- who uploaded it, what it was called, and which record field holds it.
--
-- `files` has one row for each distinct set of bytes. `file_refs` has one row for each upload. Two uploads of the
-- same bytes share a blob and have two refs. A ref with no `attached_at` is an upload no record holds yet. A job
-- removes those after 24 hours. `deleted_at` is set when a record lets go of the file.
CREATE TABLE IF NOT EXISTS files (
    sha256 CHAR(64) PRIMARY KEY,
    size BIGINT NOT NULL,
    media_type VARCHAR(100) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS file_refs (
    id UUID PRIMARY KEY,
    sha256 CHAR(64) NOT NULL REFERENCES files(sha256),
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64),
    record_id VARCHAR(64),
    field VARCHAR(100),
    name VARCHAR(255) NOT NULL,
    uploaded_by VARCHAR(255) NOT NULL,
    uploaded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    scan_state VARCHAR(20) NOT NULL DEFAULT 'skipped',
    attached_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_file_refs_record ON file_refs (app_slug, record_id) WHERE record_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_file_refs_sha ON file_refs (sha256);
CREATE INDEX IF NOT EXISTS idx_file_refs_unattached ON file_refs (uploaded_at) WHERE attached_at IS NULL;
