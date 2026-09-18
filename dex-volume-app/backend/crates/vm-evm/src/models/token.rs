use std::ops::Div;
use std::str::FromStr;

use crate::{constants::Network, contracts::UniswapV4StateView, helpers::u256_to_f64};
use alloy::{
    primitives::{Address, FixedBytes, U256, address},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenFinancials {
    pub price: f64,
    pub liquidity: f64,
    pub market_cap: f64,
    pub eth: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CustomPool {
    pub pool: String,
    pub pair: Address,
    pub version: u32,
    pub fee: u32,
    pub balance: U256,
    pub tokens: U256,
    pub token_id: Option<String>,
    pub tick_spacing: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CustomToken {
    pub name: String,
    pub symbol: String,
    pub token: Address,
    pub owner: Address,
    pub decimals: u8,
    pub total_supply: U256,
    pub remaining_supply: U256,
    pub burned_supply: U256,
    pub pools: Vec<CustomPool>,
}

impl CustomToken {
    pub async fn get_pool_info(
        &self,
        pool: CustomPool,
        network: Network,
        network_price: f64,
    ) -> TokenFinancials {
        let mut price = 0.0f64;
        let mut liquidity = 0.0;
        let mut market_cap = 0.0;

        if pool.version == 2 || pool.version == 3 {
            let decimals_tokens = U256::from(10).pow(U256::from(self.decimals));
            // Check for stablecoin pairs (USDC on Base/Ethereum, USDT on Ethereum)
            if pool
                .pair
                .eq(&address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"))
                || pool
                    .pair
                    .eq(&address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"))
                || pool
                    .pair
                    .eq(&address!("dAC17F958D2ee523a2206206994597C13D831ec7"))
            {
                let decimals_pair = U256::from(10).pow(U256::from(6));

                price = u256_to_f64(pool.balance.div(decimals_pair))
                    / u256_to_f64(pool.tokens.div(decimals_tokens));

                let pair_tokens = u256_to_f64(pool.balance.div(decimals_pair));
                liquidity = pair_tokens;
                market_cap = price * u256_to_f64(self.remaining_supply.div(decimals_tokens));
            } else if pool
                .pair
                .eq(&address!("50c5725949A6F0c72E6C4a641F24049A917DB0Cb"))
            {
                // DAI on Base (18 decimals)
                let decimals_pair = U256::from(10).pow(U256::from(18));

                price = u256_to_f64(pool.balance.div(decimals_pair))
                    / u256_to_f64(pool.tokens.div(decimals_tokens));

                let pair_tokens = u256_to_f64(pool.balance.div(decimals_pair));
                liquidity = pair_tokens;
                market_cap = price * u256_to_f64(self.remaining_supply.div(decimals_tokens));
            } else if pool
                .pair
                .eq(&address!("fde4C96c8593536E31F229EA8f37b2ADa2699bb2"))
            {
                // USDT on Base (6 decimals)
                let decimals_pair = U256::from(10).pow(U256::from(6));

                price = u256_to_f64(pool.balance.div(decimals_pair))
                    / u256_to_f64(pool.tokens.div(decimals_tokens));

                let pair_tokens = u256_to_f64(pool.balance.div(decimals_pair));
                liquidity = pair_tokens;
                market_cap = price * u256_to_f64(self.remaining_supply.div(decimals_tokens));
            } else {
                // WETH/native pair
                price = network_price * u256_to_f64(pool.balance) / u256_to_f64(pool.tokens);
                liquidity = (price * u256_to_f64(pool.tokens.div(decimals_tokens)))
                    + (network_price
                        * u256_to_f64(pool.balance.div(U256::from(1_000_000_000_000_000_000u128))));
                market_cap = price * u256_to_f64(self.remaining_supply.div(decimals_tokens));
            }
        } else if pool.version == 4 {
            let uniswap_v4_state_view_address = match network {
                Network::Ethereum => {
                    Address::from_str("0x7ffe42c4a5deea5b0fec41c94c136cf115597227").unwrap()
                }
                Network::Base => {
                    Address::from_str("0xa3c0c9b65bad0b08107aa264b0f3db444b867a71").unwrap()
                }
                _ => {
                    return TokenFinancials {
                        eth: network_price,
                        price,
                        liquidity,
                        market_cap,
                    }
                }
            };

            let provider = network.get_provider();
            let uniswap_v4_state_view =
                UniswapV4StateView::new(uniswap_v4_state_view_address, provider);

            let pool_id: Result<FixedBytes<32>, alloy::hex::FromHexError> =
                FixedBytes::from_str(&pool.pool);

            if let Ok(pool_id) = pool_id {
                let slot0 = uniswap_v4_state_view.getSlot0(pool_id).call().await;
                if let Ok(slot0) = slot0 {
                    let q96 = U256::from(2).pow(U256::from(96));
                    let sqrt_price = slot0.sqrtPriceX96.to_string().parse::<f64>().unwrap()
                        / q96.to_string().parse::<f64>().unwrap();
                    price = (1.0 / (sqrt_price * sqrt_price)) * network_price;
                }
            }
        }

        TokenFinancials {
            eth: network_price,
            price,
            liquidity,
            market_cap,
        }
    }
}
