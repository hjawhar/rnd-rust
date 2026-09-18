CREATE TABLE project_access (
    id SERIAL PRIMARY KEY,
    project_id INT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    user_id INT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(project_id, user_id)
);
CREATE INDEX idx_project_access_user_id ON project_access(user_id);
CREATE INDEX idx_project_access_project_id ON project_access(project_id);
