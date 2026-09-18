use std::{collections::HashMap, error::Error, sync::Arc};

use crate::{
    markets::{
        generic::market_pair::{GenericPoolInfo, MarketEnum, MarketPair},
        meteora_damm::models::{
            meteora_dynamic_amm::MeteoraDAMMPool, meteora_dynamic_amm_v2::MeteoraDAMMV2Pool,
        },
        meteora_dlmm::models::meteora_dynamic_lmm::MeteoraDLMMPool,
        pumpfun_amm::{
            models::{
                pumpfun_amm_pool::PumpfunAmmPool, pumpfun_bonding_curve::PumpfunBondingCurve,
            },
            utils::pumpfun::{get_bonding_curve_pda, get_pumpfun_amm_pool},
        },
        raydium_amm_v4::models::{raydium_amm_config::AmmConfig, raydium_amm_v4::RaydiumAMMV4},
        raydium_clmm::models::raydium_clmm_pool::RaydiumCLMMPool,
    },
    utils::constants::{
        METEORA_DYNAMIC_AMM, METEORA_DYNAMIC_AMM_V2, METEORA_DYNAMIC_LMM, RAYDIUM_CLMM,
        RAYDIUM_LIQUIDITY_POOL_V4, USDC, WSOL,
    },
};

use base64::Engine;
use borsh::BorshDeserialize;
use futures::future;
use solana_account_decoder_client_types::{UiAccountData, UiAccountEncoding};
use solana_commitment_config::CommitmentConfig;
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_rpc_client_api::{
    config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
    filter::{Memcmp, MemcmpEncodedBytes, RpcFilterType},
};
use tokio::{sync::Mutex, task::JoinHandle};

/// Decode RPC UI accounts into (Pubkey, raw_bytes) tuples
fn decode_ui_accounts(
    accounts: Vec<(Pubkey, solana_account_decoder_client_types::UiAccount)>,
) -> Vec<(Pubkey, Vec<u8>)> {
    accounts
        .into_iter()
        .filter_map(|(pubkey, ui_account)| match ui_account.data {
            UiAccountData::Binary(data, UiAccountEncoding::Base64) => {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(&data)
                    .ok()?;
                Some((pubkey, decoded))
            }
            _ => None,
        })
        .collect()
}

pub async fn get_meteora_dynamic_amm_pools(
    rpc: Arc<RpcClient>,
    mint: String,
) -> Result<Vec<MarketPair>, Box<dyn Error + Send + Sync>> {
    let mut market_pairs: Vec<MarketPair> = vec![];

    {
        let data_size = 944;
        // println!("Searching with data size: {} bytes", data_size);

        // Create filters for both token_a_mint and token_b_mint
        let filters_a = vec![
            RpcFilterType::DataSize(data_size),
            RpcFilterType::Memcmp(Memcmp::new(
                40, // offset for token_a_mint
                MemcmpEncodedBytes::Base58(mint.clone()),
            )),
        ];

        let accounts = rpc
            .get_program_ui_accounts_with_config(
                &Pubkey::from_str_const(METEORA_DYNAMIC_AMM),
                RpcProgramAccountsConfig {
                    filters: Some(filters_a),
                    account_config: RpcAccountInfoConfig {
                        commitment: Some(CommitmentConfig::confirmed()),
                        encoding: Some(UiAccountEncoding::Base64),
                        ..RpcAccountInfoConfig::default()
                    },
                    ..RpcProgramAccountsConfig::default()
                },
            )
            .await;

        let accounts = decode_ui_accounts(accounts.unwrap_or(vec![]));
        for account in accounts.iter() {
            if let Ok(pool) = MeteoraDAMMPool::deserialize(&mut &account.1[8..]) {
                market_pairs.push(MarketPair {
                    name: "Meteora DAMM".to_string(),
                    pair: account.0.to_string(),
                    market: MarketEnum::MeteoraDAMM(pool),
                });
            }
        }
    }

    {
        let data_size = 952;
        // println!("Searching with data size: {} bytes", data_size);

        // Create filters for both token_a_mint and token_b_mint
        let filters_a = vec![
            RpcFilterType::DataSize(data_size),
            RpcFilterType::Memcmp(Memcmp::new(
                40, // offset for token_a_mint
                MemcmpEncodedBytes::Base58(mint.clone()),
            )),
        ];

        let accounts = rpc
            .get_program_ui_accounts_with_config(
                &Pubkey::from_str_const(METEORA_DYNAMIC_AMM),
                RpcProgramAccountsConfig {
                    filters: Some(filters_a),
                    account_config: RpcAccountInfoConfig {
                        commitment: Some(CommitmentConfig::confirmed()),
                        encoding: Some(UiAccountEncoding::Base64),
                        ..RpcAccountInfoConfig::default()
                    },
                    ..RpcProgramAccountsConfig::default()
                },
            )
            .await;

        let accounts = decode_ui_accounts(accounts.unwrap_or(vec![]));
        for account in accounts.iter() {
            if let Ok(pool) = MeteoraDAMMPool::deserialize(&mut &account.1[8..]) {
                market_pairs.push(MarketPair {
                    name: "Meteora DAMM".to_string(),
                    pair: account.0.to_string(),
                    market: MarketEnum::MeteoraDAMM(pool),
                });
            }
        }
    }
    Ok(market_pairs)
}

