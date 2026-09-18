use super::*;
use crate::db::schema::{projects, transactions};
use crate::models::transaction::{NewTransaction, Transaction};
#[allow(unused_imports)] // Required by Diesel's inner_join derive on projects::table
use crate::models::project::Project;

impl Database {
    pub async fn add_transaction(&self, new_transaction: &NewTransaction) -> DbResult<Option<Transaction>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let res: Option<Transaction> = diesel::insert_into(transactions::table)
            .values(new_transaction)
            .on_conflict(transactions::tx_hash)
            .do_nothing()
            .returning(Transaction::as_returning())
            .get_result(connection)
            .await
            .optional()?;
        Ok(res)
    }

    pub async fn add_transactions(&self, new_transactions: &[NewTransaction]) -> DbResult<Vec<Transaction>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let res: Vec<Transaction> = diesel::insert_into(transactions::table)
            .values(new_transactions)
            .on_conflict(transactions::tx_hash)
            .do_nothing()
            .returning(Transaction::as_returning())
            .get_results(connection)
            .await?;
        Ok(res)
    }

    pub async fn update_transaction(
        &self,
        id: &i32,
        tx_type: String,
        token_in: String,
        token_out: String,
        new_value: f64,
        new_balance: f64,
    ) -> DbResult<bool> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let res: usize = diesel::update(transactions::table)
            .filter(transactions::id.eq(id))
            .set((
                transactions::tx_type.eq(&tx_type),
                transactions::token_in.eq(&token_in),
                transactions::token_out.eq(&token_out),
                transactions::value.eq(&f64_to_big_int(new_value)),
                transactions::tokens.eq(&f64_to_big_int(new_balance)),
            ))
            .execute(connection)
            .await?;
        Ok(res > 0)
    }

    pub async fn update_transaction_slot(
        &self,
        tx_hash: &str,
        slot: i64,
        value: Option<BigDecimal>,
        tokens: Option<BigDecimal>,
    ) -> DbResult<Option<Transaction>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;

        if let (Some(v), Some(t)) = (value, tokens) {
            let result = diesel::update(transactions::table)
                .filter(transactions::tx_hash.eq(tx_hash))
                .set((
                    transactions::slot.eq(slot),
                    transactions::value.eq(v),
                    transactions::tokens.eq(t),
                ))
                .returning(Transaction::as_returning())
                .get_result(connection)
                .await
                .ok();
            Ok(result)
        } else {
            let result = diesel::update(transactions::table)
                .filter(transactions::tx_hash.eq(tx_hash))
                .set(transactions::slot.eq(slot))
                .returning(Transaction::as_returning())
                .get_result(connection)
                .await
                .ok();
            Ok(result)
        }
    }

    pub async fn get_transactions(
        &self,
        project_id: i32,
        page: i32,
        limit: i32,
    ) -> DbResult<Vec<Transaction>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let data: Vec<Transaction> = transactions::table
            .filter(transactions::project_id.eq(project_id))
            .order(transactions::slot.desc())
            .offset(((page - 1) * limit) as i64)
            .limit(limit as i64)
            .select(Transaction::as_returning())
            .load(connection)
            .await?;
        Ok(data)
    }


    pub async fn get_transactions_count(&self, project_id: i32) -> DbResult<i64> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;

        let count = transactions::table
            .filter(transactions::project_id.eq(project_id))
            .count()
            .get_result(connection)
            .await?;
        Ok(count)
    }

    /// Get all transactions with slot=0 (unconfirmed) for EVM projects.
    pub async fn get_unconfirmed_evm_transactions(
        &self,
    ) -> DbResult<Vec<(Transaction, String)>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let evm_networks = vec!["ethereum", "base", "arbitrum", "bsc", "avalanche"];
        let data: Vec<(Transaction, String)> = transactions::table
            .inner_join(projects::table)
            .filter(transactions::slot.eq(0i64))
            .filter(projects::network.eq_any(evm_networks))
            .select((Transaction::as_returning(), projects::network))
            .load(connection)
            .await?;
        Ok(data)
    }

    pub async fn delete_transaction(&self, id: i32) -> DbResult<bool> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let res: usize = diesel::delete(transactions::table.filter(transactions::id.eq(id)))
            .execute(connection)
            .await?;
        Ok(res > 0)
    }
}
