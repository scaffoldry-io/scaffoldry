-- Older servers created a provisioning credential on every boot. Its secret was a string written in the source, so anyone
-- who had read the source could use it. This revokes it. The row is kept as a record. The hash below is the SHA-256 of that
-- public string, so it is not a secret.
UPDATE api_tokens
   SET revoked_at = COALESCE(revoked_at, NOW())
 WHERE kind = 'scim'
   AND token_hash = '96d72274517e0d926344cee50c7da354c52ccf06fb22fc45a34958db62a84d4f';
