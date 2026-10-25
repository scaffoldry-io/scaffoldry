-- Policy library phase 1: policies, their sources, snapshots of each source, rules that cite a passage, questions and bindings.
-- Standard search: OSCAL 1.1.2 catalog and back-matter (adopted); W3C Web Annotation text quote (model for a cited passage).
CREATE TABLE IF NOT EXISTS library_policies (
    id UUID PRIMARY KEY,
    key VARCHAR(64) NOT NULL UNIQUE,
    title VARCHAR(255) NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    organization_id UUID NOT NULL REFERENCES organizations(id),
    owner JSONB NOT NULL,
    review_every_days INTEGER NOT NULL DEFAULT 365,
    last_reviewed_at TIMESTAMPTZ,
    status VARCHAR(10) NOT NULL DEFAULT 'Active' CHECK (status IN ('Active', 'Retired')),
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS library_sources (
    id UUID PRIMARY KEY,
    policy_id UUID NOT NULL REFERENCES library_policies(id),
    title VARCHAR(255) NOT NULL,
    kind VARCHAR(8) NOT NULL CHECK (kind IN ('url', 'text')),
    href TEXT,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS library_snapshots (
    id UUID PRIMARY KEY,
    source_id UUID NOT NULL REFERENCES library_sources(id),
    taken_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    media_type VARCHAR(128) NOT NULL,
    content_sha256 CHAR(64) NOT NULL,
    text TEXT NOT NULL,
    text_sha256 CHAR(64) NOT NULL,
    text_origin VARCHAR(10) NOT NULL CHECK (text_origin IN ('fetched', 'supplied')),
    taken_by VARCHAR(255) NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_library_snapshots_source ON library_snapshots (source_id, taken_at DESC);

CREATE TABLE IF NOT EXISTS library_rules (
    id UUID PRIMARY KEY,
    policy_id UUID NOT NULL REFERENCES library_policies(id),
    key VARCHAR(64) NOT NULL,
    version INTEGER NOT NULL,
    statement TEXT NOT NULL,
    applies_to TEXT NOT NULL DEFAULT '',
    status VARCHAR(16) NOT NULL CHECK (status IN ('Draft', 'InReview', 'Published', 'Rejected', 'Superseded', 'Retired')),
    source_changed BOOLEAN NOT NULL DEFAULT FALSE,
    submitted_by VARCHAR(255) NOT NULL,
    submitted_via VARCHAR(8) NOT NULL CHECK (submitted_via IN ('agent', 'person')),
    submitted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    decided_by VARCHAR(255),
    decided_at TIMESTAMPTZ,
    decision_note TEXT NOT NULL DEFAULT '',
    UNIQUE (policy_id, key, version)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_library_rule_one_published ON library_rules (policy_id, key) WHERE status = 'Published';

CREATE TABLE IF NOT EXISTS library_citations (
    id UUID PRIMARY KEY,
    rule_id UUID NOT NULL REFERENCES library_rules(id),
    snapshot_id UUID NOT NULL REFERENCES library_snapshots(id),
    passage TEXT NOT NULL,
    locator VARCHAR(255) NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_library_citations_rule ON library_citations (rule_id);

CREATE TABLE IF NOT EXISTS library_questions (
    id UUID PRIMARY KEY,
    policy_id UUID NOT NULL REFERENCES library_policies(id),
    question TEXT NOT NULL,
    context_passage TEXT NOT NULL DEFAULT '',
    snapshot_id UUID REFERENCES library_snapshots(id),
    asked_by VARCHAR(255) NOT NULL,
    asked_via VARCHAR(8) NOT NULL CHECK (asked_via IN ('agent', 'person')),
    asked_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    answer TEXT,
    answered_by VARCHAR(255),
    answered_at TIMESTAMPTZ
);

-- A binding belongs to a rule key, not to a version, so it survives a reworded rule.
CREATE TABLE IF NOT EXISTS library_bindings (
    id UUID PRIMARY KEY,
    rule_key_policy_id UUID NOT NULL REFERENCES library_policies(id),
    rule_key VARCHAR(64) NOT NULL,
    target_kind VARCHAR(24) NOT NULL CHECK (target_kind IN ('cedar_policy', 'guard_template', 'sensitivity_category', 'retention_policy', 'authority_row')),
    target_ref VARCHAR(255) NOT NULL,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (rule_key_policy_id, rule_key, target_kind, target_ref)
);
