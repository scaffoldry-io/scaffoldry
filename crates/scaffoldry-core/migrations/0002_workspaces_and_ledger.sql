-- Scaffoldry Workspaces, Collaborators, Sessions, and Cryptographic Ledger Schema (PostgreSQL 17)
-- Aligned with NIST OSCAL 1.1.2 AU-02, AC-02, and AC-03

CREATE TABLE IF NOT EXISTS workspaces (
    id VARCHAR(64) PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    code VARCHAR(64) NOT NULL,
    organization VARCHAR(255) NOT NULL,
    department VARCHAR(255) NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    icon VARCHAR(64) NOT NULL DEFAULT '📁',
    lead VARCHAR(255) NOT NULL,
    visibility VARCHAR(32) NOT NULL DEFAULT 'restricted',
    allowed_affiliations JSONB NOT NULL DEFAULT '[]',
    data_classification VARCHAR(64) NOT NULL DEFAULT 'Internal',
    cedar_policy_guard TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_workspaces_department ON workspaces(department);
CREATE INDEX IF NOT EXISTS idx_workspaces_visibility ON workspaces(visibility);

CREATE TABLE IF NOT EXISTS workspace_collaborators (
    id VARCHAR(64) PRIMARY KEY,
    workspace_id VARCHAR(64) NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    eppn VARCHAR(255) NOT NULL,
    name VARCHAR(255) NOT NULL,
    role VARCHAR(32) NOT NULL,
    scoped_affiliation VARCHAR(64) NOT NULL,
    department VARCHAR(255) NOT NULL,
    added_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_workspace_collab UNIQUE (workspace_id, eppn)
);

CREATE INDEX IF NOT EXISTS idx_collab_workspace ON workspace_collaborators(workspace_id);
CREATE INDEX IF NOT EXISTS idx_collab_eppn ON workspace_collaborators(eppn);

CREATE TABLE IF NOT EXISTS dataset_records (
    id VARCHAR(64) PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    data JSONB NOT NULL DEFAULT '{}',
    ceds_mapping JSONB NOT NULL DEFAULT '{}',
    is_ferpa_sensitive BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_dataset_records_app_slug ON dataset_records(app_slug);
CREATE INDEX IF NOT EXISTS idx_dataset_records_data_gin ON dataset_records USING gin(data);

CREATE TABLE IF NOT EXISTS auth_sessions (
    token VARCHAR(255) PRIMARY KEY,
    user_eppn VARCHAR(255) NOT NULL,
    user_name VARCHAR(255) NOT NULL,
    user_role_title VARCHAR(255) NOT NULL,
    user_affiliation VARCHAR(64) NOT NULL,
    user_department VARCHAR(255) NOT NULL,
    original_admin JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS governance_ledger (
    sequence BIGINT PRIMARY KEY,
    entry_hash VARCHAR(64) NOT NULL UNIQUE,
    previous_hash VARCHAR(64) NOT NULL,
    timestamp_iso VARCHAR(64) NOT NULL,
    principal VARCHAR(255) NOT NULL,
    organization_code VARCHAR(64) NOT NULL,
    app_slug VARCHAR(64),
    decision_type VARCHAR(64) NOT NULL,
    oscal_control_id VARCHAR(32) NOT NULL,
    rationale TEXT NOT NULL,
    payload_hash VARCHAR(64) NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_ledger_entry_hash ON governance_ledger(entry_hash);
CREATE INDEX IF NOT EXISTS idx_ledger_principal ON governance_ledger(principal);
ALTER TABLE governance_ledger ADD COLUMN IF NOT EXISTS payload_hash VARCHAR(64);
