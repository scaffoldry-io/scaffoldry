-- Scaffoldry Core Relational Schema (PostgreSQL 17)
-- Aligned with NCES CEDS v11.0, REFEDS eduPerson, and EDUCAUSE HERM v3.1

CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- 1. ORGANIZATIONS (CEDS PostsecondaryInstitution & AcademicSubdivision)
CREATE TABLE IF NOT EXISTS organizations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    parent_id UUID REFERENCES organizations(id) ON DELETE RESTRICT,
    name VARCHAR(255) NOT NULL,
    code VARCHAR(64) NOT NULL UNIQUE,
    org_type VARCHAR(64) NOT NULL,
    ipeds_unit_id VARCHAR(32),
    opeid VARCHAR(32),
    herm_domain VARCHAR(64),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 2. PERSONS (CEDS Person & PersonIdentity)
CREATE TABLE IF NOT EXISTS persons (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    first_name VARCHAR(128) NOT NULL,
    last_name VARCHAR(128) NOT NULL,
    email VARCHAR(255) NOT NULL UNIQUE,
    eppn VARCHAR(255) UNIQUE,
    directory_id VARCHAR(64) UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 3. ROLES & APPOINTMENTS (CEDS Staff/Faculty & eduPersonScopedAffiliation)
CREATE TABLE IF NOT EXISTS roles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    person_id UUID NOT NULL REFERENCES persons(id) ON DELETE CASCADE,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    role_title VARCHAR(128) NOT NULL,
    scoped_affiliation VARCHAR(64) NOT NULL,
    is_primary BOOLEAN NOT NULL DEFAULT TRUE,
    effective_date DATE NOT NULL DEFAULT CURRENT_DATE,
    end_date DATE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 4. ACADEMIC PLANS (CEDS AcademicProgram / Course / Plan)
CREATE TABLE IF NOT EXISTS academic_plans (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    program_name VARCHAR(255) NOT NULL,
    cip_code VARCHAR(16) NOT NULL,
    degree_level VARCHAR(64) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 5. FACILITIES (CEDS Facility & SpaceUtilization / FICM)
CREATE TABLE IF NOT EXISTS facilities (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    building_name VARCHAR(255) NOT NULL,
    room_number VARCHAR(32) NOT NULL,
    ficm_code VARCHAR(16) NOT NULL,
    capacity INTEGER NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 6. APPLICATIONS (Departmental Tools & DNS Aliasing)
CREATE TABLE IF NOT EXISTS apps (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    slug VARCHAR(64) NOT NULL,
    title VARCHAR(255) NOT NULL,
    description TEXT,
    herm_capability_id VARCHAR(32),
    custom_domain VARCHAR(255) UNIQUE,
    custom_domain_verified BOOLEAN NOT NULL DEFAULT FALSE,
    is_published BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_org_app_slug UNIQUE (organization_id, slug)
);

CREATE INDEX IF NOT EXISTS idx_apps_custom_domain ON apps(custom_domain) WHERE custom_domain IS NOT NULL;

-- 7. RECORDS (Dynamic Metadata Envelope with CEDS Mapping)
CREATE TABLE IF NOT EXISTS records (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    created_by UUID REFERENCES persons(id) ON DELETE SET NULL,
    data JSONB NOT NULL DEFAULT '{}',
    ceds_mapping JSONB NOT NULL DEFAULT '{}',
    is_ferpa_sensitive BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_records_app_id ON records(app_id);
CREATE INDEX IF NOT EXISTS idx_records_data_gin ON records USING gin(data);
