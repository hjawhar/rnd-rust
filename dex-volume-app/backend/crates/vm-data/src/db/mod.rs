pub mod schema;

mod access;
mod audit;
mod projects;
mod subscriptions;
mod transactions;
mod uniswap;
mod users;
mod wallets;

// Shared traits and utilities — sub-modules access these via `use super::*;`
use bigdecimal::BigDecimal;
use dotenv::dotenv;
use std::env;
use std::ops::Mul;
use std::time::SystemTime;
use crate::utils::helpers::f64_to_big_int;
use uuid::Uuid;

use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::{
    AsyncPgConnection, RunQueryDsl, async_connection_wrapper::AsyncConnectionWrapper,
    pooled_connection::AsyncDieselConnectionManager, pooled_connection::deadpool::Pool,
};


use diesel::{self, prelude::*};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

pub type DbResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone)]
pub struct Database {
    pool: Pool<AsyncPgConnection>,
}

impl Database {
    pub async fn new(max_pool_size: usize) -> Self {
        dotenv().ok();
        let database_url = env::var("DATABASE_URL").expect("DATABASE_URL is required");
        Self::new_with_url(&database_url, max_pool_size).await
    }

    pub async fn new_with_url(database_url: &str, max_pool_size: usize) -> Self {
        let config = AsyncDieselConnectionManager::<AsyncPgConnection>::new(database_url);

        let pool = Pool::builder(config)
            .max_size(max_pool_size)
            .wait_timeout(Some(std::time::Duration::from_secs(5)))
            .create_timeout(Some(std::time::Duration::from_secs(5)))
            .recycle_timeout(Some(std::time::Duration::from_secs(5)))
            .runtime(deadpool::Runtime::Tokio1)
            .build()
            .expect("Failed to create database pool");

        tracing::info!("Database pool created successfully (max_size={})", max_pool_size);

        Self { pool }
    }

    /// Pre-warm the connection pool by eagerly creating connections.
    /// Avoids cold-start latency on first queries after startup.
    pub async fn warm_pool(&self, count: usize) {
        let max = self.pool.status().max_size;
        let target = count.min(max);
        let mut handles = Vec::with_capacity(target);
        for _ in 0..target {
            match self.pool.get().await {
                Ok(conn) => handles.push(conn),
                Err(e) => {
                    tracing::warn!("Failed to pre-warm DB connection: {}", e);
                    break;
                }
            }
        }
        tracing::info!("Pre-warmed {} DB connections", handles.len());
        // handles drop here — connections return to pool but stay alive
    }

    async fn get_connection(
        &self,
    ) -> DbResult<diesel_async::pooled_connection::deadpool::Object<AsyncPgConnection>> {
        self.pool
            .get()
            .await
            .map_err(|e| {
                tracing::error!("Failed to get DB connection from pool: {}", e);
                Box::new(e) as Box<dyn std::error::Error + Send + Sync>
            })
    }

    pub async fn check_connection(&self) -> DbResult<()> {
        let mut conn = self.get_connection().await?;

        diesel::sql_query("SELECT 1")
            .execute(&mut *conn)
            .await
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                Box::new(std::io::Error::other(
                    e.to_string(),
                ))
            })?;

        tracing::debug!("Database connection health check passed");
        Ok(())
    }

    pub fn run_pending_migrations(&self) -> Result<(), Box<dyn std::error::Error>> {
        let database_url = env::var("DATABASE_URL").expect("DATABASE_URL is required");
        let mut conn = AsyncConnectionWrapper::<AsyncPgConnection>::establish(&database_url)?;

        let migrations = conn.run_pending_migrations(MIGRATIONS).unwrap();
        tracing::info!("{:#?}", migrations);
        Ok(())
    }
}
