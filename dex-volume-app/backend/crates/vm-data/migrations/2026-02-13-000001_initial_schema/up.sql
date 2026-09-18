CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    group_id INT NOT NULL,
    address TEXT UNIQUE NOT NULL,
    nonce TEXT NOT NULL,
    whitelisted BOOLEAN NOT NULL,
    date_added TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE projects (
    id SERIAL PRIMARY KEY,
    user_id INT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    address TEXT NOT NULL,
    pool TEXT NOT NULL,
    pool_type TEXT NOT NULL,
    trading_strategy TEXT NOT NULL DEFAULT 'VOLUME_MAKER',
    trading_interval DECIMAL NOT NULL DEFAULT 1,
    trading_daily_volume DECIMAL NOT NULL DEFAULT 1000000,
    name TEXT,
    symbol TEXT,
    description TEXT,
    image TEXT,
    date_added TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    fees DECIMAL NOT NULL DEFAULT 0,
    network TEXT NOT NULL DEFAULT 'solana',
    decimals INTEGER,
    pair TEXT,
    max_market_impact_bps INTEGER,
    trade_multiplier DOUBLE PRECISION
);

CREATE TABLE wallets (
    id SERIAL PRIMARY KEY,
    project_id INT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    address TEXT NOT NULL,
    pk TEXT NOT NULL,
    is_main BOOLEAN NOT NULL DEFAULT false,
    date_added TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE transactions (
    id SERIAL PRIMARY KEY,
    project_id INT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    slot BIGINT NOT NULL,
    sol_price DECIMAL NOT NULL,
    value DECIMAL NOT NULL,
    tokens DECIMAL NOT NULL,
    address TEXT NOT NULL,
    tx_hash TEXT NOT NULL,
    tx_type TEXT NOT NULL,
    token_in TEXT NOT NULL,
    token_out TEXT NOT NULL,
    date_added TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE uniswap_v4_pools (
    id SERIAL PRIMARY KEY,
    network_id INT NOT NULL,
    token_id BIGINT NOT NULL,
    pool_key TEXT NOT NULL,
    currency0 TEXT NOT NULL,
    currency1 TEXT NOT NULL,
    tick_spacing TEXT NOT NULL,
    fee TEXT NOT NULL,
    hooks TEXT NOT NULL,
    UNIQUE (network_id, token_id)
);
