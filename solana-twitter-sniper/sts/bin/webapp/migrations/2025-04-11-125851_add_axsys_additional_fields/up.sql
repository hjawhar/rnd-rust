-- Your SQL goes here
ALTER TABLE tasks ADD twitter_api TEXT default 'EXTREME' NOT NULL;
ALTER TABLE tasks ADD twitter_strategy TEXT default 'TWEET';
ALTER TABLE tasks ADD twitter_handle_checker TEXT;
ALTER TABLE tasks ADD twitter_token_override TEXT;
ALTER TABLE tasks ADD words TEXT;
