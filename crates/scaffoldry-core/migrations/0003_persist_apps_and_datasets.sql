-- Scaffoldry App Manifests, Published Datasets, Relationships, Workflow Automations, and SCIM Identity (PostgreSQL 17)
-- Aligned with NIST OSCAL 1.1.2 CM-03, SC-07, and AC-03

CREATE TABLE IF NOT EXISTS app_manifests (
    slug VARCHAR(64) PRIMARY KEY,
    title VARCHAR(255) NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    organization_code VARCHAR(64) NOT NULL,
    department VARCHAR(255) NOT NULL,
    herm_capability_id VARCHAR(64),
    custom_domain VARCHAR(255) UNIQUE,
    custom_domain_verified BOOLEAN NOT NULL DEFAULT FALSE,
    manifest JSONB NOT NULL DEFAULT '{}',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_app_manifests_department ON app_manifests(department);
CREATE INDEX IF NOT EXISTS idx_app_manifests_custom_domain ON app_manifests(custom_domain) WHERE custom_domain IS NOT NULL;

CREATE TABLE IF NOT EXISTS published_datasets (
    id VARCHAR(64) PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    department VARCHAR(255) NOT NULL,
    organization VARCHAR(255) NOT NULL,
    sensitivity_level VARCHAR(64) NOT NULL DEFAULT 'Directory',
    herm_capability_id VARCHAR(64),
    payload JSONB NOT NULL DEFAULT '{}',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_published_datasets_department ON published_datasets(department);

CREATE TABLE IF NOT EXISTS dataset_relationships (
    id VARCHAR(64) PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    source_dataset_id VARCHAR(64) NOT NULL,
    target_dataset_id VARCHAR(64) NOT NULL,
    source_field VARCHAR(64) NOT NULL,
    target_field VARCHAR(64) NOT NULL,
    relationship_type VARCHAR(32) NOT NULL DEFAULT 'OneToMany',
    display_field VARCHAR(64) NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_dataset_rels_source ON dataset_relationships(source_dataset_id);
CREATE INDEX IF NOT EXISTS idx_dataset_rels_target ON dataset_relationships(target_dataset_id);

CREATE TABLE IF NOT EXISTS workflow_automations (
    id VARCHAR(64) PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    name VARCHAR(255) NOT NULL,
    rule_json JSONB NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_workflow_automations_app ON workflow_automations(app_slug);

CREATE TABLE IF NOT EXISTS scim_users (
    id VARCHAR(64) PRIMARY KEY,
    user_name VARCHAR(255) NOT NULL,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    payload JSONB NOT NULL DEFAULT '{}',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS scim_groups (
    id VARCHAR(64) PRIMARY KEY,
    display_name VARCHAR(255) NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
