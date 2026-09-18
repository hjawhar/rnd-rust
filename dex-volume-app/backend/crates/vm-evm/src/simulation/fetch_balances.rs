use std::{collections::HashMap, error::Error, str::FromStr};

use alloy::{
    dyn_abi::DynSolValue,
    network::TransactionBuilder,
    primitives::{Address, I256, U256, keccak256},
    providers::Provider,
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
};

use crate::{
    constants::{MAX_UINT_HEX, Network},
    contracts::{BalanceChecker, ERC20Contract},
    helpers::split_array_ranges,
    models::simulation::{
        EthCallManyPayloadParamsAccountOverride, EthCallManyPayloadParamsAccountOverrideState,
    },
    simulation::{
        erigon::trace_txs,
        fetch_pools::{get_uniswap_v4_pools_next_id, get_v4_pools},
    },
};

use vm_data::db::Database;
use vm_data::models::uniswap::NewUniswapV4Pool;

pub async fn get_eth_balances(
    network: Network,
    addresses: Vec<Address>,
) -> Result<Vec<U256>, Box<dyn Error + Send + Sync>> {
    let provider = network.get_provider();
    let gas_price = provider.estimate_eip1559_fees().await?;
    let max_fee_per_gas = gas_price.max_fee_per_gas;
    let max_priority_fee_per_gas = gas_price.max_priority_fee_per_gas;
    let mut _account_overrides: EthCallManyPayloadParamsAccountOverride = HashMap::new();
    let mut txs: Vec<TransactionRequest> = vec![];

    let mut balances = vec![];
    let address = match network {
        Network::Ethereum => Address::from_str("0x7770BaD1d7F58D5e6EB1E84A61C4676AaF99f7Cf")?,
        Network::Base => Address::from_str("0x11f7c74830FC96e18B1b0722B80026D9CE37A5E2")?,
        _ => return Err(format!("Balance checker not configured for {:?}", network).into()),
    };

    let contract = BalanceChecker::new(address, provider.clone());

    for i in 0..addresses.len() {
        let signer = PrivateKeySigner::random();
        _account_overrides.insert(
            signer.address().to_string(),
            EthCallManyPayloadParamsAccountOverrideState {
                balance: MAX_UINT_HEX.to_string(),
            },
        );
        let result = contract.getBalance(addresses[i]);
        let from = signer.address();
        let to = *contract.address();
        let input = result.calldata().clone();
        let gas_limit = 1_000_000;
        let value = U256::from(0);

        let tx_request = TransactionRequest::default()
            .with_from(from)
            .with_to(to)
            .with_value(value)
            .with_input(input)
            .with_gas_limit(gas_limit.try_into()?)
            .with_max_fee_per_gas(max_fee_per_gas)
            .with_max_priority_fee_per_gas(max_priority_fee_per_gas)
            .with_nonce(0)
            .with_chain_id(network.clone() as u64);
        txs.push(tx_request);
    }

    let response = trace_txs(network, txs, None).await?;

    for i in 0..response.result.len() {
        let b_str = &response.result[i].output;
        let before = U256::from_str(b_str);
        if let Ok(before) = before {
            balances.push(before);
        } else {
            balances.push(U256::from(0));
        }
    }

    Ok(balances)
}