pub async fn get_meteora_dynamic_amm_v2_pools(
    rpc: Arc<RpcClient>,
    mint: String,
) -> Result<Vec<MarketPair>, Box<dyn Error + Send + Sync>> {
    let data_size = 1112; // std::mem::size_of::<api::markets::meteora::Pool>() as u64;
    let mut market_pairs: Vec<MarketPair> = vec![];
    let mut accounts: Vec<(Pubkey, Vec<u8>)> = vec![];

    {
        // Create filters for both token_a_mint and token_b_mint
        let filters = vec![
            RpcFilterType::DataSize(data_size),
            RpcFilterType::Memcmp(Memcmp::new(
                200, // offset for token_a_mint
                MemcmpEncodedBytes::Base58(mint.clone()),
            )),
        ];

        let accounts1 = rpc
            .get_program_ui_accounts_with_config(
                &Pubkey::from_str_const(METEORA_DYNAMIC_AMM_V2),
                RpcProgramAccountsConfig {
                    filters: Some(filters),
                    account_config: RpcAccountInfoConfig {
                        commitment: Some(CommitmentConfig::confirmed()),
                        encoding: Some(UiAccountEncoding::Base64),
                        ..RpcAccountInfoConfig::default()
                    },
                    ..RpcProgramAccountsConfig::default()
                },
            )
            .await?;

        accounts.append(&mut decode_ui_accounts(accounts1));
    }

    {
        // Create filters for both token_a_mint and token_b_mint
        let filters = vec![
            RpcFilterType::DataSize(data_size),
            RpcFilterType::Memcmp(Memcmp::new(
                168, // offset for token_a_mint
                MemcmpEncodedBytes::Base58(mint.clone()),
            )),
        ];

        let accounts1 = rpc
            .get_program_ui_accounts_with_config(
                &Pubkey::from_str_const(METEORA_DYNAMIC_AMM_V2),
                RpcProgramAccountsConfig {
                    filters: Some(filters),
                    account_config: RpcAccountInfoConfig {
                        commitment: Some(CommitmentConfig::confirmed()),
                        encoding: Some(UiAccountEncoding::Base64),
                        ..RpcAccountInfoConfig::default()
                    },
                    ..RpcProgramAccountsConfig::default()
                },
            )
            .await?;

        accounts.append(&mut decode_ui_accounts(accounts1));
    }

    if accounts.iter().len() > 0 {
        for account in accounts {
            if let Ok(pool) = MeteoraDAMMV2Pool::deserialize(&mut &account.1[8..]) {
                market_pairs.push(MarketPair {
                    name: "Meteora DAMMV2".to_string(),
                    pair: account.0.to_string(),
                    market: MarketEnum::MeteoraDAMMV2(pool),
                });
            }
        }
    }
    Ok(market_pairs)
}

