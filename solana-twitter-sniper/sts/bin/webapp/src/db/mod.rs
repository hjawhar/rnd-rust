pub mod schema;
use crate::models::tasks::{NewTask, Task, UpdateTask};
use crate::models::user::{NewUser, User};
use crate::models::wallet::{NewWallet, Wallet, WalletRetrieved};
use dotenv::dotenv;
use schema::tasks;
use schema::users;
use schema::wallets;
use std::sync::Arc;
use std::{env, time::SystemTime};
use tokio::sync::Mutex;

use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::{
    async_connection_wrapper::AsyncConnectionWrapper, AsyncConnection, AsyncPgConnection,
    RunQueryDsl,
};

use uuid::Uuid;

use diesel::{self, prelude::*};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

#[derive(Clone)]
pub struct Database {
    pub connection: Arc<Mutex<AsyncPgConnection>>,
}

impl Database {
    pub async fn new() -> Self {
        dotenv().ok();

        let database_url = env::var("DATABASE_URL").expect("DATABASE_URL is required");
        let connection = AsyncPgConnection::establish(&database_url).await;
        match connection {
            Ok(conn) => Self {
                connection: Arc::new(Mutex::new(conn)),
            },
            Err(_) => panic!("Error connecting to {}", database_url),
        }
    }

    pub fn run_pending_migrations(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Should be in the form of postgres://user:password@localhost/database?sslmode=require
        let database_url = env::var("DATABASE_URL").expect("DATABASE_URL is required");
        let mut conn = AsyncConnectionWrapper::<AsyncPgConnection>::establish(&database_url)?;

        let abc = conn.run_pending_migrations(MIGRATIONS).unwrap();
        tracing::info!("{:#?}", abc);
        Ok(())
    }

    pub async fn get_user_by_address(&self, user_address: &String) -> Vec<User> {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let users_data: Vec<User> = users::table
            .filter(users::address.eq(user_address))
            .load(connection)
            .await
            .unwrap();
        users_data
    }

    pub async fn get_user_by_id(&self, id: i32) -> Option<User> {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let users_data: Vec<User> = users::table
            .filter(users::id.eq(id))
            .load(connection)
            .await
            .unwrap();

        if users_data.len() > 0 {
            Some(users_data[0].clone())
        } else {
            None
        }
    }

    pub async fn add_user(&self, new_user: &NewUser) -> User {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let res: User = diesel::insert_into(users::table)
            .values(new_user)
            .returning(User::as_returning())
            .get_result(connection)
            .await
            .unwrap();
        res
    }

    pub async fn delete_user(&self, user_id: &i32) -> bool {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let deleted = diesel::delete(users::table.filter(users::id.eq(user_id)))
            .execute(connection)
            .await
            .unwrap();
        return deleted > 0;
    }

    pub async fn delete_users(&self, addresses: &Vec<String>) -> bool {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let deleted = diesel::delete(users::table.filter(users::address.eq_any(addresses)))
            .execute(connection)
            .await
            .unwrap();
        return deleted > 0;
    }

    pub async fn update_user_nonce(&self, user_address: &String, id: Uuid) {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let _: usize = diesel::update(users::table)
            .filter(users::address.eq(user_address.clone()))
            .set((
                users::nonce.eq(id.to_string()),
                users::last_login_date_time.eq(SystemTime::now()),
            ))
            .execute(connection)
            .await
            .unwrap();
    }

    pub async fn update_user(&self, id: &i32, new_user: &NewUser) -> bool {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let res: usize = diesel::update(users::table)
            .filter(users::id.eq(id))
            .set((
                users::nonce.eq(&new_user.nonce),
                users::whitelisted.eq(&new_user.whitelisted),
            ))
            .execute(connection)
            .await
            .unwrap();
        res > 0
    }

    pub async fn update_user_address(&self, id: &i32, address: &str) -> bool {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let res: usize = diesel::update(users::table)
            .filter(users::id.eq(id))
            .set(users::address.eq(address))
            .execute(connection)
            .await
            .unwrap();
        res > 0
    }

    pub async fn get_users(&self) -> Vec<User> {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let data: Vec<User> = users::table
            .order(users::id.asc())
            .load(connection)
            .await
            .unwrap();
        data
    }

    // Tasks

    pub async fn add_task(&self, new_task: &NewTask) -> Task {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let res: Task = diesel::insert_into(tasks::table)
            .values(new_task)
            .returning(Task::as_returning())
            .get_result(connection)
            .await
            .unwrap();
        res
    }

    pub async fn get_tasks(&self) -> Vec<Task> {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let data: Vec<Task> = tasks::table
            .order(tasks::id.asc())
            .load(connection)
            .await
            .unwrap();
        data
    }

    pub async fn get_tasks_with_tweet_handle(&self, twitter_handle: String) -> Vec<Task> {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let data: Vec<Task> = tasks::table
            .filter(tasks::twitter_handle.eq(Some(twitter_handle)))
            .order(tasks::id.asc())
            .load(connection)
            .await
            .unwrap();
        data
    }

    pub async fn get_task(&self, task_id: i32) -> Option<Task> {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let tasks_data: Vec<Task> = tasks::table
            .filter(tasks::id.eq(task_id))
            .load(connection)
            .await
            .unwrap();

        if tasks_data.len() > 0 {
            Some(tasks_data[0].clone())
        } else {
            None
        }
    }

    pub async fn get_task_by_tweet_handle(&self, twitter_handle: String) -> Option<Task> {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let tasks_data: Vec<Task> = tasks::table
            .filter(tasks::twitter_handle.eq(twitter_handle))
            .load(connection)
            .await
            .unwrap();

        if tasks_data.len() > 0 {
            Some(tasks_data[0].clone())
        } else {
            None
        }
    }

