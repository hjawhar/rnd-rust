CREATE TABLE audit_logs (
    id SERIAL PRIMARY KEY,
    user_id INT NOT NULL REFERENCES users(id),
    project_id INT,
    action TEXT NOT NULL,
    details JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_audit_logs_user_id_created_at ON audit_logs(user_id, created_at DESC);
CREATE INDEX idx_audit_logs_project_id_created_at ON audit_logs(project_id, created_at DESC);