pub async fn get_meteora_dynamic_lmm_pools(
    rpc: Arc<RpcClient>,
    mint: String,
) -> Result<Vec<MarketPair>, Box<dyn Error + Send + Sync>> {
    let data_size = 904; // std::mem::size_of::<api::markets::meteora::Pool>() as u64;

    // Create filters for both token_a_mint and token_b_mint
    let filters = vec![
        RpcFilterType::DataSize(data_size),
        RpcFilterType::Memcmp(Memcmp::new(
            88, // offset for token_a_mint
            MemcmpEncodedBytes::Base58(mint),
        )),
    ];

    let accounts = rpc
        .get_program_ui_accounts_with_config(
            &Pubkey::from_str_const(METEORA_DYNAMIC_LMM),
            RpcProgramAccountsConfig {
                filters: Some(filters),
                account_config: RpcAccountInfoConfig {
                    commitment: Some(CommitmentConfig::confirmed()),
                    encoding: Some(UiAccountEncoding::Base64),
                    ..RpcAccountInfoConfig::default()
                },
                ..RpcProgramAccountsConfig::default()
            },
        )
        .await;

    let accounts = decode_ui_accounts(accounts.unwrap_or(vec![]));
    let mut market_pairs: Vec<MarketPair> = vec![];
    for account in accounts.iter() {
        if let Ok(lp_pair) = MeteoraDLMMPool::deserialize(&mut &account.1[8..]) {
            market_pairs.push(MarketPair {
                name: "Meteora DLMM".to_string(),
                pair: account.0.to_string(),
                market: MarketEnum::MeteoraDLMM(lp_pair),
            });
        }
    }

    // let lb_pairs: Vec<_> = accounts
    //     .iter()
    //     .map(|account| LbPair::deserialize(&mut &account.1.data[8..]))
    //     .filter(|x| x.is_ok())
    //     .map(|x| x.unwrap())
    //     .collect();
    Ok(market_pairs)
}

pub async fn get_raydium_amm_v4_pools(
    rpc: Arc<RpcClient>,
    mint: String,
) -> Result<Vec<MarketPair>, Box<dyn Error + Send + Sync>> {
    let mut market_pairs: Vec<MarketPair> = vec![];
    {
        let accounts = rpc
            .get_program_ui_accounts_with_config(
                &Pubkey::from_str_const(RAYDIUM_LIQUIDITY_POOL_V4),
                RpcProgramAccountsConfig {
                    filters: Some(vec![
                        RpcFilterType::Memcmp(Memcmp::new(
                            400,
                            MemcmpEncodedBytes::Base58(mint.clone()),
                        )),
                        RpcFilterType::DataSize(752),
                    ]),
                    account_config: RpcAccountInfoConfig {
                        encoding: Some(UiAccountEncoding::Base64),
                        commitment: Some(CommitmentConfig::confirmed()),
                        ..RpcAccountInfoConfig::default()
                    },
                    ..RpcProgramAccountsConfig::default()
                },
            )
            .await;

        let accounts = decode_ui_accounts(accounts.unwrap_or(vec![]));
        for account in accounts.iter() {
            if let Ok(pool) = RaydiumAMMV4::deserialize(&mut &account.1[..]) {
                market_pairs.push(MarketPair {
                    name: "Raydium AMM V4".to_string(),
                    pair: account.0.to_string(),
                    market: MarketEnum::RaydiumAMMV4(pool),
                });
            }
        }
    }

    {
        let accounts = rpc
            .get_program_ui_accounts_with_config(
                &Pubkey::from_str_const(RAYDIUM_LIQUIDITY_POOL_V4),
                RpcProgramAccountsConfig {
                    filters: Some(vec![
                        RpcFilterType::Memcmp(Memcmp::new(
                            432,
                            MemcmpEncodedBytes::Base58(mint.clone()),
                        )),
                        RpcFilterType::DataSize(752),
                    ]),
                    account_config: RpcAccountInfoConfig {
                        encoding: Some(UiAccountEncoding::Base64),
                        commitment: Some(CommitmentConfig::confirmed()),
                        ..RpcAccountInfoConfig::default()
                    },
                    ..RpcProgramAccountsConfig::default()
                },
            )
            .await;

        let accounts = decode_ui_accounts(accounts.unwrap_or(vec![]));
        for account in accounts.iter() {
            if let Ok(pool) = RaydiumAMMV4::deserialize(&mut &account.1[..]) {
                market_pairs.push(MarketPair {
                    name: "Raydium AMM V4".to_string(),
                    pair: account.0.to_string(),
                    market: MarketEnum::RaydiumAMMV4(pool),
                });
            }
        }
    }

    Ok(market_pairs)

    // let accounts = accounts.unwrap_or(vec![]);
    // let pools: Vec<_> = accounts
    //     .iter()
    //     .map(|account| AmmInfo::deserialize(&mut &account.1.data[..]))
    //     .filter(|x| x.is_ok())
    //     .map(|x| x.unwrap())
    //     .collect();
    // Ok(pools)
}

