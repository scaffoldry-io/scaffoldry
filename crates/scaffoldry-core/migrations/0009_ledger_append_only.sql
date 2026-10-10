CREATE OR REPLACE FUNCTION governance_ledger_immutable() RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'governance_ledger is append-only';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS governance_ledger_no_change ON governance_ledger;
CREATE TRIGGER governance_ledger_no_change
    BEFORE UPDATE OR DELETE OR TRUNCATE ON governance_ledger
    FOR EACH STATEMENT EXECUTE FUNCTION governance_ledger_immutable();
