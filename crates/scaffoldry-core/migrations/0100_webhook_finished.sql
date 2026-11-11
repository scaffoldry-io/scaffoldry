-- Integrations phase 5: when a delivery reached its end, delivered or dead. The Overview counts dead deliveries of
-- the last day from this, not from when the delivery was queued, because a delivery can retry for two days first.
ALTER TABLE webhook_deliveries ADD COLUMN IF NOT EXISTS finished_at TIMESTAMPTZ;
UPDATE webhook_deliveries SET finished_at = COALESCE(delivered_at, created_at) WHERE finished_at IS NULL AND state IN ('delivered', 'dead');
