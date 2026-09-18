-- Lookup indexes for frequently-queried columns

-- projects: filtered by user_id in get_projects(), get_project()
CREATE INDEX IF NOT EXISTS idx_projects_user_id ON projects (user_id);

-- projects: filtered by network in get_all_projects_by_network(), reset_project_statuses()
CREATE INDEX IF NOT EXISTS idx_projects_network ON projects (network);

-- wallets: filtered by project_id in get_project_wallets(), delete_project_wallets()
CREATE INDEX IF NOT EXISTS idx_wallets_project_id ON wallets (project_id);

-- wallets: filtered by address in get_project_wallet_by_addresses()
CREATE INDEX IF NOT EXISTS idx_wallets_address ON wallets (address);

-- users: filtered by address in get_user_by_address(), auth lookups
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_address ON users (address);
