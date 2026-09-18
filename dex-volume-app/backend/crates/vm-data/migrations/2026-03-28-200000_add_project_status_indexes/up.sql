-- Composite index for orphan sweep query: get_running_projects()
-- WHERE status = 'running' AND locked = false
CREATE INDEX idx_projects_status_locked ON projects (status, locked);

-- Composite index for auto-lock sweep: get_projects_due_for_lock()
-- WHERE locked = false AND lock_at IS NOT NULL AND lock_at <= NOW()
CREATE INDEX idx_projects_locked_lock_at ON projects (locked, lock_at) WHERE lock_at IS NOT NULL;
