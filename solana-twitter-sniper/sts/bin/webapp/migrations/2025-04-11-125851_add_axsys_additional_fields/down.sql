-- This file should undo anything in `up.sql`
ALTER TABLE tasks DROP COLUMN twitter_api;
ALTER TABLE tasks DROP COLUMN twitter_strategy;
ALTER TABLE tasks DROP COLUMN twitter_handle_checker;
ALTER TABLE tasks DROP COLUMN twitter_token_override;
ALTER TABLE tasks DROP COLUMN words;
