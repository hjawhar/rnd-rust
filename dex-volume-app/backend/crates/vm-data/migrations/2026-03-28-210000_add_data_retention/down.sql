DROP FUNCTION IF EXISTS delete_old_audit_logs(INTERVAL);
DROP FUNCTION IF EXISTS delete_old_transactions(INTERVAL);
DROP INDEX IF EXISTS idx_transactions_date_added;
