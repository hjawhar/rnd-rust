-- Paginated transaction listing: WHERE project_id = ? ORDER BY slot DESC OFFSET/LIMIT
CREATE INDEX idx_transactions_project_slot ON transactions (project_id, slot DESC);

-- Buy/sell statistics: WHERE project_id = ? AND tx_type = ?
CREATE INDEX idx_transactions_project_type ON transactions (project_id, tx_type);

-- Interval-based statistics: WHERE project_id = ? AND date_added >= NOW() - INTERVAL
CREATE INDEX idx_transactions_project_date ON transactions (project_id, date_added);
