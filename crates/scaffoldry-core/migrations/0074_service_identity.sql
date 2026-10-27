-- External apps phase 5: a confidential client may have its own service identity, client:{client_id}. Its rights are
-- grants with source 'client'.
ALTER TABLE app_clients ADD COLUMN IF NOT EXISTS service_identity BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE access_grants DROP CONSTRAINT IF EXISTS access_grants_source_check;
ALTER TABLE access_grants ADD CONSTRAINT access_grants_source_check CHECK (source IN ('membership', 'appointment', 'rule', 'request', 'task', 'flow', 'client'));
