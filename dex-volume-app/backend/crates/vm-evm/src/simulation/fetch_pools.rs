use std::{collections::HashMap, error::Error, str::FromStr};

use alloy::{
    network::TransactionBuilder,
    primitives::{Address, Bytes, U256, address},
    providers::Provider,
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
    sol_types::SolCall,
};

use crate::{
    constants::{MAX_UINT_HEX, Network},
    contracts::{
        FindPoolsContract::{self},
        UniswapV4PositionManager::{self, getPoolAndPositionInfoReturn},
    },
    models::{
        simulation::{
            EthCallManyPayloadParamsAccountOverride, EthCallManyPayloadParamsAccountOverrideState,
        },
        token::{CustomPool, CustomToken},
    },
    simulation::erigon::trace_txs,
};

use vm_data::db::Database;

pub async fn get_pools(
    db: &Database,
    network: Network,
    token_address: Address,
) -> Result<CustomToken, Box<dyn Error + Send + Sync>> {
    let provider = network.get_provider();
    let gas_price = provider.estimate_eip1559_fees().await?;
    let max_fee_per_gas = gas_price.max_fee_per_gas;
    let max_priority_fee_per_gas = gas_price.max_priority_fee_per_gas;
    let mut _account_overrides: EthCallManyPayloadParamsAccountOverride = HashMap::new();
    let mut txs: Vec<TransactionRequest> = vec![];

    let finder_contract_address;
    let addresses;

    match network.clone() {
        Network::Ethereum => {
            finder_contract_address =
                Address::from_str("0x723f603332A3Ed3e1fD565BC6E2046fcF43b368A")?;
            addresses = vec![
                address!("c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"),
                address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
                address!("dAC17F958D2ee523a2206206994597C13D831ec7"),
                address!("6982508145454ce325ddbe47a25d4ec3d2311933"),
                address!("95aD61b0a150d79219dCF64E1E6Cc01f0B64C4cE"),
            ];
        }
        Network::Base => {
            finder_contract_address =
                Address::from_str("0xB8F0AF63a6a6054445B576D0Baf616DeB316423e")?;
            addresses = vec![
                address!("4200000000000000000000000000000000000006"),
                address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
                address!("50c5725949A6F0c72E6C4a641F24049A917DB0Cb"),
                address!("fde4C96c8593536E31F229EA8f37b2ADa2699bb2"),
                address!("0b3e328455c4059EEb9e3f84b5543F74E24e7E1b"),
            ];
        }
        _ => return Err(format!("Pool finder not configured for {:?}", network).into()),
    }

    let contract = FindPoolsContract::new(finder_contract_address, provider.clone());
    let res = contract.findPools(token_address, addresses);

    let to = *contract.address();
    let input = res.calldata().clone();
    let signer = PrivateKeySigner::random();
    _account_overrides.insert(
        signer.address().to_string(),
        EthCallManyPayloadParamsAccountOverrideState {
            balance: MAX_UINT_HEX.to_string(),
        },
    );
    let from = signer.address();
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

    let response = trace_txs(network.clone(), txs, None).await?;
    let data = Bytes::from_str(&response.result[0].output)?;
    let decoded = FindPoolsContract::findPoolsCall::abi_decode_returns(&data[..])?;

    let additional_pools = db
        .get_latest_uniswap_v4_pools_by_address(network.clone() as i32, token_address.to_string())
        .await
        .unwrap_or_default();

    let mut pools: Vec<CustomPool> = vec![];
    for pool in decoded.pools {
        pools.push(CustomPool {
            pool: pool.pool.to_string(),
            pair: pool.pair,
            version: pool.version,
            fee: pool.fee,
            balance: pool.balance,
            tokens: pool.tokens,
            token_id: None,
            tick_spacing: None,
        });
    }

    for additional_pool in additional_pools {
        pools.push(CustomPool {
            pool: additional_pool.pool_key,
            pair: if additional_pool.currency0.eq(&token_address.to_string()) {
                Address::from_str(&additional_pool.currency1)?
            } else {
                Address::from_str(&additional_pool.currency0)?
            },
            version: 4,
            fee: additional_pool.fee.parse::<u32>().unwrap_or(0),
            balance: U256::from(0),
            tokens: decoded.totalSupply,
            token_id: Some(additional_pool.token_id.to_string()),
            tick_spacing: Some(additional_pool.tick_spacing.to_string()),
        });
    }

    let token = CustomToken {
        name: decoded.name,
        symbol: decoded.symbol,
        token: decoded.token,
        owner: decoded.owner,
        decimals: decoded.decimals,
        total_supply: decoded.totalSupply,
        remaining_supply: decoded.remainingSupply,
        burned_supply: decoded.burnedSupply,
        pools,
    };

    Ok(token)
}

