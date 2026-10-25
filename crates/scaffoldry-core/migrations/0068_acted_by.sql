-- Who held the keyboard. Under impersonation `actor` is the person acted as and `acted_by` is the administrator.
ALTER TABLE record_history ADD COLUMN IF NOT EXISTS acted_by TEXT;
