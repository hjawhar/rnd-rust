-- Your SQL goes here
CREATE TABLE tasks (
    id SERIAL,
    user_id int NOT NULL,
    wallet_id int,
    twitter_id TEXT,
    twitter_handle TEXT,
    servers TEXT,
    block_leaders TEXT,
    value decimal,
    tip decimal,
    slippage int default 50 NOT NULL,
    tries int default 1 NOT NULL,
    frontrunning_protection boolean default true NOT NULL,
    enable_alerts boolean default true NOT NULL,
    selected_pool TEXT default 'EXCLUDE_PUMPFUN' NOT NULL,
    PRIMARY KEY (id),
    CONSTRAINT fk_user FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
    CONSTRAINT fk_wallet FOREIGN KEY(wallet_id) REFERENCES wallets(id) ON DELETE CASCADE
)