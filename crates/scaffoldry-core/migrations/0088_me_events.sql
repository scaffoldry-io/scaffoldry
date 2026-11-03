-- Real-time phase 3: the person's own events, sent by the tables themselves so every writer is covered.
-- Each sends pg_notify('scaffoldry_me', { to, type, ... }), delivered only if the write commits. The payload holds ids,
-- counts and states: never a title, a body, a name or a value.

-- A new notice: its id and the recipient's unread count.
CREATE OR REPLACE FUNCTION scaffoldry_me_notification_added() RETURNS trigger AS $$
BEGIN
    PERFORM pg_notify('scaffoldry_me', json_build_object(
        'to', lower(NEW.recipient), 'type', 'notification', 'id', NEW.id,
        'count', (SELECT count(*) FROM notifications WHERE lower(recipient) = lower(NEW.recipient) AND read_at IS NULL))::text);
    RETURN NULL;
END $$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS me_notification_added ON notifications;
CREATE TRIGGER me_notification_added AFTER INSERT ON notifications FOR EACH ROW EXECUTE FUNCTION scaffoldry_me_notification_added();

-- Notices marked read: one event per person with the new count, however many were marked.
CREATE OR REPLACE FUNCTION scaffoldry_me_notifications_read() RETURNS trigger AS $$
DECLARE r RECORD;
BEGIN
    FOR r IN SELECT DISTINCT lower(n.recipient) AS who FROM new_rows n JOIN old_rows o ON o.id = n.id WHERE o.read_at IS DISTINCT FROM n.read_at LOOP
        PERFORM pg_notify('scaffoldry_me', json_build_object(
            'to', r.who, 'type', 'notification', 'id', NULL,
            'count', (SELECT count(*) FROM notifications WHERE lower(recipient) = r.who AND read_at IS NULL))::text);
    END LOOP;
    RETURN NULL;
END $$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS me_notifications_read ON notifications;
CREATE TRIGGER me_notifications_read AFTER UPDATE ON notifications REFERENCING OLD TABLE AS old_rows NEW TABLE AS new_rows
    FOR EACH STATEMENT EXECUTE FUNCTION scaffoldry_me_notifications_read();

-- A job's state, or its whole percent, changed. A heartbeat changes neither and sends nothing.
CREATE OR REPLACE FUNCTION scaffoldry_me_job() RETURNS trigger AS $$
BEGIN
    PERFORM pg_notify('scaffoldry_me', json_build_object(
        'to', lower(NEW.created_by), 'type', 'job', 'id', NEW.id, 'state', NEW.state,
        'percent', CASE WHEN NEW.progress_total > 0 THEN LEAST(100, NEW.progress_done * 100 / NEW.progress_total) END)::text);
    RETURN NULL;
END $$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS me_job_added ON jobs;
CREATE TRIGGER me_job_added AFTER INSERT ON jobs FOR EACH ROW EXECUTE FUNCTION scaffoldry_me_job();
DROP TRIGGER IF EXISTS me_job_changed ON jobs;
CREATE TRIGGER me_job_changed AFTER UPDATE OF state, progress_done, progress_total ON jobs FOR EACH ROW
    WHEN (OLD.state IS DISTINCT FROM NEW.state
          OR (CASE WHEN OLD.progress_total > 0 THEN OLD.progress_done * 100 / OLD.progress_total END)
             IS DISTINCT FROM (CASE WHEN NEW.progress_total > 0 THEN NEW.progress_done * 100 / NEW.progress_total END))
    EXECUTE FUNCTION scaffoldry_me_job();

-- A proposal was made or decided: its id and status, for the person who proposed it.
CREATE OR REPLACE FUNCTION scaffoldry_me_proposal() RETURNS trigger AS $$
BEGIN
    PERFORM pg_notify('scaffoldry_me', json_build_object('to', lower(NEW.proposed_by), 'type', 'proposal', 'id', NEW.id, 'status', NEW.status)::text);
    RETURN NULL;
END $$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS me_proposal_added ON app_proposals;
CREATE TRIGGER me_proposal_added AFTER INSERT ON app_proposals FOR EACH ROW EXECUTE FUNCTION scaffoldry_me_proposal();
DROP TRIGGER IF EXISTS me_proposal_changed ON app_proposals;
CREATE TRIGGER me_proposal_changed AFTER UPDATE OF status ON app_proposals FOR EACH ROW
    WHEN (OLD.status IS DISTINCT FROM NEW.status) EXECUTE FUNCTION scaffoldry_me_proposal();