pub async fn get_pumpfun_pools(
    rpc: Arc<RpcClient>,
    mint: String,
) -> Result<Vec<MarketPair>, Box<dyn Error + Send + Sync>> {
    let mut market_pairs: Vec<MarketPair> = vec![];
    let pool_key = get_pumpfun_amm_pool(Pubkey::from_str_const(&mint));
    {
        let account_data = rpc.get_account_data(&pool_key).await;
        if let Ok(account_data) = account_data
            && let Ok(mut pool) = PumpfunAmmPool::deserialize(&mut &account_data[8..])
        {
            let bonding_curve_pda = get_bonding_curve_pda(Pubkey::from_str_const(&mint));

            let bonding_curve_account_data = rpc.get_account_data(&bonding_curve_pda).await;
            if let Ok(bonding_curve_account_data) = bonding_curve_account_data
                && let Ok(bonding_curve_account_data) =
                    PumpfunBondingCurve::deserialize(&mut &bonding_curve_account_data[8..])
            {
                pool.bonding_curve = Some(bonding_curve_account_data);
            }

            market_pairs.push(MarketPair {
                name: "Pumpfun AMM".to_string(),
                pair: pool_key.to_string(),
                market: MarketEnum::PumpfunAMM(pool),
            });
        }
    }

    Ok(market_pairs)
}

pub async fn get_raydium_clmm_pools(
    rpc: Arc<RpcClient>,
    mint: String,
) -> Result<Vec<MarketPair>, Box<dyn Error + Send + Sync>> {
    let mut market_pairs: Vec<MarketPair> = vec![];
    {
        let accounts = rpc
            .get_program_ui_accounts_with_config(
                &Pubkey::from_str_const(RAYDIUM_CLMM),
                RpcProgramAccountsConfig {
                    filters: Some(vec![
                        RpcFilterType::Memcmp(Memcmp::new(
                            105,
                            MemcmpEncodedBytes::Base58(mint.clone()),
                        )),
                        RpcFilterType::DataSize(1544),
                    ]),
                    account_config: RpcAccountInfoConfig {
                        encoding: Some(UiAccountEncoding::Base64),
                        commitment: Some(CommitmentConfig::confirmed()),
                        ..RpcAccountInfoConfig::default()
                    },
                    ..RpcProgramAccountsConfig::default()
                },
            )
            .await;
        if let Ok(accounts) = accounts {
            let accounts = decode_ui_accounts(accounts);
            for account in accounts.iter() {
                if let Ok(pool) = RaydiumCLMMPool::deserialize(&mut &account.1[8..]) {
                    market_pairs.push(MarketPair {
                        name: "Raydium CLMM".to_string(),
                        pair: account.0.to_string(),
                        market: MarketEnum::RaydiumCLMM(pool),
                    });
                }
            }
        }
    }

    Ok(market_pairs)
}

