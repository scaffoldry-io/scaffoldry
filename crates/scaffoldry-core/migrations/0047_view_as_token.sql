-- A view-as token is scoped to one app. Other tokens leave this empty.
ALTER TABLE api_tokens ADD COLUMN IF NOT EXISTS app_slug VARCHAR(255);
