-- Every change to a record, written in the same transaction as the change. Append-only.
CREATE TABLE IF NOT EXISTS record_history (
    id BIGSERIAL PRIMARY KEY,
    app_slug VARCHAR(64) NOT NULL,
    table_id VARCHAR(64) NOT NULL,
    record_id VARCHAR(64) NOT NULL,
    version INTEGER NOT NULL,
    action VARCHAR(10) NOT NULL,
    changes JSONB NOT NULL,
    actor VARCHAR(255) NOT NULL,
    actor_kind VARCHAR(12) NOT NULL,
    token_label VARCHAR(255),
    batch_id UUID NOT NULL,
    reverts_batch UUID,
    proposal_id UUID,
    at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_history_record ON record_history (record_id, id DESC);
CREATE INDEX IF NOT EXISTS idx_history_app ON record_history (app_slug, id DESC);
CREATE INDEX IF NOT EXISTS idx_history_batch ON record_history (batch_id);
CREATE INDEX IF NOT EXISTS idx_history_actor ON record_history (actor, id DESC);

CREATE OR REPLACE FUNCTION record_history_immutable() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' AND current_setting('scaffoldry.purge', true) = 'on' THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'record_history is append-only';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS record_history_no_change ON record_history;
CREATE TRIGGER record_history_no_change
    BEFORE UPDATE OR DELETE ON record_history
    FOR EACH ROW EXECUTE FUNCTION record_history_immutable();
