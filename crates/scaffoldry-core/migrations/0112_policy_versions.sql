-- Admin console phase 5: the running policy is a stored, versioned, testable thing.
-- A version is a Draft until a person other than its author activates it. At most one version is Active.
CREATE TABLE IF NOT EXISTS policy_versions (
    id UUID PRIMARY KEY,
    version INTEGER NOT NULL UNIQUE,
    source TEXT NOT NULL,
    status VARCHAR(8) NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    activated_by VARCHAR(255),
    activated_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_policy_one_active ON policy_versions ((status)) WHERE status = 'Active';

-- A test case: a question in Cedar entity JSON and the answer it must get.
CREATE TABLE IF NOT EXISTS policy_cases (
    id UUID PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    principal JSONB NOT NULL,
    action VARCHAR(64) NOT NULL,
    resource JSONB NOT NULL,
    expected VARCHAR(5) NOT NULL,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- OSCAL documents the institution edits, one per kind. `component-definition` maps controls to statements and status.
CREATE TABLE IF NOT EXISTS governance_documents (
    kind VARCHAR(32) PRIMARY KEY,
    document JSONB NOT NULL,
    updated_by VARCHAR(255) NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
