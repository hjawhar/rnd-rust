use super::*;
use crate::db::schema::uniswap_v4_pools;
use crate::models::uniswap::{NewUniswapV4Pool, UniswapV4Pool};

impl Database {
    pub async fn add_v4_pool(&self, new_pool: &NewUniswapV4Pool) -> DbResult<UniswapV4Pool> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let pool = diesel::insert_into(uniswap_v4_pools::table)
            .values(new_pool)
            .returning(UniswapV4Pool::as_returning())
            .get_result(connection)
            .await?;
        Ok(pool)
    }

    pub async fn get_latest_uniswap_v4_pools_by_address(
        &self,
        network_id: i32,
        address: String,
    ) -> DbResult<Vec<UniswapV4Pool>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let pools = uniswap_v4_pools::table
            .filter(
                uniswap_v4_pools::network_id
                    .eq(network_id)
                    .and(
                        uniswap_v4_pools::currency0
                            .eq(&address)
                            .or(uniswap_v4_pools::currency1.eq(&address)),
                    ),
            )
            .load(connection)
            .await?;
        Ok(pools)
    }

    pub async fn get_latest_uniswap_v4_pool_by_network(
        &self,
        network_id: i32,
    ) -> DbResult<Option<UniswapV4Pool>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let pool = uniswap_v4_pools::table
            .filter(uniswap_v4_pools::network_id.eq(network_id))
            .order(uniswap_v4_pools::token_id.desc())
            .first(connection)
            .await
            .optional()?;
        Ok(pool)
    }
}
