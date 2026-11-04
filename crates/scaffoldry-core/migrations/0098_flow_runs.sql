-- Workflows phase 10: a flow's runs, newest first, listed by start without reading every instance.
CREATE INDEX IF NOT EXISTS idx_process_instances_rule_started ON process_instances (rule_id, (instance_json->>'started_at') DESC, id DESC);