pub async fn get_tokens_balances(
    network: Network,
    contract_address: Address,
    addresses: Vec<Address>,
) -> Result<Vec<U256>, Box<dyn Error + Send + Sync>> {
    let provider = network.get_provider();
    let gas_price = provider.estimate_eip1559_fees().await?;
    let max_fee_per_gas = gas_price.max_fee_per_gas;
    let max_priority_fee_per_gas = gas_price.max_priority_fee_per_gas;
    let mut _account_overrides: EthCallManyPayloadParamsAccountOverride = HashMap::new();

    let mut txs: Vec<TransactionRequest> = vec![];
    let mut balances = vec![];

    for address in &addresses {
        let signer = PrivateKeySigner::random();
        _account_overrides.insert(
            signer.address().to_string(),
            EthCallManyPayloadParamsAccountOverrideState {
                balance: MAX_UINT_HEX.to_string(),
            },
        );

        let from = signer.address();
        let to = contract_address;
        let erc20contract = ERC20Contract::new(contract_address, provider.clone());
        let balance_res = erc20contract.balanceOf(*address);
        let input = balance_res.calldata().clone();
        let gas_limit = 100_000;
        let value = U256::from(0);

        let tx_request = TransactionRequest::default()
            .with_from(from)
            .with_to(to)
            .with_value(value)
            .with_input(input)
            .with_gas_limit(gas_limit.try_into()?)
            .with_max_fee_per_gas(max_fee_per_gas)
            .with_max_priority_fee_per_gas(max_priority_fee_per_gas)
            .with_nonce(0)
            .with_chain_id(network.clone() as u64);
        txs.push(tx_request);
    }

    let response = trace_txs(network, txs, None).await?;

    for i in 0..response.result.len() {
        let b_str = &response.result[i].output;
        let before = U256::from_str(b_str);
        if let Ok(before) = before {
            balances.push(before);
        } else {
            balances.push(U256::from(0));
        }
    }

    Ok(balances)
}

pub async fn populate_uniswap_v4_pools(
    db: &Database,
    network: Network,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut start_idx = 1u64;
    let next_id = get_uniswap_v4_pools_next_id(network.clone()).await.map_err(|e| {
        tracing::error!("[V4 POOLS] Failed to get next pool ID on {network:#?}: {e}");
        e
    })?;
    let end_idx = next_id.to::<u64>();
    let mut total_fetched = 0;

    let latest_synced = db
        .get_latest_uniswap_v4_pool_by_network(network.clone() as i32)
        .await;

    if let Ok(Some(latest_synced)) = latest_synced {
        start_idx = latest_synced.token_id as u64 + 1;
    }

    let split = split_array_ranges(start_idx, end_idx, 10000);
    for s in split {
        tracing::debug!("Fetching V4 pools on {network:#?} from {} to {}", s.0, s.1);

        let v4_pools = get_v4_pools(network.clone(), s.0 as i32, s.1 as i32).await.map_err(|e| {
            tracing::error!("[V4 POOLS] Failed to fetch pools on {network:#?} range {}..{}: {e}", s.0, s.1);
            e
        })?;

        for pool_info in v4_pools {
            let message_value = DynSolValue::Tuple(vec![
                DynSolValue::Address(pool_info.poolKey.currency0),
                DynSolValue::Address(pool_info.poolKey.currency1),
                DynSolValue::Uint(U256::from(pool_info.poolKey.fee), 24),
                DynSolValue::Int(I256::from(pool_info.poolKey.tickSpacing), 24),
                DynSolValue::Address(pool_info.poolKey.hooks),
            ]);
            let encoded_message = message_value.abi_encode();
            let message_hash = keccak256(&encoded_message);

            let new_pool = NewUniswapV4Pool {
                network_id: network.clone() as i32,
                token_id: start_idx as i64,
                pool_key: message_hash.to_string(),
                currency0: pool_info.poolKey.currency0.to_string(),
                currency1: pool_info.poolKey.currency1.to_string(),
                tick_spacing: pool_info.poolKey.tickSpacing.to_string(),
                fee: pool_info.poolKey.fee.to_string(),
                hooks: pool_info.poolKey.hooks.to_string(),
            };

            let _ = db.add_v4_pool(&new_pool).await;
            start_idx += 1;
            total_fetched += 1;
        }
        tracing::debug!("Fetched {total_fetched} V4 pools on {network:#?} so far");
    }

    tracing::debug!("Fetched a total of {total_fetched} V4 pools on {network:#?}");
    Ok(())
}