pub async fn get_raydium_clmm_configs(
    rpc: Arc<RpcClient>,
) -> Result<Vec<(Pubkey, AmmConfig)>, Box<dyn Error + Send + Sync>> {
    let mut clmm_configs: Vec<(Pubkey, AmmConfig)> = vec![];
    {
        let accounts = rpc
            .get_program_ui_accounts_with_config(
                &Pubkey::from_str_const(RAYDIUM_CLMM),
                RpcProgramAccountsConfig {
                    filters: Some(vec![RpcFilterType::DataSize(117)]),
                    account_config: RpcAccountInfoConfig {
                        encoding: Some(UiAccountEncoding::Base64),
                        commitment: Some(CommitmentConfig::confirmed()),
                        ..RpcAccountInfoConfig::default()
                    },
                    ..RpcProgramAccountsConfig::default()
                },
            )
            .await;
        if let Ok(accounts) = accounts {
            let accounts = decode_ui_accounts(accounts);
            for account in accounts.iter() {
                if let Ok(pool) = AmmConfig::deserialize(&mut &account.1[8..]) {
                    clmm_configs.push((account.0, pool));
                }
            }
        }
    }

    Ok(clmm_configs)
}

pub async fn get_pools_financials(
    rpc: Arc<RpcClient>,
    mapped_pool: GenericPoolInfo,
    base_balances: Arc<Mutex<HashMap<String, f64>>>,
    quote_balances: Arc<Mutex<HashMap<String, f64>>>,
) -> Vec<JoinHandle<()>> {
    let mut handles = vec![];
    let mapped_pool_clone = mapped_pool.clone();
    let rpc_clone = rpc.clone();
    let b = base_balances.clone();
    handles.push(tokio::task::spawn(async move {
        // Fetch tokens owned by the base mint
        let wallet_address = Pubkey::from_str_const(&mapped_pool_clone.base_vault);
        let result = rpc_clone.get_token_account_balance(&wallet_address).await;
        if let Ok(result) = result {
            b.lock().await.insert(
                mapped_pool_clone.base_vault,
                result.ui_amount.unwrap_or(0.0),
            );
        }
    }));

    let rpc_clone = rpc.clone();
    let q = quote_balances.clone();
    let mapped_pool_clone = mapped_pool.clone();
    handles.push(tokio::task::spawn(async move {
        let wallet_address = Pubkey::from_str_const(&mapped_pool.quote_vault);
        let result = rpc_clone.get_token_account_balance(&wallet_address).await;
        if let Ok(result) = result {
            q.lock().await.insert(
                mapped_pool_clone.quote_vault,
                result.ui_amount.unwrap_or(0.0),
            );
        }
    }));

    handles
}

