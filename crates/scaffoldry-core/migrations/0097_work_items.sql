-- Workflows phase 5: My Work. What each person is asked to do and what they wait on. A title and a summary hold names,
-- never record values. Items open and close in the transaction of the change that causes them.
CREATE TABLE IF NOT EXISTS work_items (
    id UUID PRIMARY KEY,
    person VARCHAR(255) NOT NULL,
    kind VARCHAR(10) NOT NULL,
    state VARCHAR(8) NOT NULL DEFAULT 'open',
    app_slug VARCHAR(64),
    workspace_id VARCHAR(64),
    flow_id VARCHAR(64),
    instance_id VARCHAR(128),
    step_id VARCHAR(64),
    record_id VARCHAR(64),
    proposal_id UUID,
    title VARCHAR(200) NOT NULL,
    summary VARCHAR(300) NOT NULL,
    via VARCHAR(30) NOT NULL DEFAULT 'holder',
    due_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    closed_at TIMESTAMPTZ,
    closed_reason VARCHAR(20)
);
CREATE INDEX IF NOT EXISTS idx_work_open ON work_items (person, state, due_at) WHERE state = 'open';
CREATE INDEX IF NOT EXISTS idx_work_instance ON work_items (instance_id);
-- The person a delegate acts for. The brief's delegate items need it, and its table has no column for it.
ALTER TABLE work_items ADD COLUMN IF NOT EXISTS acting_for VARCHAR(255);
-- A page of a person's open items: due date first, then age, with the id to break ties.
CREATE INDEX IF NOT EXISTS idx_work_page ON work_items (person, due_at, created_at, id) WHERE state = 'open';
CREATE INDEX IF NOT EXISTS idx_work_proposal ON work_items (proposal_id) WHERE proposal_id IS NOT NULL;

-- Real-time phase 3: an item opened or closed tells its person the new counts, and the item's id. No text.
CREATE OR REPLACE FUNCTION scaffoldry_me_work_changed() RETURNS trigger AS $$
BEGIN
    PERFORM pg_notify('scaffoldry_me', json_build_object(
        'to', NEW.person, 'type', 'work', 'id', NEW.id,
        'counts', (SELECT COALESCE(json_object_agg(kind, n), '{}'::json) FROM (
            SELECT kind, count(*) AS n FROM work_items WHERE person = NEW.person AND state = 'open' GROUP BY kind) k))::text);
    RETURN NULL;
END $$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS me_work_changed ON work_items;
CREATE TRIGGER me_work_changed AFTER INSERT OR UPDATE OF state ON work_items FOR EACH ROW EXECUTE FUNCTION scaffoldry_me_work_changed();

-- A proposal that leaves Pending closes its review and waiting items in the same transaction, whichever path decides it.
CREATE OR REPLACE FUNCTION scaffoldry_proposal_work_closed() RETURNS trigger AS $$
BEGIN
    IF OLD.status = 'Pending' AND NEW.status <> 'Pending' THEN
        UPDATE work_items SET state = 'closed', closed_at = NOW(), closed_reason = lower(NEW.status)
        WHERE proposal_id = NEW.id AND state = 'open';
    END IF;
    RETURN NULL;
END $$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS proposal_work_closed ON app_proposals;
CREATE TRIGGER proposal_work_closed AFTER UPDATE OF status ON app_proposals FOR EACH ROW EXECUTE FUNCTION scaffoldry_proposal_work_closed();