pub async fn get_uniswap_v4_pools_next_id(
    network: Network,
) -> Result<U256, Box<dyn Error + Send + Sync>> {
    let provider = network.get_provider();
    let position_manager_address = match network.clone() {
        Network::Ethereum => address!("bd216513d74c8cf14cf4747e6aaa6420ff64ee9e"),
        Network::Base => address!("7c5f5a4bbd8fd63184577525326123b519429bdc"),
        _ => return Err(format!("V4 position manager not configured for {:?}", network).into()),
    };

    let contract = UniswapV4PositionManager::new(position_manager_address, provider.clone());
    let next_token_id = contract.nextTokenId().call().await?;
    Ok(next_token_id)
}

pub async fn get_v4_pools(
    network: Network,
    start_idx: i32,
    end_idx: i32,
) -> Result<Vec<getPoolAndPositionInfoReturn>, Box<dyn Error + Send + Sync>> {
    let provider = network.get_provider();
    let gas_price = provider.estimate_eip1559_fees().await?;
    let max_fee_per_gas = gas_price.max_fee_per_gas;
    let max_priority_fee_per_gas = gas_price.max_priority_fee_per_gas;
    let mut _account_overrides: EthCallManyPayloadParamsAccountOverride = HashMap::new();
    let mut positions_infos = vec![];
    let mut txs: Vec<TransactionRequest> = vec![];

    let position_manager_address = match network.clone() {
        Network::Ethereum => Address::from_str("0xbd216513d74c8cf14cf4747e6aaa6420ff64ee9e")?,
        Network::Base => Address::from_str("0x7c5f5a4bbd8fd63184577525326123b519429bdc")?,
        _ => return Err(format!("V4 position manager not configured for {:?}", network).into()),
    };

    let contract = UniswapV4PositionManager::new(position_manager_address, provider.clone());

    for i in start_idx..=end_idx {
        let res = contract.getPoolAndPositionInfo(U256::from(i));
        let to = *contract.address();
        let input = res.calldata().clone();
        let signer = PrivateKeySigner::random();
        _account_overrides.insert(
            signer.address().to_string(),
            EthCallManyPayloadParamsAccountOverrideState {
                balance: MAX_UINT_HEX.to_string(),
            },
        );
        let from = signer.address();
        let gas_limit = 1_000_000;

        let tx_request = TransactionRequest::default()
            .with_from(from)
            .with_to(to)
            .with_value(U256::from(0))
            .with_input(input)
            .with_gas_limit(gas_limit.try_into()?)
            .with_max_fee_per_gas(max_fee_per_gas)
            .with_max_priority_fee_per_gas(max_priority_fee_per_gas)
            .with_nonce(0)
            .with_chain_id(network.clone() as u64);
        txs.push(tx_request);
    }

    let response = trace_txs(network, txs, None).await?;
    for res in &response.result {
        let data = Bytes::from_str(&res.output)?;
        let decoded =
            UniswapV4PositionManager::getPoolAndPositionInfoCall::abi_decode_returns(&data[..])?;
        positions_infos.push(decoded);
    }
    Ok(positions_infos)
}