pub async fn get_pools(
    rpc: Arc<RpcClient>,
    token: String,
) -> Result<Vec<GenericPoolInfo>, Box<dyn Error + Send + Sync>> {
    let mut handles = vec![];
    let market_pairs = Arc::new(Mutex::new(HashMap::new()));

    let cloned_rpc = rpc.clone();
    let cloned_token = token.clone();
    let cloned_market_pairs = market_pairs.clone();
    handles.push(tokio::task::spawn(async move {
        let pools = get_pumpfun_pools(cloned_rpc, cloned_token).await;
        if let Ok(pools) = pools {
            // println!("Pumpfun AMM pools: {:#?}", pools.len());
            let mut old_pools = cloned_market_pairs.lock().await;
            for pool in pools {
                old_pools.insert(pool.pair.clone(), pool);
            }
        }
    }));

    let cloned_rpc = rpc.clone();
    let cloned_token = token.clone();
    let cloned_market_pairs = market_pairs.clone();
    handles.push(tokio::task::spawn(async move {
        let pools = get_raydium_amm_v4_pools(cloned_rpc, cloned_token).await;
        if let Ok(pools) = pools {
            // println!("Raydium AMM V4 pools: {:#?}", pools.len());
            let mut old_pools = cloned_market_pairs.lock().await;
            for pool in pools {
                old_pools.insert(pool.pair.clone(), pool);
            }
        }
    }));

    let cloned_rpc = rpc.clone();
    let cloned_token = token.clone();
    let cloned_market_pairs = market_pairs.clone();
    handles.push(tokio::task::spawn(async move {
        let pools = get_raydium_clmm_pools(cloned_rpc, cloned_token).await;
        if let Ok(pools) = pools {
            // println!("Raydium CLMM pools: {:#?}", pools.len());
            let mut old_pools = cloned_market_pairs.lock().await;
            for pool in pools {
                old_pools.insert(pool.pair.clone(), pool);
            }
        }
    }));

    let cloned_rpc = rpc.clone();
    let cloned_token = token.clone();
    let cloned_market_pairs = market_pairs.clone();
    handles.push(tokio::task::spawn(async move {
        let pools = get_meteora_dynamic_amm_pools(cloned_rpc, cloned_token).await;
        if let Ok(pools) = pools {
            // println!("Meteora DAMM pools: {:#?}", pools.len());
            let mut old_pools = cloned_market_pairs.lock().await;
            for pool in pools {
                old_pools.insert(pool.pair.clone(), pool);
            }
        }
    }));

    let cloned_rpc = rpc.clone();
    let cloned_token = token.clone();
    let cloned_market_pairs = market_pairs.clone();
    handles.push(tokio::task::spawn(async move {
        let pools = get_meteora_dynamic_lmm_pools(cloned_rpc, cloned_token).await;
        if let Ok(pools) = pools {
            // println!("Meteora DLMM pools: {:#?}", pools.len());
            let mut old_pools = cloned_market_pairs.lock().await;
            for pool in pools {
                old_pools.insert(pool.pair.clone(), pool);
            }
        }
    }));

    let cloned_rpc = rpc.clone();
    let cloned_token = token.clone();
    let cloned_market_pairs = market_pairs.clone();
    handles.push(tokio::task::spawn(async move {
        let pools = get_meteora_dynamic_amm_v2_pools(cloned_rpc, cloned_token).await;
        if let Ok(pools) = pools {
            // println!("Meteora DAMMV2 pools: {:#?}", pools.len());
            let mut old_pools = cloned_market_pairs.lock().await;
            for pool in pools {
                old_pools.insert(pool.pair.clone(), pool);
            }
        }
    }));

    future::join_all(handles).await;
    let market_pairs = market_pairs.lock().await.clone();
    let market_pairs: Vec<_> = market_pairs
        .keys()
        .map(|key| MarketPair {
            pair: key.clone(),
            market: market_pairs.get(key).unwrap().clone().market,
            name: market_pairs.get(key).unwrap().clone().name,
        })
        .collect();

    let clmm_configs = get_raydium_clmm_configs(rpc.clone())
        .await
        .unwrap_or(vec![]);

    let mut mapped_pools: Vec<GenericPoolInfo> = vec![];
    for market_pair in &market_pairs {
        let mut mapped_pool = market_pair.clone().generic();
        if let MarketEnum::RaydiumCLMM(raydium_clmm_pool) = &market_pair.market {
            let found = clmm_configs
                .iter()
                .find(|x| x.0.eq(&raydium_clmm_pool.amm_config));
            if let Some(found) = found {
                mapped_pool.fees = found.1.trade_fee_rate as f64 / 10000.0;
            }
        }
        if mapped_pool.quote_mint == WSOL || mapped_pool.quote_mint == USDC {
            mapped_pools.push(mapped_pool);
        }
    }

    let mut handles: Vec<JoinHandle<()>> = vec![];
    let base_balances: Arc<Mutex<HashMap<String, f64>>> = Arc::new(Mutex::new(HashMap::new()));
    let quote_balances: Arc<Mutex<HashMap<String, f64>>> = Arc::new(Mutex::new(HashMap::new()));
    for mapped_pool in mapped_pools.clone() {
        let subhandles = get_pools_financials(
            rpc.clone(),
            mapped_pool.clone(),
            base_balances.clone(),
            quote_balances.clone(),
        )
        .await;
        for handle in subhandles {
            handles.push(handle);
        }
    }

    future::join_all(handles).await;

    let base_balances = base_balances.lock().await.clone();
    let quote_balances = quote_balances.lock().await.clone();
    for mapped_pool in &mut mapped_pools {
        mapped_pool.base_balance = if let Some(balance) = base_balances.get(&mapped_pool.base_vault)
        {
            *balance
        } else {
            0.0
        };

        mapped_pool.quote_balance =
            if let Some(balance) = quote_balances.get(&mapped_pool.quote_vault) {
                *balance
            } else {
                0.0
            };
    }
    let mut mapped_pools: Vec<_> = mapped_pools
        .iter()
        .filter(|mapped_pool| mapped_pool.base_balance > 0.0 && mapped_pool.quote_balance > 0.0)
        .cloned()
        .collect();
    mapped_pools.sort_by(|a, b| b.base_balance.partial_cmp(&a.base_balance).unwrap());
    Ok(mapped_pools)
}

