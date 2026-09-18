CREATE TABLE subscriptions (
    id SERIAL PRIMARY KEY,
    project_id INTEGER NOT NULL UNIQUE REFERENCES projects(id) ON DELETE CASCADE,
    monthly_rate NUMERIC NOT NULL,
    currency TEXT NOT NULL DEFAULT 'USD',
    started_at TIMESTAMP NOT NULL DEFAULT NOW(),
    next_payment_due TIMESTAMP NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    notes TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_subscriptions_project_id ON subscriptions(project_id);
CREATE INDEX idx_subscriptions_status ON subscriptions(status);
CREATE INDEX idx_subscriptions_next_payment_due ON subscriptions(next_payment_due);

CREATE TABLE payments (
    id SERIAL PRIMARY KEY,
    subscription_id INTEGER NOT NULL REFERENCES subscriptions(id) ON DELETE CASCADE,
    amount NUMERIC NOT NULL,
    currency TEXT NOT NULL DEFAULT 'USD',
    paid_at TIMESTAMP NOT NULL,
    recorded_by INTEGER NOT NULL REFERENCES users(id),
    notes TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_payments_subscription_id ON payments(subscription_id);
