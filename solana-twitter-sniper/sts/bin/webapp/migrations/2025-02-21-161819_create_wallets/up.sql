-- Your SQL goes here
CREATE TABLE wallets (
    id SERIAL,
    user_id int NOT NULL,
    address VARCHAR(100) NOT NULL,
    name TEXT NOT NULL,
    comments TEXT,
    pk TEXT NOT NULL,
    PRIMARY KEY (id),
    CONSTRAINT fk_user FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
)