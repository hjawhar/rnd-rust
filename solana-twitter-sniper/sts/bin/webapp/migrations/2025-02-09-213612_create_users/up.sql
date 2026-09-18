-- Your SQL goes here
CREATE TABLE users (
    id SERIAL,
    group_id int NOT NULL,
    address VARCHAR(42) UNIQUE NOT NULL,
    nonce TEXT NOT NULL,
    whitelisted bool NOT NULL,
    last_login_date_time TIMESTAMP,
    PRIMARY KEY (id)
)