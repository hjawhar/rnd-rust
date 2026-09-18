use super::*;
use crate::db::schema::{project_access, projects, transactions, wallets};
use crate::models::project::{NewProject, Project, UpdateProjectPayload};
use crate::models::transaction::{TransactionSummaryBuy, TransactionSummaryOverall, TransactionSummarySell};
use crate::models::wallet::{NewWallet, Wallet};

impl Database {
    // Projects

    pub async fn add_project(&self, new_project: &NewProject) -> DbResult<Project> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let res: Project = diesel::insert_into(projects::table)
            .values(new_project)
            .returning(Project::as_returning())
            .get_result(connection)
            .await?;
        Ok(res)
    }

    /// Atomically insert a project and its main wallet in a single DB transaction.
    /// If either insert fails, both are rolled back — no orphaned records.
    pub async fn add_project_with_wallet(
        &self,
        new_project: &NewProject,
        new_wallet: &NewWallet,
    ) -> DbResult<(Project, Wallet)> {
        use diesel_async::AsyncConnection;
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let new_project = new_project.clone();
        let new_wallet = new_wallet.clone();
        connection
            .transaction::<(Project, Wallet), diesel::result::Error, _>(|conn| {
                Box::pin(async move {
                    let project: Project = diesel::insert_into(projects::table)
                        .values(&new_project)
                        .returning(Project::as_returning())
                        .get_result(conn)
                        .await?;

                    let mut wallet_to_insert = new_wallet;
                    wallet_to_insert.project_id = project.id;

                    let wallet: Wallet = diesel::insert_into(wallets::table)
                        .values(&wallet_to_insert)
                        .returning(Wallet::as_returning())
                        .get_result(conn)
                        .await?;

                    Ok((project, wallet))
                })
            })
            .await
            .map_err(|e: diesel::result::Error| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }

    pub async fn get_all_projects(&self) -> DbResult<Vec<Project>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let data: Vec<Project> = projects::table
            .order(projects::id.asc())
            .load(connection)
            .await?;
        Ok(data)
    }

    pub async fn get_projects(&self, user_id: i32) -> DbResult<Vec<Project>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let shared_project_ids = project_access::table
            .filter(project_access::user_id.eq(user_id))
            .select(project_access::project_id);
        let data: Vec<Project> = projects::table
            .filter(
                projects::user_id.eq(user_id)
                    .or(projects::id.eq_any(shared_project_ids))
            )
            .order(projects::id.asc())
            .load(connection)
            .await?;
        Ok(data)
    }

    pub async fn get_project(&self, user_id: i32, project_id: i32) -> DbResult<Option<Project>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let shared_project_ids = project_access::table
            .filter(project_access::user_id.eq(user_id))
            .select(project_access::project_id);
        let project = projects::table
            .filter(
                projects::id.eq(project_id).and(
                    projects::user_id.eq(user_id)
                        .or(projects::id.eq_any(shared_project_ids))
                ),
            )
            .first(connection)
            .await
            .optional()?;
        Ok(project)
    }

    /// Admin-only: fetch a project by ID without user authorization checks.
    pub async fn get_project_by_id(&self, project_id: i32) -> DbResult<Option<Project>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let project = projects::table
            .filter(projects::id.eq(project_id))
            .first(connection)
            .await
            .optional()?;
        Ok(project)
    }

    /// Lightweight query: returns (network, locked) for a project the user is authorized to access.
    /// Avoids fetching the full Project row when only these fields are needed for chain routing + killswitch.
    pub async fn get_project_network(&self, user_id: i32, project_id: i32) -> DbResult<Option<(String, bool)>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let shared_project_ids = project_access::table
            .filter(project_access::user_id.eq(user_id))
            .select(project_access::project_id);
        let result: Option<(String, bool)> = projects::table
            .filter(
                projects::id.eq(project_id).and(
                    projects::user_id.eq(user_id)
                        .or(projects::id.eq_any(shared_project_ids))
                ),
            )
            .select((projects::network, projects::locked))
            .first(connection)
            .await
            .optional()?;
        Ok(result)
    }

    pub async fn get_project_statistics_overall(
        &self,
        project_id: i32,
    ) -> DbResult<(Option<BigDecimal>, Option<BigDecimal>)> {
        use diesel::dsl::sum;

        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;

        let result = transactions::table
            .filter(transactions::project_id.eq(project_id))
            .select((
                sum(transactions::value),
                sum(transactions::value.mul(transactions::sol_price)),
            ))
            .first(connection)
            .await?;
        Ok(result)
    }

    pub async fn get_project_statistics_overall_interval(
        &self,
        project_id: i32,
        interval: String,
    ) -> DbResult<(
        TransactionSummaryOverall,
        TransactionSummaryBuy,
        TransactionSummarySell,
    )> {
        use diesel::sql_query;
        use diesel::sql_types::{Integer, Text};

        let (results_overall, results_buys, results_sells) = tokio::join!(
            async {
                let mut conn = self.get_connection().await?;
                sql_query(
                    "SELECT \
                        COALESCE(SUM(value), 0) as value, \
                        COALESCE(SUM(value * sol_price), 0) as total \
                    FROM transactions WHERE \
                        project_id = $1 \
                        AND date_added >= NOW() - $2::interval"
                )
                    .bind::<Integer, _>(project_id)
                    .bind::<Text, _>(&interval)
                    .get_result::<TransactionSummaryOverall>(&mut *conn)
                    .await
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
            },
            async {
                let mut conn = self.get_connection().await?;
                sql_query(
                    "SELECT \
                        COALESCE(SUM(value), 0) as value, \
                        COALESCE(SUM(tokens), 0) as tokens, \
                        COALESCE(SUM(value * sol_price), 0) as total \
                    FROM transactions WHERE \
                        project_id = $1 \
                        AND tx_type = 'BUY' \
                        AND date_added >= NOW() - $2::interval"
                )
                    .bind::<Integer, _>(project_id)
                    .bind::<Text, _>(&interval)
                    .get_result::<TransactionSummaryBuy>(&mut *conn)
                    .await
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
            },
            async {
                let mut conn = self.get_connection().await?;
                sql_query(
                    "SELECT \
                        COALESCE(SUM(value), 0) as value, \
                        COALESCE(SUM(tokens), 0) as tokens, \
                        COALESCE(SUM(value * sol_price), 0) as total \
                    FROM transactions WHERE \
                        project_id = $1 \
                        AND tx_type = 'SELL' \
                        AND date_added >= NOW() - $2::interval"
                )
                    .bind::<Integer, _>(project_id)
                    .bind::<Text, _>(&interval)
                    .get_result::<TransactionSummarySell>(&mut *conn)
                    .await
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
            },
        );

        Ok((
            results_overall?,
            results_buys?,
            results_sells?,
        ))
    }

    pub async fn get_project_statistics_buy(
        &self,
        project_id: i32,
    ) -> DbResult<(Option<BigDecimal>, Option<BigDecimal>, Option<BigDecimal>)> {
        use diesel::dsl::sum;

        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;

        let result = transactions::table
            .filter(
                transactions::project_id
                    .eq(project_id)
                    .and(transactions::tx_type.eq("BUY")),
            )
            .select((
                sum(transactions::value),
                sum(transactions::tokens),
                sum(transactions::value.mul(transactions::sol_price)),
            ))
            .first(connection)
            .await?;
        Ok(result)
    }

    pub async fn get_project_statistics_sell(
        &self,
        project_id: i32,
    ) -> DbResult<(Option<BigDecimal>, Option<BigDecimal>, Option<BigDecimal>)> {
        use diesel::dsl::sum;

        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;

        let result = transactions::table
            .filter(
                transactions::project_id
                    .eq(project_id)
                    .and(transactions::tx_type.eq("SELL")),
            )
            .select((
                sum(transactions::value),
                sum(transactions::tokens),
                sum(transactions::value.mul(transactions::sol_price)),
            ))
            .first(connection)
            .await?;
        Ok(result)
    }

    pub async fn update_project(
        &self,
        user_id: &i32,
        project_id: &i32,
        update_project: &UpdateProjectPayload,
    ) -> DbResult<bool> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let shared_project_ids = project_access::table
            .filter(project_access::user_id.eq(user_id))
            .select(project_access::project_id);
        let res: usize = diesel::update(projects::table)
            .filter(
                projects::id.eq(project_id).and(
                    projects::user_id.eq(user_id)
                        .or(projects::id.eq_any(shared_project_ids))
                ),
            )
            .set(update_project)
            .execute(connection)
            .await?;
        Ok(res > 0)
    }

    pub async fn update_project_fees(&self, project_id: &i32, fees: BigDecimal) -> DbResult<bool> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let res: usize = diesel::update(projects::table)
            .filter(projects::id.eq(project_id))
            .set(projects::fees.eq(&fees))
            .execute(connection)
            .await?;
        Ok(res > 0)
    }

    pub async fn update_project_info(
        &self,
        project_id: &i32,
        pool: String,
        pool_type: String,
    ) -> DbResult<bool> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let res: usize = diesel::update(projects::table)
            .filter(projects::id.eq(project_id))
            .set((projects::pool_type.eq(&pool_type), projects::pool.eq(&pool)))
            .execute(connection)
            .await?;
        Ok(res > 0)
    }

    pub async fn update_project_status(&self, project_id: i32, status: &str) -> DbResult<bool> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let res: usize = diesel::update(projects::table)
            .filter(projects::id.eq(project_id))
            .set(projects::status.eq(status))
            .execute(connection)
            .await?;
        Ok(res > 0)
    }

    /// Bulk reset all projects for a network to 'stopped'.
    /// Pass "solana" for Solana projects, or "evm" for all EVM chains.
    pub async fn reset_project_statuses(&self, network: &str) -> DbResult<i64> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let count = if network == "evm" {
            diesel::update(projects::table)
                .filter(projects::network.ne("solana").and(projects::status.ne("stopped")))
                .set(projects::status.eq("stopped"))
                .execute(connection)
                .await?
        } else {
            diesel::update(projects::table)
                .filter(projects::network.eq(network).and(projects::status.ne("stopped")))
                .set(projects::status.eq("stopped"))
                .execute(connection)
                .await?
        };
        Ok(count as i64)
    }

    pub async fn delete_project(&self, project_id: &i32) -> DbResult<bool> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let deleted = diesel::delete(projects::table.filter(projects::id.eq(project_id)))
            .execute(connection)
            .await?;
        Ok(deleted > 0)
    }

    // Cross-entity project operations

    /// Get all projects for a chain category.
    /// Pass "solana" for Solana projects, or "evm" for all EVM chains (base, ethereum, etc.).
    pub async fn get_all_projects_by_network(&self, network: &str) -> DbResult<Vec<Project>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let projects = if network == "evm" {
            projects::table
                .filter(projects::network.ne("solana"))
                .order(projects::id.asc())
                .load(connection)
                .await?
        } else {
            projects::table
                .filter(projects::network.eq(network))
                .order(projects::id.asc())
                .load(connection)
                .await?
        };
        Ok(projects)
    }

    /// Get all projects with status = 'running' and not locked (for orphan task detection).
    /// Returns (id, user_id, network) tuples.
    pub async fn get_running_projects(&self) -> DbResult<Vec<(i32, i32, String)>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let data: Vec<(i32, i32, String)> = projects::table
            .filter(projects::status.eq("running").and(projects::locked.eq(false)))
            .select((projects::id, projects::user_id, projects::network))
            .load(connection)
            .await?;
        Ok(data)
    }

    /// Admin killswitch: set or clear the locked flag on a project.
    /// Also sets status to "stopped" and clears lock_at when locking.
    pub async fn set_project_locked(&self, project_id: i32, locked: bool) -> DbResult<Option<(String, String)>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        if locked {
            diesel::update(projects::table)
                .filter(projects::id.eq(project_id))
                .set((
                    projects::locked.eq(true),
                    projects::status.eq("stopped"),
                    projects::lock_at.eq(None::<SystemTime>),
                ))
                .execute(connection)
                .await?;
        } else {
            diesel::update(projects::table)
                .filter(projects::id.eq(project_id))
                .set((
                    projects::locked.eq(false),
                    projects::lock_at.eq(None::<SystemTime>),
                ))
                .execute(connection)
                .await?;
        }
        // Return (network, status) so the caller can send stop command if needed
        let result: Option<(String, String)> = projects::table
            .filter(projects::id.eq(project_id))
            .select((projects::network, projects::status))
            .first(connection)
            .await
            .optional()?;
        Ok(result)
    }

    /// Set a scheduled lock time for a project.
    pub async fn set_project_lock_at(&self, project_id: i32, lock_at: SystemTime) -> DbResult<bool> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let res: usize = diesel::update(projects::table)
            .filter(projects::id.eq(project_id))
            .set(projects::lock_at.eq(Some(lock_at)))
            .execute(connection)
            .await?;
        Ok(res > 0)
    }

    /// Get all unlocked projects whose lock_at time has passed.
    /// Returns (id, user_id, network, status) tuples.
    pub async fn get_projects_due_for_lock(&self) -> DbResult<Vec<(i32, i32, String, String)>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let now = SystemTime::now();
        let data: Vec<(i32, i32, String, String)> = projects::table
            .filter(
                projects::locked.eq(false)
                    .and(projects::lock_at.is_not_null())
                    .and(projects::lock_at.le(now)),
            )
            .select((projects::id, projects::user_id, projects::network, projects::status))
            .load(connection)
            .await?;
        Ok(data)
    }
}