pub async fn get_pool(
    rpc: Arc<RpcClient>,
    pool_addres: String,
) -> Result<Option<GenericPoolInfo>, Box<dyn Error + Send + Sync>> {
    let market_pair = get_market_account_raw(rpc.clone(), pool_addres).await;
    if let Some(market_pair) = market_pair {
        let mut handles: Vec<JoinHandle<()>> = vec![];
        let base_balances: Arc<Mutex<HashMap<String, f64>>> = Arc::new(Mutex::new(HashMap::new()));
        let quote_balances: Arc<Mutex<HashMap<String, f64>>> = Arc::new(Mutex::new(HashMap::new()));
        let mut mapped_pool = market_pair.clone().generic();
        let subhandles = get_pools_financials(
            rpc.clone(),
            mapped_pool.clone(),
            base_balances.clone(),
            quote_balances.clone(),
        )
        .await;
        for handle in subhandles {
            handles.push(handle);
        }

        future::join_all(handles).await;

        // let mut mapped_pools = mapped_pools;
        let base_balances = base_balances.lock().await.clone();
        let quote_balances = quote_balances.lock().await.clone();

        mapped_pool.base_balance = if let Some(balance) = base_balances.get(&mapped_pool.base_vault)
        {
            *balance
        } else {
            0.0
        };

        mapped_pool.quote_balance =
            if let Some(balance) = quote_balances.get(&mapped_pool.quote_vault) {
                *balance
            } else {
                0.0
            };

        return Ok(Some(mapped_pool));
    }

    Ok(None)
}

pub async fn get_pool_financials(
    rpc: Arc<RpcClient>,
    market_pair: MarketPair,
) -> Result<GenericPoolInfo, Box<dyn Error + Send + Sync>> {
    let mut handles: Vec<JoinHandle<()>> = vec![];
    let base_balances: Arc<Mutex<HashMap<String, f64>>> = Arc::new(Mutex::new(HashMap::new()));
    let quote_balances: Arc<Mutex<HashMap<String, f64>>> = Arc::new(Mutex::new(HashMap::new()));
    let mut mapped_pool = market_pair.clone().generic();
    let subhandles = get_pools_financials(
        rpc.clone(),
        mapped_pool.clone(),
        base_balances.clone(),
        quote_balances.clone(),
    )
    .await;
    for handle in subhandles {
        handles.push(handle);
    }

    future::join_all(handles).await;

    // let mut mapped_pools = mapped_pools;
    let base_balances = base_balances.lock().await.clone();
    let quote_balances = quote_balances.lock().await.clone();

    mapped_pool.base_balance = if let Some(balance) = base_balances.get(&mapped_pool.base_vault) {
        *balance
    } else {
        0.0
    };

    mapped_pool.quote_balance = if let Some(balance) = quote_balances.get(&mapped_pool.quote_vault)
    {
        *balance
    } else {
        0.0
    };

    Ok(mapped_pool)
}

