-- Policy library phase 6: the health screen asks of every policy whether it has a published rule, and of every rule
-- whether it is bound. These indexes keep each of its four statements to one pass.
CREATE INDEX IF NOT EXISTS idx_library_rules_policy_status ON library_rules (policy_id, status);
CREATE INDEX IF NOT EXISTS idx_library_bindings_rule ON library_bindings (rule_key_policy_id, rule_key);
CREATE INDEX IF NOT EXISTS idx_library_questions_open ON library_questions (policy_id, asked_at) WHERE answer IS NULL;
CREATE INDEX IF NOT EXISTS idx_library_citations_snapshot ON library_citations (snapshot_id);
-- The rules marked source_changed are few. This finds them without reading the rest.
CREATE INDEX IF NOT EXISTS idx_library_rules_source_changed ON library_rules (id) WHERE source_changed AND status = 'Published';
