CREATE TABLE IF NOT EXISTS accounts (
    name TEXT PRIMARY KEY NOT NULL,
    email TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    verified INTEGER NOT NULL DEFAULT 0,
    verify_token TEXT,
    cert_fingerprint TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
