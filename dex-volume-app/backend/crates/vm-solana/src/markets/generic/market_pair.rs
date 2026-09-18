use crate::{
    markets::{
        meteora_damm::{
            models::{
                meteora_dynamic_amm::MeteoraDAMMPool, meteora_dynamic_amm_v2::MeteoraDAMMV2Pool,
            },
            utils::meteora_dynamic_amm::derive_token_vault_address,
        },
        meteora_dlmm::models::meteora_dynamic_lmm::MeteoraDLMMPool,
        pumpfun_amm::models::pumpfun_amm_pool::PumpfunAmmPool,
        raydium_amm_v4::models::raydium_amm_v4::RaydiumAMMV4,
        raydium_clmm::models::raydium_clmm_pool::RaydiumCLMMPool,
    },
    utils::constants::WSOL,
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MarketEnum {
    RaydiumAMMV4(RaydiumAMMV4),
    MeteoraDLMM(MeteoraDLMMPool),
    MeteoraDAMMV2(MeteoraDAMMV2Pool),
    MeteoraDAMM(MeteoraDAMMPool),
    PumpfunAMM(PumpfunAmmPool),
    RaydiumCLMM(RaydiumCLMMPool),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketPair {
    pub name: String,
    pub pair: String,
    pub market: MarketEnum,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenericPoolInfo {
    pub name: String,
    pub pair: String,
    pub quote_mint: String,
    pub base_mint: String,
    pub fees: f64,
    pub quote_vault: String,
    pub base_vault: String,
    pub quote_balance: f64,
    pub base_balance: f64,
}

impl MarketPair {
    /// Phase 3.4: Convert MarketPair to a trait object for polymorphic operations
    ///
    /// This allows using the Market trait interface with any DEX:
    /// ```
    /// let market = market_pair.to_market();
    /// let price = market.current_price()?;
    /// let ixs = market.build_swap_instruction(...).await?;
    /// ```
    pub fn to_market(&self) -> Box<dyn crate::markets::traits::Market> {
        match &self.market {
            MarketEnum::RaydiumAMMV4(pool) => Box::new(
                crate::markets::raydium_amm_v4::market_impl::RaydiumAMMV4Market::new(
                    pool.clone(),
                    self.clone(),
                ),
            ),
            MarketEnum::RaydiumCLMM(pool) => Box::new(
                crate::markets::raydium_clmm::market_impl::RaydiumCLMMMarket::new(
                    pool.clone(),
                    self.clone(),
                ),
            ),
            MarketEnum::MeteoraDLMM(pool) => Box::new(
                crate::markets::meteora_dlmm::market_impl::MeteoraDLMMMarket::new(
                    pool.clone(),
                    self.clone(),
                ),
            ),
            MarketEnum::MeteoraDAMM(pool) => Box::new(
                crate::markets::meteora_damm::market_impl::MeteoraDAMMMarket::new(
                    pool.clone(),
                    self.clone(),
                ),
            ),
            MarketEnum::MeteoraDAMMV2(pool) => Box::new(
                crate::markets::meteora_damm::market_impl_v2::MeteoraDAMMV2Market::new(
                    pool.clone(),
                    self.clone(),
                ),
            ),
            MarketEnum::PumpfunAMM(pool) => Box::new(
                crate::markets::pumpfun_amm::market_impl::PumpfunAMMMarket::new(
                    pool.clone(),
                    self.clone(),
                ),
            ),
        }
    }

    pub fn generic(&self) -> GenericPoolInfo {
        

        match &self.market {
            MarketEnum::RaydiumAMMV4(amm_info) => {
                let flipped = amm_info.quote_mint.to_string() != WSOL;
                GenericPoolInfo {
                    name: self.name.clone(),
                    pair: self.pair.clone(),
                    quote_mint: if flipped {
                        amm_info.base_mint.to_string()
                    } else {
                        amm_info.quote_mint.to_string()
                    },
                    base_mint: if flipped {
                        amm_info.quote_mint.to_string()
                    } else {
                        amm_info.base_mint.to_string()
                    },
                    fees: (amm_info.trade_fee_numerator as f64
                        / amm_info.trade_fee_denominator as f64)
                        * 100.0,
                    quote_vault: if flipped {
                        amm_info.base_vault.to_string()
                    } else {
                        amm_info.quote_vault.to_string()
                    },
                    base_vault: if flipped {
                        amm_info.quote_vault.to_string()
                    } else {
                        amm_info.base_vault.to_string()
                    },
                    quote_balance: 0.0,
                    base_balance: 0.0,
                }
            }
            MarketEnum::MeteoraDLMM(lb_pair) => GenericPoolInfo {
                name: self.name.clone(),
                pair: self.pair.clone(),
                quote_mint: lb_pair.token_y_mint.to_string(),
                base_mint: lb_pair.token_x_mint.to_string(),
                fees: (lb_pair.bin_step as f64 * lb_pair.parameters.base_factor as f64) / 1e6,
                quote_vault: lb_pair.reserve_y.to_string(),
                base_vault: lb_pair.reserve_x.to_string(),
                quote_balance: 0.0,
                base_balance: 0.0,
            },
            MarketEnum::MeteoraDAMMV2(pool) => GenericPoolInfo {
                name: self.name.clone(),
                pair: self.pair.clone(),
                quote_mint: pool.token_b_mint.to_string(),
                base_mint: pool.token_a_mint.to_string(),
                fees: (pool.pool_fees.base_fee.cliff_fee_numerator as f64 / 1e9) * 100.0,
                quote_vault: pool.token_b_vault.to_string(),
                base_vault: pool.token_a_vault.to_string(),
                quote_balance: 0.0,
                base_balance: 0.0,
            },
            MarketEnum::MeteoraDAMM(pool) => GenericPoolInfo {
                name: self.name.clone(),
                pair: self.pair.clone(),
                quote_mint: pool.token_b_mint.to_string(),
                base_mint: pool.token_a_mint.to_string(),
                fees: (pool.fees.trade_fee_numerator as f64
                    / pool.fees.trade_fee_denominator as f64)
                    * 100.0,
                quote_vault: derive_token_vault_address(pool.b_vault).0.to_string(),
                base_vault: derive_token_vault_address(pool.a_vault).0.to_string(),
                quote_balance: 0.0,
                base_balance: 0.0,
            },
            MarketEnum::PumpfunAMM(pumpfun_amm_pool) => GenericPoolInfo {
                name: self.name.clone(),
                pair: self.pair.clone(),
                quote_mint: pumpfun_amm_pool.quote_mint.to_string(),
                base_mint: pumpfun_amm_pool.base_mint.to_string(),
                fees: 0.25,
                quote_vault: pumpfun_amm_pool.pool_quote_token_account.to_string(),
                base_vault: pumpfun_amm_pool.pool_base_token_account.to_string(),
                quote_balance: 0.0,
                base_balance: 0.0,
            },
            MarketEnum::RaydiumCLMM(raydium_clmm_pool) => GenericPoolInfo {
                name: self.name.clone(),
                pair: self.pair.clone(),
                quote_mint: raydium_clmm_pool.token_mint_0.to_string(),
                base_mint: raydium_clmm_pool.token_mint_1.to_string(),
                fees: 0.0,
                quote_vault: raydium_clmm_pool.token_vault_0.to_string(),
                base_vault: raydium_clmm_pool.token_vault_1.to_string(),
                quote_balance: 0.0,
                base_balance: 0.0,
            },
        }
    }
}
