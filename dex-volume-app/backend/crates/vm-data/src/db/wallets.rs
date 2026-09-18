use super::*;
use crate::db::schema::{project_access, projects, wallets};
use crate::models::project::Project;
use crate::models::wallet::{NewWallet, Wallet};

impl Database {
    pub async fn add_project_wallet(&self, new_wallet: &NewWallet) -> DbResult<Wallet> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;

        let wallet = diesel::insert_into(wallets::table)
            .values(new_wallet)
            .returning(Wallet::as_returning())
            .get_result(connection)
            .await?;
        Ok(wallet)
    }

    /// Atomically insert multiple wallets in a single DB transaction.
    /// If any insert fails, all are rolled back — no partial wallet sets.
    pub async fn add_wallets_batch(&self, new_wallets: &[NewWallet]) -> DbResult<Vec<Wallet>> {
        if new_wallets.is_empty() {
            return Ok(vec![]);
        }
        use diesel_async::AsyncConnection;
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let new_wallets = new_wallets.to_vec();
        connection
            .transaction::<Vec<Wallet>, diesel::result::Error, _>(|conn| {
                Box::pin(async move {
                    let wallets: Vec<Wallet> = diesel::insert_into(wallets::table)
                        .values(&new_wallets)
                        .returning(Wallet::as_returning())
                        .get_results(conn)
                        .await?;
                    Ok(wallets)
                })
            })
            .await
            .map_err(|e: diesel::result::Error| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }

    pub async fn get_project_wallet(
        &self,
        project_id: &i32,
        wallet_id: &i32,
    ) -> DbResult<Option<(Wallet, Project)>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let result = wallets::table
            .inner_join(projects::table)
            .filter(
                wallets::id
                    .eq(wallet_id)
                    .and(wallets::project_id.eq(project_id))
                    .and(wallets::project_id.eq(projects::id)),
            )
            .select((Wallet::as_returning(), Project::as_returning()))
            .first(connection)
            .await
            .optional()?;
        Ok(result)
    }

    pub async fn get_project_wallet_by_addresses(
        &self,
        user_id: i32,
        project_id: i32,
        addresses: &Vec<String>,
    ) -> DbResult<Vec<Wallet>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let shared_project_ids = project_access::table
            .filter(project_access::user_id.eq(user_id))
            .select(project_access::project_id);

        let wallets_data: Vec<Wallet> = wallets::table
            .inner_join(projects::table)
            .filter(
                wallets::project_id
                    .eq(project_id)
                    .and(
                        projects::user_id.eq(user_id)
                            .or(projects::id.eq_any(shared_project_ids))
                    )
                    .and(wallets::address.eq_any(addresses)),
            )
            .select(Wallet::as_returning())
            .load(connection)
            .await?;
        Ok(wallets_data)
    }

    pub async fn get_project_wallets(&self, user_id: i32, project_id: i32) -> DbResult<Vec<Wallet>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let shared_project_ids = project_access::table
            .filter(project_access::user_id.eq(user_id))
            .select(project_access::project_id);

        let wallets = wallets::table
            .inner_join(projects::table)
            .filter(
                wallets::project_id
                    .eq(project_id)
                    .and(
                        projects::user_id.eq(user_id)
                            .or(projects::id.eq_any(shared_project_ids))
                    ),
            )
            .select(Wallet::as_returning())
            .load(connection)
            .await?;
        Ok(wallets)
    }

    pub async fn get_project_wallets_by_ids_full(
        &self,
        user_id: i32,
        project_id: i32,
        wallet_ids: &Vec<i32>,
    ) -> DbResult<Vec<Wallet>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let shared_project_ids = project_access::table
            .filter(project_access::user_id.eq(user_id))
            .select(project_access::project_id);

        let wallets = wallets::table
            .inner_join(projects::table)
            .filter(
                wallets::project_id
                    .eq(project_id)
                    .and(
                        projects::user_id.eq(user_id)
                            .or(projects::id.eq_any(shared_project_ids))
                    )
                    .and(wallets::id.eq_any(wallet_ids)),
            )
            .select(Wallet::as_returning())
            .load(connection)
            .await?;
        Ok(wallets)
    }

    pub async fn delete_project_wallets(
        &self,
        user_id: i32,
        project_id: i32,
        wallet_ids: &Vec<i32>,
    ) -> DbResult<(bool, Vec<Wallet>)> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let shared_project_ids = project_access::table
            .filter(project_access::user_id.eq(user_id))
            .select(project_access::project_id);

        let wallets_data: Vec<Wallet> = wallets::table
            .inner_join(projects::table)
            .filter(
                wallets::project_id
                    .eq(project_id)
                    .and(
                        projects::user_id.eq(user_id)
                            .or(projects::id.eq_any(shared_project_ids))
                    )
                    .and(wallets::id.eq_any(wallet_ids)),
            )
            .select(Wallet::as_returning())
            .load(connection)
            .await?;

        let mapped_ids: Vec<_> = wallets_data.iter().map(|x| x.id).collect();
        let deleted = diesel::delete(wallets::table.filter(wallets::id.eq_any(mapped_ids)))
            .execute(connection)
            .await?;
        Ok((deleted > 0, wallets_data))
    }

    pub async fn delete_wallet(&self, project_id: i32, wallet_id: i32) -> DbResult<bool> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;

        let deleted = diesel::delete(
            wallets::table.filter(
                wallets::project_id
                    .eq(project_id)
                    .and(wallets::id.eq(wallet_id)),
            ),
        )
        .execute(connection)
        .await?;
        Ok(deleted > 0)
    }

    pub async fn update_wallet(
        &self,
        project_id: i32,
        wallet_id: i32,
        pk: String,
        address: String,
    ) -> DbResult<bool> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let updated: usize = diesel::update(wallets::table)
            .filter(
                wallets::project_id
                    .eq(project_id)
                    .and(wallets::id.eq(wallet_id)),
            )
            .set((wallets::pk.eq(pk), wallets::address.eq(address)))
            .execute(connection)
            .await?;
        Ok(updated > 0)
    }

    /// Get all wallet-project pairs for a chain category.
    /// Pass "solana" for Solana, or "evm" for all EVM chains.
    pub async fn get_wallets_projects_by_network(&self, network: &str) -> DbResult<Vec<(Wallet, Project)>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let data = if network == "evm" {
            wallets::table
                .inner_join(projects::table)
                .filter(
                    wallets::project_id
                        .eq(projects::id)
                        .and(projects::network.ne("solana")),
                )
                .select((Wallet::as_returning(), Project::as_returning()))
                .load(connection)
                .await?
        } else {
            wallets::table
                .inner_join(projects::table)
                .filter(
                    wallets::project_id
                        .eq(projects::id)
                        .and(projects::network.eq(network)),
                )
                .select((Wallet::as_returning(), Project::as_returning()))
                .load(connection)
                .await?
        };
        Ok(data)
    }
}