    pub async fn update_task(&self, task_id: &i32, update_task: &UpdateTask) -> bool {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let res: usize = diesel::update(tasks::table)
            .filter(tasks::id.eq(task_id))
            .set((
                tasks::wallet_id.eq(&update_task.wallet_id),
                tasks::twitter_id.eq(&update_task.twitter_id),
                tasks::twitter_handle.eq(&update_task.twitter_handle),
                tasks::servers.eq(&update_task.servers),
                tasks::block_leaders.eq(&update_task.block_leaders),
                tasks::value.eq(&update_task.value),
                tasks::tip.eq(&update_task.tip),
                tasks::slippage.eq(&update_task.slippage),
                tasks::tries.eq(&update_task.tries),
                tasks::frontrunning_protection.eq(&update_task.frontrunning_protection),
                tasks::enable_alerts.eq(&update_task.enable_alerts),
                tasks::selected_pool.eq(&update_task.selected_pool),
                tasks::twitter_api.eq(&update_task.twitter_api),
                tasks::twitter_strategy.eq(&update_task.twitter_strategy),
                tasks::twitter_handle_checker.eq(&update_task.twitter_handle_checker),
                tasks::twitter_token_override.eq(&update_task.twitter_token_override),
                tasks::words.eq(&update_task.words),
            ))
            .execute(connection)
            .await
            .unwrap();
        res > 0
    }

    pub async fn delete_task(&self, task_id: &i32) -> bool {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let deleted = diesel::delete(tasks::table.filter(tasks::id.eq(task_id)))
            .execute(connection)
            .await
            .unwrap();
        return deleted > 0;
    }

    // Wallet

    pub async fn add_wallet(&self, new_wallet: &NewWallet) -> WalletRetrieved {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let wallet = diesel::insert_into(wallets::table)
            .values(new_wallet)
            .returning((
                wallets::id,
                wallets::address,
                wallets::name,
                wallets::comments,
                wallets::nonce_account_address,
            ))
            .get_result(connection)
            .await
            .unwrap();
        return wallet;
    }

    pub async fn update_wallet_account_nonce(
        &self,
        id: &i32,
        nonce_account_address: &String,
    ) -> bool {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let res: usize = diesel::update(wallets::table)
            .filter(wallets::id.eq(id))
            .set((wallets::nonce_account_address.eq(Some(nonce_account_address))))
            .execute(connection)
            .await
            .unwrap();
        res > 0
    }

    pub async fn get_wallet(&self, wallet_id: &i32) -> Option<Wallet> {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let wallets_data: Vec<Wallet> = wallets::table
            .filter(wallets::id.eq(wallet_id))
            .load(connection)
            .await
            .unwrap();

        if wallets_data.len() > 0 {
            Some(wallets_data[0].clone())
        } else {
            None
        }
    }

    pub async fn get_all_wallets(&self) -> Vec<Wallet> {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let wallets_data: Vec<Wallet> = wallets::table.load(connection).await.unwrap();
        return wallets_data;
    }

    pub async fn get_wallet_by_addresses(
        &self,
        user_id: Option<i32>,
        addresses: &Vec<String>,
    ) -> Vec<Wallet> {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        if let Some(user_id) = user_id {
            let wallets_data: Vec<Wallet> = wallets::table
                .filter(
                    wallets::user_id
                        .eq(user_id)
                        .and(wallets::address.eq_any(addresses)),
                )
                .load(connection)
                .await
                .unwrap();
            wallets_data
        } else {
            let wallets_data: Vec<Wallet> = wallets::table
                .filter(wallets::address.eq_any(addresses))
                .load(connection)
                .await
                .unwrap();
            wallets_data
        }
    }

    pub async fn get_wallets(&self) -> Vec<WalletRetrieved> {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        let wallets_data = wallets::table
            .select((
                wallets::id,
                wallets::address,
                wallets::name,
                wallets::comments,
                wallets::nonce_account_address,
            ))
            .load(connection)
            .await
            .unwrap();
        wallets_data
    }

    pub async fn get_wallets_by_ids_full(
        &self,
        user_id: Option<i32>,
        wallet_ids: &Vec<i32>,
    ) -> Vec<Wallet> {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;

        if let Some(user_id) = user_id {
            let wallets_data = wallets::table
                .filter(
                    wallets::user_id
                        .eq(user_id)
                        .and(wallets::id.eq_any(wallet_ids)),
                )
                .select(Wallet::as_returning())
                .load(connection)
                .await
                .unwrap();
            wallets_data
        } else {
            let wallets_data = wallets::table
                .filter(wallets::id.eq_any(wallet_ids))
                .select(Wallet::as_returning())
                .load(connection)
                .await
                .unwrap();
            wallets_data
        }
    }

    pub async fn delete_wallets(&self, user_id: Option<i32>, wallet_ids: &Vec<i32>) -> bool {
        let mut lock_guard = self.connection.lock().await;
        let connection = &mut *lock_guard;
        if let Some(user_id) = user_id {
            let deleted = diesel::delete(
                wallets::table.filter(
                    wallets::user_id
                        .eq(user_id)
                        .and(wallets::id.eq_any(wallet_ids)),
                ),
            )
            .execute(connection)
            .await
            .unwrap();
            return deleted > 0;
        } else {
            let deleted = diesel::delete(wallets::table.filter(wallets::id.eq_any(wallet_ids)))
                .execute(connection)
                .await
                .unwrap();
            return deleted > 0;
        }
    }
}
