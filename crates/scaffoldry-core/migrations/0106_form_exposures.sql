-- Guests phase 2: opening a form to guests is a proposal that a second person decides. A row is one such proposal.
-- `config_digest` is a hash of the form's config when it was proposed, so a form changed after that cannot be approved.
CREATE TABLE IF NOT EXISTS form_exposures (
    id UUID PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    form_id UUID NOT NULL,
    status VARCHAR(12) NOT NULL DEFAULT 'Pending',
    proposed_by VARCHAR(320) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    checks JSONB NOT NULL,
    change TEXT NOT NULL,
    config_digest CHAR(64) NOT NULL,
    decided_by VARCHAR(320),
    decided_at TIMESTAMPTZ,
    rationale TEXT
);
CREATE INDEX IF NOT EXISTS idx_form_exposures_form ON form_exposures (app_slug, form_id);
