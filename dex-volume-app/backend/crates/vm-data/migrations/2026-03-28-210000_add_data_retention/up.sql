-- Index to support efficient date-range deletes on transactions.
-- The (date_added) index enables: DELETE FROM transactions WHERE date_added < NOW() - INTERVAL '90 days'
-- Note: transactions already has idx_transactions_project_date on (project_id, date_added)
-- but that composite index isn't useful for a full-table date prune.
CREATE INDEX idx_transactions_date_added ON transactions (date_added);

-- Retention helper: delete transactions older than the given interval.
-- Usage: SELECT delete_old_transactions('90 days');
-- Returns: number of rows deleted.
CREATE OR REPLACE FUNCTION delete_old_transactions(retention_interval INTERVAL)
RETURNS BIGINT AS $$
DECLARE
    deleted BIGINT;
BEGIN
    DELETE FROM transactions WHERE date_added < NOW() - retention_interval;
    GET DIAGNOSTICS deleted = ROW_COUNT;
    RETURN deleted;
END;
$$ LANGUAGE plpgsql;

-- Retention helper: delete audit logs older than the given interval.
-- Usage: SELECT delete_old_audit_logs('180 days');
-- Returns: number of rows deleted.
CREATE OR REPLACE FUNCTION delete_old_audit_logs(retention_interval INTERVAL)
RETURNS BIGINT AS $$
DECLARE
    deleted BIGINT;
BEGIN
    DELETE FROM audit_logs WHERE created_at < NOW() - retention_interval;
    GET DIAGNOSTICS deleted = ROW_COUNT;
    RETURN deleted;
END;
$$ LANGUAGE plpgsql;
