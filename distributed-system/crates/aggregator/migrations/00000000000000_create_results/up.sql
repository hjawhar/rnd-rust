CREATE EXTENSION IF NOT EXISTS "pgcrypto";

CREATE TABLE results (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    request_id UUID NOT NULL,
    task_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    enrichment JSONB,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    processor_id TEXT NOT NULL
);

CREATE INDEX idx_results_task_type ON results(task_type);
CREATE INDEX idx_results_processed_at ON results(processed_at);
