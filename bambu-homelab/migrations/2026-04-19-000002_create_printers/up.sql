CREATE TABLE printers (
    id VARCHAR(255) PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    ip VARCHAR(45) NOT NULL,
    serial VARCHAR(255) NOT NULL UNIQUE,
    access_code VARCHAR(255) NOT NULL,
    model VARCHAR(50) NOT NULL DEFAULT 'X1C',
    owner_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_printers_owner ON printers(owner_id);