pub async fn get_market_account_raw(
    rpc: Arc<RpcClient>,
    pool_addres: String,
) -> Option<MarketPair> {
    let account = rpc
        .get_account_data(&Pubkey::from_str_const(&pool_addres))
        .await;
    let account = match account {
        Ok(account) => account,
        Err(_) => return None,
    };
    let mut market_pair: Option<MarketPair> = None;
    if account.len() == 904 && account[0..8].eq(&vec![33, 11, 49, 98, 181, 101, 177, 13]) {
        if let Ok(lp_pair) = MeteoraDLMMPool::deserialize(&mut &account[8..]) {
            market_pair = Some(MarketPair {
                name: "Meteora DLMM".to_string(),
                pair: pool_addres.clone(),
                market: MarketEnum::MeteoraDLMM(lp_pair),
            });
        }
    } else if account.len() == 1112 && account[0..8].eq(&vec![241, 154, 109, 4, 17, 177, 109, 188])
    {
        if let Ok(pool) = MeteoraDAMMV2Pool::deserialize(&mut &account[8..]) {
            market_pair = Some(MarketPair {
                name: "Meteora DAMMV2".to_string(),
                pair: pool_addres.clone(),
                market: MarketEnum::MeteoraDAMMV2(pool),
            });
        }
    } else if account.len() == 944 && account[0..8].eq(&vec![241, 154, 109, 4, 17, 177, 109, 188]) {
        if let Ok(pool) = MeteoraDAMMPool::deserialize(&mut &account[8..]) {
            market_pair = Some(MarketPair {
                name: "Meteora DAMM".to_string(),
                pair: pool_addres.clone(),
                market: MarketEnum::MeteoraDAMM(pool),
            });
        }
    } else if account.len() == 952 && account[0..8].eq(&vec![241, 154, 109, 4, 17, 177, 109, 188]) {
        if let Ok(pool) = MeteoraDAMMPool::deserialize(&mut &account[8..]) {
            market_pair = Some(MarketPair {
                name: "Meteora DAMM".to_string(),
                pair: pool_addres.clone(),
                market: MarketEnum::MeteoraDAMM(pool),
            });
        }
    } else if account.len() == 752 && account[0..8].eq(&vec![6, 0, 0, 0, 0, 0, 0, 0]) {
        if let Ok(pool) = RaydiumAMMV4::deserialize(&mut &account[..]) {
            market_pair = Some(MarketPair {
                name: "Raydium AMM V4".to_string(),
                pair: pool_addres.clone(),
                market: MarketEnum::RaydiumAMMV4(pool),
            });
        }
    } else if account.len() == 1544
        && account[0..8].eq(&vec![247, 237, 227, 245, 215, 195, 222, 70])
    {
        if let Ok(pool) = RaydiumCLMMPool::deserialize(&mut &account[8..]) {
            market_pair = Some(MarketPair {
                name: "Raydium CLMM".to_string(),
                pair: pool_addres.clone(),
                market: MarketEnum::RaydiumCLMM(pool),
            });
        }
    } else if (account.len() == 300 || account.len() == 301)
        && account[0..8].eq(&vec![241, 154, 109, 4, 17, 177, 109, 188])
        && let Ok(mut pool) = PumpfunAmmPool::deserialize(&mut &account[8..])
    {
        let bonding_curve_pda = get_bonding_curve_pda(pool.base_mint);

        let bonding_curve_account_data = rpc.get_account_data(&bonding_curve_pda).await;
        if let Ok(bonding_curve_account_data) = bonding_curve_account_data
            && let Ok(bonding_curve_account_data) =
                PumpfunBondingCurve::deserialize(&mut &bonding_curve_account_data[8..])
        {
            pool.bonding_curve = Some(bonding_curve_account_data);
        }

        market_pair = Some(MarketPair {
            name: "Pumpfun AMM".to_string(),
            pair: pool_addres.to_string(),
            market: MarketEnum::PumpfunAMM(pool),
        });
    } else {
        tracing::warn!(pool_addres = %pool_addres, "get_market_account_raw: {} - {:#?}", account.len(), &account[0..8]);
    }

    market_pair
}
