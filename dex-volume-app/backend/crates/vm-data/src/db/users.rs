use super::*;
use crate::db::schema::users;
use crate::models::user::{NewUser, User};

impl Database {
    pub async fn get_user_by_address(&self, user_address: &str) -> DbResult<Option<User>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let user = users::table
            .filter(users::address.eq(user_address))
            .first(connection)
            .await
            .optional()?;
        Ok(user)
    }

    pub async fn get_user_by_id(&self, id: i32) -> DbResult<Option<User>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let user = users::table
            .filter(users::id.eq(id))
            .first(connection)
            .await
            .optional()?;
        Ok(user)
    }

    pub async fn add_user(&self, new_user: &NewUser) -> DbResult<User> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let res: User = diesel::insert_into(users::table)
            .values(new_user)
            .returning(User::as_returning())
            .get_result(connection)
            .await?;
        Ok(res)
    }

    pub async fn delete_user(&self, user_id: &i32) -> DbResult<bool> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let deleted = diesel::delete(users::table.filter(users::id.eq(user_id)))
            .execute(connection)
            .await?;
        Ok(deleted > 0)
    }

    pub async fn delete_users(&self, addresses: &Vec<String>) -> DbResult<bool> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let deleted = diesel::delete(users::table.filter(users::address.eq_any(addresses)))
            .execute(connection)
            .await?;
        Ok(deleted > 0)
    }

    pub async fn set_user_refresh_token(
        &self,
        user_id: i32,
        token: &str,
        expires_at: SystemTime,
    ) -> DbResult<()> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        diesel::update(users::table)
            .filter(users::id.eq(user_id))
            .set((
                users::refresh_token.eq(Some(token)),
                users::refresh_token_expires_at.eq(Some(expires_at)),
            ))
            .execute(connection)
            .await?;
        Ok(())
    }

    pub async fn get_user_by_refresh_token(&self, token: &str) -> DbResult<Option<User>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let user = users::table
            .filter(users::refresh_token.eq(token))
            .first(connection)
            .await
            .optional()?;
        Ok(user)
    }

    pub async fn clear_user_refresh_token(&self, user_id: i32) -> DbResult<()> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        diesel::update(users::table)
            .filter(users::id.eq(user_id))
            .set((
                users::refresh_token.eq(None::<String>),
                users::refresh_token_expires_at.eq(None::<SystemTime>),
                users::session_id.eq(None::<String>),
            ))
            .execute(connection)
            .await?;
        Ok(())
    }

    pub async fn set_user_session_id(&self, user_id: i32, session_id: &str) -> DbResult<()> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        diesel::update(users::table)
            .filter(users::id.eq(user_id))
            .set(users::session_id.eq(Some(session_id)))
            .execute(connection)
            .await?;
        Ok(())
    }

    pub async fn get_user_session_id(&self, user_id: i32) -> DbResult<Option<String>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let sid: Option<Option<String>> = users::table
            .filter(users::id.eq(user_id))
            .select(users::session_id)
            .first(connection)
            .await
            .optional()?;
        Ok(sid.flatten())
    }

    pub async fn update_user_nonce(&self, user_address: &String, id: Uuid) -> DbResult<()> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        diesel::update(users::table)
            .filter(users::address.eq(user_address.clone()))
            .set((users::nonce.eq(id.to_string()),))
            .execute(connection)
            .await?;
        Ok(())
    }

    pub async fn update_user(&self, id: &i32, new_user: &NewUser) -> DbResult<bool> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let res: usize = diesel::update(users::table)
            .filter(users::id.eq(id))
            .set((
                users::nonce.eq(&new_user.nonce),
                users::whitelisted.eq(&new_user.whitelisted),
            ))
            .execute(connection)
            .await?;
        Ok(res > 0)
    }

    pub async fn update_user_address(&self, id: &i32, address: &str) -> DbResult<bool> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let res: usize = diesel::update(users::table)
            .filter(users::id.eq(id))
            .set(users::address.eq(address))
            .execute(connection)
            .await?;
        Ok(res > 0)
    }

    pub async fn get_users(&self) -> DbResult<Vec<User>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let data: Vec<User> = users::table
            .order(users::id.asc())
            .load(connection)
            .await?;
        Ok(data)
    }
}
