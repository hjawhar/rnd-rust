use alloy::{
    network::{Ethereum, EthereumWallet, TransactionBuilder},
    primitives::{Address, Bytes, FixedBytes, U160, U256, aliases::U48},
    providers::{PendingTransactionBuilder, Provider},
    rpc::types::TransactionRequest,
    signers::{
        k256::{self},
        local::LocalSigner,
    },
};

use crate::{
    constants::{MAX_UINT_HEX, Network},
    contracts::{
        ERC20Contract, BaseUniswapV2Router, Permit2, UniswapV4PositionManager,
        UniswapV4UniversalRouter,
    },
    helpers::{str_to_pk, u256_to_f64},
    models::token::{CustomPool, CustomToken},
};
use vm_data::utils::helpers::f64_to_big_int;

use vm_data::models::{
    streams::{StreamInfo, StreamType},
    transaction::NewTransaction,
};

use std::{
    error::Error,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use std::{str::FromStr, time::Duration};
use tokio::sync::mpsc::Sender;

pub struct TxWatcherInfo {
    pub project_id: i32,
    pub wallet_id: i32,
    pub user_id: i32,
    pub eth_price: f64,
    pub trade_volume_usdc: f64,
}

impl Clone for TxWatcherInfo {
    fn clone(&self) -> Self {
        Self {
            project_id: self.project_id,
            wallet_id: self.wallet_id,
            user_id: self.user_id,
            eth_price: self.eth_price,
            trade_volume_usdc: self.trade_volume_usdc,
        }
    }
}

pub struct BuildRequests {
    pub from: Address,
    pub to: Address,
    pub gas_limit: i32,
    pub input: Bytes,
    pub value: U256,
    pub nonce: u64,
    pub signer: LocalSigner<k256::ecdsa::SigningKey>,
    pub tx_type: String,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub address: Address,
    pub decimals: u8,
}

#[derive(Debug, Clone)]
pub struct PoolKey {
    pub currency0: Address,
    pub currency1: Address,
    pub fee: u32,
    pub tick_spacing: i32,
    pub hooks: Address,
}

#[derive(Debug, Clone)]
pub struct PathKey {
    pub intermediate_currency: Address,
    pub fee: u32,
    pub tick_spacing: i32,
    pub hooks: Address,
    pub hook_data: Bytes,
}

#[derive(Debug, Clone)]
pub struct SwapExactIn {
    pub currency_in: Address,
    pub path: Vec<PathKey>,
    pub amount_in: U256,
    pub amount_out_minimum: U256,
}

#[derive(Debug, Clone)]
pub struct SwapExactOut {
    pub currency_out: Address,
    pub path: Vec<PathKey>,
    pub amount_out: U256,
    pub amount_in_maximum: U256,
}

pub fn encode_multihop_exact_in_path(pool_keys: &[PoolKey], currency_in: Address) -> Vec<PathKey> {
    let mut path_keys = Vec::new();
    let mut current_currency_in = currency_in;

    for pool_key in pool_keys {
        let currency_out = if current_currency_in == pool_key.currency0 {
            pool_key.currency1
        } else {
            pool_key.currency0
        };

        let path_key = PathKey {
            intermediate_currency: currency_out,
            fee: pool_key.fee,
            tick_spacing: pool_key.tick_spacing,
            hooks: pool_key.hooks,
            hook_data: Bytes::from(vec![]),
        };

        path_keys.push(path_key);
        current_currency_in = currency_out;
    }

    path_keys
}

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum V4Action {
    SwapExactIn = 0x07,
    SwapExactOut = 0x09,
    SettleAll = 0x0c,
    TakeAll = 0x0f,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum CommandType {
    Sweep = 0x04,
    V4Swap = 0x10,
}

#[derive(Debug, Clone)]
pub struct V4Planner {
    pub actions: Vec<u8>,
    pub params: Vec<Bytes>,
}

impl Default for V4Planner {
    fn default() -> Self {
        Self::new()
    }
}

impl V4Planner {
    pub fn new() -> Self {
        Self {
            actions: Vec::new(),
            params: Vec::new(),
        }
    }

    pub fn add_action(&mut self, action: V4Action, params: Bytes) {
        self.actions.push(action as u8);
        self.params.push(params);
    }

    pub fn finalize(&self) -> Bytes {
        let actions_bytes = Bytes::from(self.actions.clone());
        let encoded = alloy::sol_types::SolValue::abi_encode_params(&(
            actions_bytes.clone(),
            self.params.clone(),
        ));
        Bytes::from(encoded)
    }
}

#[derive(Debug, Clone)]
pub struct RoutePlanner {
    pub commands: Vec<u8>,
    pub inputs: Vec<Bytes>,
}

impl Default for RoutePlanner {
    fn default() -> Self {
        Self::new()
    }
}

impl RoutePlanner {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
            inputs: Vec::new(),
        }
    }

    pub fn add_command(&mut self, command: CommandType, input: Bytes) {
        self.commands.push(command as u8);
        self.inputs.push(input);
    }
}

pub fn encode_swap_exact_in(config: &SwapExactIn) -> Bytes {
    let mut encoded = Vec::new();

    let initial_offset = U256::from(0x20);
    encoded.extend_from_slice(&initial_offset.to_be_bytes::<32>());

    let mut currency_in_bytes = vec![0u8; 12];
    currency_in_bytes.extend_from_slice(config.currency_in.as_slice());
    encoded.extend_from_slice(&currency_in_bytes);

    let path_offset = U256::from(0x80);
    encoded.extend_from_slice(&path_offset.to_be_bytes::<32>());

    encoded.extend_from_slice(&config.amount_in.to_be_bytes::<32>());
    encoded.extend_from_slice(&config.amount_out_minimum.to_be_bytes::<32>());

    let path_length = U256::from(config.path.len());
    encoded.extend_from_slice(&path_length.to_be_bytes::<32>());

    let mut path_data = Vec::new();
    let mut path_offsets = Vec::new();
    let mut current_offset = 32 * config.path.len();

    for path_key in &config.path {
        path_offsets.push(current_offset);
        let mut path_key_data = Vec::new();

        let mut intermediate_currency_bytes = vec![0u8; 12];
        intermediate_currency_bytes.extend_from_slice(path_key.intermediate_currency.as_slice());
        path_key_data.extend_from_slice(&intermediate_currency_bytes);

        let fee = U256::from(path_key.fee);
        path_key_data.extend_from_slice(&fee.to_be_bytes::<32>());

        let tick_spacing = if path_key.tick_spacing < 0 {
            let abs_val = U256::from((-path_key.tick_spacing) as u64);
            U256::MAX - abs_val + U256::from(1)
        } else {
            U256::from(path_key.tick_spacing as u64)
        };
        path_key_data.extend_from_slice(&tick_spacing.to_be_bytes::<32>());

        let mut hooks_bytes = vec![0u8; 12];
        hooks_bytes.extend_from_slice(path_key.hooks.as_slice());
        path_key_data.extend_from_slice(&hooks_bytes);

        let hook_data_offset = U256::from(0xa0);
        path_key_data.extend_from_slice(&hook_data_offset.to_be_bytes::<32>());

        let hook_data_len = U256::from(path_key.hook_data.len());
        path_key_data.extend_from_slice(&hook_data_len.to_be_bytes::<32>());

        if !path_key.hook_data.is_empty() {
            path_key_data.extend_from_slice(&path_key.hook_data);
            let padding = (32 - (path_key.hook_data.len() % 32)) % 32;
            path_key_data.extend_from_slice(&vec![0u8; padding]);
        }

        current_offset += path_key_data.len();
        path_data.push(path_key_data);
    }

    for offset in path_offsets {
        let offset_u256 = U256::from(offset);
        encoded.extend_from_slice(&offset_u256.to_be_bytes::<32>());
    }

    for data in path_data {
        encoded.extend_from_slice(&data);
    }

    Bytes::from(encoded)
}

/// Encode SwapExactOut params — same ABI layout as SwapExactIn
/// (address, PathKey[], uint128, uint128) but with output semantics.
/// `currency_out` = what you want to receive, path leads back to what you pay.
pub fn encode_swap_exact_out(config: &SwapExactOut) -> Bytes {
    let mut encoded = Vec::new();

    let initial_offset = U256::from(0x20);
    encoded.extend_from_slice(&initial_offset.to_be_bytes::<32>());

    let mut currency_out_bytes = vec![0u8; 12];
    currency_out_bytes.extend_from_slice(config.currency_out.as_slice());
    encoded.extend_from_slice(&currency_out_bytes);

    let path_offset = U256::from(0x80);
    encoded.extend_from_slice(&path_offset.to_be_bytes::<32>());

    encoded.extend_from_slice(&config.amount_out.to_be_bytes::<32>());
    encoded.extend_from_slice(&config.amount_in_maximum.to_be_bytes::<32>());

    let path_length = U256::from(config.path.len());
    encoded.extend_from_slice(&path_length.to_be_bytes::<32>());

    let mut path_data = Vec::new();
    let mut path_offsets = Vec::new();
    let mut current_offset = 32 * config.path.len();

    for path_key in &config.path {
        path_offsets.push(current_offset);
        let mut path_key_data = Vec::new();

        let mut intermediate_currency_bytes = vec![0u8; 12];
        intermediate_currency_bytes.extend_from_slice(path_key.intermediate_currency.as_slice());
        path_key_data.extend_from_slice(&intermediate_currency_bytes);

        let fee = U256::from(path_key.fee);
        path_key_data.extend_from_slice(&fee.to_be_bytes::<32>());

        let tick_spacing = if path_key.tick_spacing < 0 {
            let abs_val = U256::from((-path_key.tick_spacing) as u64);
            U256::MAX - abs_val + U256::from(1)
        } else {
            U256::from(path_key.tick_spacing as u64)
        };
        path_key_data.extend_from_slice(&tick_spacing.to_be_bytes::<32>());

        let mut hooks_bytes = vec![0u8; 12];
        hooks_bytes.extend_from_slice(path_key.hooks.as_slice());
        path_key_data.extend_from_slice(&hooks_bytes);

        let hook_data_offset = U256::from(0xa0);
        path_key_data.extend_from_slice(&hook_data_offset.to_be_bytes::<32>());

        let hook_data_len = U256::from(path_key.hook_data.len());
        path_key_data.extend_from_slice(&hook_data_len.to_be_bytes::<32>());

        if !path_key.hook_data.is_empty() {
            path_key_data.extend_from_slice(&path_key.hook_data);
            let padding = (32 - (path_key.hook_data.len() % 32)) % 32;
            path_key_data.extend_from_slice(&vec![0u8; padding]);
        }

        current_offset += path_key_data.len();
        path_data.push(path_key_data);
    }

    for offset in path_offsets {
        let offset_u256 = U256::from(offset);
        encoded.extend_from_slice(&offset_u256.to_be_bytes::<32>());
    }

    for data in path_data {
        encoded.extend_from_slice(&data);
    }

    Bytes::from(encoded)
}

pub fn encode_settle_all(currency: Address, amount: U256) -> Bytes {
    let mut encoded = Vec::new();
    let mut currency_bytes = vec![0u8; 12];
    currency_bytes.extend_from_slice(currency.as_slice());
    encoded.extend_from_slice(&currency_bytes);
    encoded.extend_from_slice(&amount.to_be_bytes::<32>());
    Bytes::from(encoded)
}

pub fn encode_take_all(currency: Address, min_amount: U256) -> Bytes {
    let mut encoded = Vec::new();
    let mut currency_bytes = vec![0u8; 12];
    currency_bytes.extend_from_slice(currency.as_slice());
    encoded.extend_from_slice(&currency_bytes);
    encoded.extend_from_slice(&min_amount.to_be_bytes::<32>());
    Bytes::from(encoded)
}

/// Encode SWEEP command input: (address token, address recipient, uint256 amountMin)
/// Sends remaining balance of `token` held by the router to `recipient`.
/// Use token = address(0) for native ETH.
pub fn encode_sweep(token: Address, recipient: Address, amount_min: U256) -> Bytes {
    let mut encoded = Vec::new();
    let mut token_bytes = vec![0u8; 12];
    token_bytes.extend_from_slice(token.as_slice());
    encoded.extend_from_slice(&token_bytes);
    let mut recipient_bytes = vec![0u8; 12];
    recipient_bytes.extend_from_slice(recipient.as_slice());
    encoded.extend_from_slice(&recipient_bytes);
    encoded.extend_from_slice(&amount_min.to_be_bytes::<32>());
    Bytes::from(encoded)
}

pub async fn exec_transaction(
    tx_mpsc: Arc<Sender<StreamInfo>>,
    network: Network,
    token: CustomToken,
    pool: CustomPool,
    buy: bool,
    sender: String,
    amount_in: U256,
    amount_out_minimum: U256,
    tx_watcher_info: TxWatcherInfo,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let token_in: Address;
    let token_out: Address;

    let mut build_requests: Vec<BuildRequests> = vec![];

    let provider = network.get_provider();

    let signer = str_to_pk(&sender)?;

    let deadline = U256::from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_secs()
            + 100,
    );

    // Parallel nonce + gas fetch (saves ~50ms per trade)
    let is_avalanche = network.clone() as i32 == Network::Avalanche as i32;
    let (nonce_result, gas_result) = tokio::join!(
        provider.get_transaction_count(signer.address()),
        async {
            if is_avalanche {
                provider
                    .get_gas_price()
                    .await
                    .map(|p| (p, p))
            } else {
                provider
                    .estimate_eip1559_fees()
                    .await
                    .map(|f| (f.max_fee_per_gas, f.max_priority_fee_per_gas))
            }
        }
    );
    let mut nonce = match nonce_result {
        Ok(n) => n,
        Err(e) => {
            tracing::error!("[EVM TX] Failed to get nonce for {}: {}", signer.address(), e);
            return Err(e.into());
        }
    };
    let (max_fee_per_gas, max_priority_fee_per_gas) = match gas_result {
        Ok(g) => g,
        Err(e) => {
            tracing::error!("[EVM TX] Failed to get gas price: {}", e);
            return Err(e.into());
        }
    };

    let uniswap_router_v2_address: Address = match network {
        Network::Ethereum => Address::from_str("0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D")?,
        Network::Base => Address::from_str("0x4752ba5dbc23f44d87826276bf6fd6b1c372ad24")?,
        _ => Address::from_str("0x4752ba5dbc23f44d87826276bf6fd6b1c372ad24")?,
    };

    let uniswap_router_v2 = BaseUniswapV2Router::new(uniswap_router_v2_address, provider.clone());

    let uniswap_v4_universal_router_address: Address = match network {
        Network::Ethereum => Address::from_str("0x66a9893cc07d91d95644aedd05d03f95e1dba8af")?,
        Network::Base => Address::from_str("0x6fF5693b99212Da76ad316178A184AB56D299b43")?,
        _ => Address::from_str("0x6fF5693b99212Da76ad316178A184AB56D299b43")?,
    };

    let uniswap_v4_universal_router =
        UniswapV4UniversalRouter::new(uniswap_v4_universal_router_address, &provider);

    let uniswap_v4_position_manager_address = match network.clone() {
        Network::Ethereum => Address::from_str("0xbd216513d74c8cf14cf4747e6aaa6420ff64ee9e")?,
        Network::Base => Address::from_str("7c5f5a4bbd8fd63184577525326123b519429bdc")?,
        _ => Address::from_str("7c5f5a4bbd8fd63184577525326123b519429bdc")?,
    };

    let uniswap_v4_position_manager =
        UniswapV4PositionManager::new(uniswap_v4_position_manager_address, provider.clone());

    if buy {
        token_in = network.get_base_token();
        token_out = token.token;

        if pool.version == 2 {
            let path: Vec<Address> = vec![];
            let result = uniswap_router_v2.swapExactETHForTokensSupportingFeeOnTransferTokens(
                amount_out_minimum,
                path,
                signer.clone().address(),
                deadline,
            );

            build_requests.push(BuildRequests {
                from: signer.address(),
                to: *uniswap_router_v2.address(),
                gas_limit: 1_000_000,
                input: result.calldata().clone(),
                value: amount_in,
                nonce,
                signer,
                tx_type: "BUY".to_string(),
            });
        } else if pool.version == 4 {
            let eth = Token {
                address: Address::from_str("0x0000000000000000000000000000000000000000")?,
                decimals: 18,
            };

            let pool_str: String = pool.pool.chars().take(52).collect();
            let pool_keys = uniswap_v4_position_manager
                .poolKeys(FixedBytes::from_str(&pool_str)?)
                .call()
                .await?;

            let token_pool_key = PoolKey {
                currency0: pool_keys.currency0,
                currency1: pool_keys.currency1,
                fee: pool_keys.fee.to::<u32>(),
                tick_spacing: pool_keys.tickSpacing.to_string().parse::<i32>()?,
                hooks: pool_keys.hooks,
            };

            let current_config = SwapExactIn {
                currency_in: eth.address,
                path: encode_multihop_exact_in_path(std::slice::from_ref(&token_pool_key), eth.address),
                amount_in,
                amount_out_minimum,
            };

            let mut v4_planner = V4Planner::new();
            v4_planner.add_action(
                V4Action::SwapExactIn,
                encode_swap_exact_in(&current_config),
            );
            v4_planner.add_action(
                V4Action::SettleAll,
                encode_settle_all(token_pool_key.currency0, current_config.amount_in),
            );
            v4_planner.add_action(
                V4Action::TakeAll,
                encode_take_all(token_pool_key.currency1, current_config.amount_out_minimum),
            );

            let encoded_actions = v4_planner.finalize();
            let mut route_planner = RoutePlanner::new();
            route_planner.add_command(CommandType::V4Swap, encoded_actions);

            let commands = Bytes::from(route_planner.commands);
            let inputs = route_planner.inputs;

            build_requests.push(BuildRequests {
                from: signer.address(),
                to: *uniswap_v4_universal_router.address(),
                gas_limit: 500_000,
                input: uniswap_v4_universal_router
                    .execute_1(commands, inputs, deadline)
                    .calldata()
                    .clone(),
                value: amount_in,
                nonce,
                signer,
                tx_type: "BUY".to_string(),
            });
        }
    } else {
        token_in = token.token;
        token_out = network.get_base_token();

        let contract_address = token.token;
        let contract = ERC20Contract::new(contract_address, provider.clone());
        let permit_contract = Permit2::new(
            Address::from_str("0x000000000022D473030F116dDEE9F6B43aC78BA3").unwrap(),
            provider.clone(),
        );

        let spender: Address;
        let allowance: U256;

        if pool.version == 2 {
            spender = *uniswap_router_v2.address();
            allowance = contract.allowance(signer.address(), spender).call().await?;
        } else if pool.version == 4 {
            spender = *uniswap_v4_universal_router.address();

            let token_to_permit2_allowance = contract
                .allowance(signer.address(), *permit_contract.address())
                .call()
                .await?;

            if token_to_permit2_allowance.le(&amount_in) {
                let max_approval = U256::from_str(MAX_UINT_HEX)?;
                let approve_result = contract.approve(*permit_contract.address(), max_approval);

                build_requests.push(BuildRequests {
                    from: signer.address(),
                    to: *contract.address(),
                    gas_limit: 100_000,
                    input: approve_result.calldata().clone(),
                    value: U256::from(0),
                    nonce,
                    signer: signer.clone(),
                    tx_type: "APPROVE".to_string(),
                });

                nonce += 1;
            }

            allowance = permit_contract
                .allowance(signer.address(), *contract.address(), spender)
                .call()
                .await?
                .amount
                .to::<U256>();
        } else {
            return Err(format!("Unsupported pool version {} for sell", pool.version).into());
        };

        if allowance.le(&amount_in) {
            let result =
                permit_contract.approve(*contract.address(), spender, U160::MAX, U48::MAX);

            build_requests.push(BuildRequests {
                from: signer.address(),
                to: *permit_contract.address(),
                gas_limit: 150_000,
                input: result.calldata().clone(),
                value: U256::from(0),
                nonce,
                signer: signer.clone(),
                tx_type: "APPROVE".to_string(),
            });

            nonce += 1;
        }

        if pool.version == 2 {
            let path: Vec<Address> = vec![];
            let result = uniswap_router_v2.swapExactTokensForETHSupportingFeeOnTransferTokens(
                amount_in,
                amount_out_minimum,
                path,
                signer.address(),
                deadline,
            );

            build_requests.push(BuildRequests {
                from: signer.address(),
                to: *uniswap_router_v2.address(),
                gas_limit: 1_000_000,
                input: result.calldata().clone(),
                value: U256::from(0),
                nonce,
                signer: signer.clone(),
                tx_type: "SELL".to_string(),
            });
        } else if pool.version == 4 {
            let pool_str: String = pool.pool.chars().take(52).collect();
            let pool_keys = uniswap_v4_position_manager
                .poolKeys(FixedBytes::from_str(&pool_str)?)
                .call()
                .await?;

            let token_pool_key = PoolKey {
                currency0: pool_keys.currency0,
                currency1: pool_keys.currency1,
                fee: pool_keys.fee.to::<u32>(),
                tick_spacing: pool_keys.tickSpacing.to_string().parse::<i32>()?,
                hooks: pool_keys.hooks,
            };

            let currency_out = if token.token == token_pool_key.currency0 {
                token_pool_key.currency1
            } else {
                token_pool_key.currency0
            };

            let current_config = SwapExactIn {
                currency_in: token.token,
                path: encode_multihop_exact_in_path(std::slice::from_ref(&token_pool_key), token.token),
                amount_in,
                amount_out_minimum,
            };

            let mut v4_planner = V4Planner::new();
            v4_planner.add_action(
                V4Action::SwapExactIn,
                encode_swap_exact_in(&current_config),
            );
            v4_planner.add_action(
                V4Action::SettleAll,
                encode_settle_all(token.token, current_config.amount_in),
            );
            v4_planner.add_action(
                V4Action::TakeAll,
                encode_take_all(currency_out, current_config.amount_out_minimum),
            );

            let encoded_actions = v4_planner.finalize();
            let mut route_planner = RoutePlanner::new();
            route_planner.add_command(CommandType::V4Swap, encoded_actions);

            let commands = Bytes::from(route_planner.commands);
            let inputs = route_planner.inputs;

            build_requests.push(BuildRequests {
                from: signer.address(),
                to: *uniswap_v4_universal_router.address(),
                gas_limit: 500_000,
                input: uniswap_v4_universal_router
                    .execute_1(commands, inputs, deadline)
                    .calldata()
                    .clone(),
                value: U256::from(0),
                nonce,
                signer: signer.clone(),
                tx_type: "SELL".to_string(),
            });
        }
    }

    if build_requests.is_empty() {
        return Ok(());
    }

    let decimals = u256_to_f64(U256::from(10).pow(U256::from(token.decimals)));

    for build_request in build_requests.iter() {
        let tx_request = TransactionRequest::default()
            .with_from(build_request.from)
            .with_to(build_request.to)
            .with_value(build_request.value)
            .with_input(build_request.input.clone())
            .with_gas_limit(build_request.gas_limit.try_into()?)
            .with_max_fee_per_gas(max_fee_per_gas)
            .with_max_priority_fee_per_gas(max_priority_fee_per_gas)
            .with_nonce(build_request.nonce)
            .with_chain_id(network.clone() as u64);

        let eth_wallet = EthereumWallet::from(build_request.signer.clone());
        let tx_envelope = match tx_request.clone().build(&eth_wallet).await {
            Ok(env) => env,
            Err(e) => {
                tracing::error!("[EVM TX] Failed to build tx envelope (type={}): {}", build_request.tx_type, e);
                return Err(e.into());
            }
        };
        let tx: PendingTransactionBuilder<Ethereum> = match provider.send_tx_envelope(tx_envelope.clone()).await {
            Ok(t) => t,
            Err(e) => {
                tracing::error!("[EVM TX] Failed to send tx (type={}): {}", build_request.tx_type, e);
                return Err(e.into());
            }
        };

        let tx_mpsc = tx_mpsc.clone();
        let tx_watcher_info = tx_watcher_info.clone();
        let req_from = build_request.from;
        let req_tx_type = build_request.tx_type.clone();
        tokio::task::spawn(async move {
            let new_tx_request = NewTransaction {
                slot: 0,
                project_id: tx_watcher_info.project_id,
                sol_price: f64_to_big_int(tx_watcher_info.eth_price),
                value: if buy {
                    f64_to_big_int(u256_to_f64(amount_in) / 1e18f64)
                } else {
                    f64_to_big_int(u256_to_f64(amount_out_minimum) / 1e18f64)
                },
                tokens: if buy {
                    f64_to_big_int(u256_to_f64(amount_out_minimum) / decimals)
                } else {
                    f64_to_big_int(u256_to_f64(amount_in) / decimals)
                },
                address: req_from.to_string(),
                tx_hash: tx.tx_hash().to_string(),
                tx_type: req_tx_type,
                token_in: token_in.to_string(),
                token_out: token_out.to_string(),
                date_added: SystemTime::now(),
            };

            let _ = tx_mpsc
                .send(StreamInfo {
                    user_id: tx_watcher_info.user_id,
                    stream_type: StreamType::NewTransactionRequest(new_tx_request.clone()),
                })
                .await;

            let receipt = tx
                .with_timeout(Some(Duration::from_secs(12)))
                .get_receipt()
                .await
                .ok();

            if let Some(receipt) = receipt {
                let mut actual_value_f64: Option<f64> = None;
                let mut actual_tokens_f64: Option<f64> = None;

                let is_trade = new_tx_request.tx_type == "BUY" || new_tx_request.tx_type == "SELL";

                // Parse receipt logs for actual output amounts (BUY/SELL only)
                if receipt.status() && is_trade {
                    // ERC-20 Transfer(address indexed from, address indexed to, uint256 value)
                    let transfer_topic = FixedBytes::<32>::from_str(
                        "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
                    )
                    .unwrap();

                    if buy {
                        // Buy: ETH spent is exact (amount_in), parse actual tokens received
                        actual_value_f64 = Some(u256_to_f64(amount_in) / 1e18);

                        for log in receipt.inner.logs() {
                            if log.inner.address == token_out
                                && log.inner.data.topics().len() >= 3
                                && log.inner.data.topics()[0] == transfer_topic
                            {
                                let to = Address::from_word(log.inner.data.topics()[2]);
                                if to == req_from {
                                    let raw = U256::from_be_slice(log.inner.data.data.as_ref());
                                    actual_tokens_f64 =
                                        Some(u256_to_f64(raw) / decimals);
                                }
                            }
                        }
                    } else {
                        // Sell: tokens sent is exact (amount_in), parse actual ETH received
                        actual_tokens_f64 = Some(u256_to_f64(amount_in) / decimals);

                        // Try WETH Withdrawal first (V2 sells unwrap WETH → ETH)
                        let withdrawal_topic = FixedBytes::<32>::from_str(
                            "0x7fcf532c15f0a6db0bd6d0e038bea71d30d808c7d98cb3bf7268a95bf5081b65",
                        )
                        .unwrap();

                        for log in receipt.inner.logs() {
                            if log.inner.address == token_out
                                && !log.inner.data.topics().is_empty()
                                && log.inner.data.topics()[0] == withdrawal_topic
                            {
                                let raw = U256::from_be_slice(log.inner.data.data.as_ref());
                                actual_value_f64 = Some(u256_to_f64(raw) / 1e18);
                            }
                        }

                        // V4 fallback: native ETH output has no WETH event,
                        // parse Swap event for actual ETH amount
                        if actual_value_f64.is_none() {
                            // V4 Swap(bytes32 indexed, address indexed, int128 amount0, int128 amount1, ...)
                            let swap_topic = FixedBytes::<32>::from_str(
                                "0x40e9cecb9f5f1f1c5b9c97dec2917b7ee92e57ba5563708daca94dd84ad7112f",
                            )
                            .unwrap();

                            for log in receipt.inner.logs() {
                                if !log.inner.data.topics().is_empty()
                                    && log.inner.data.topics()[0] == swap_topic
                                    && log.inner.data.data.len() >= 32
                                {
                                    // amount0 = first 32 bytes (int128 sign-extended to 256 bits)
                                    // For native ETH pools: currency0 = address(0) = ETH
                                    // Positive amount0 = ETH received by swapper
                                    let amount0_bytes: [u8; 32] =
                                        log.inner.data.data[0..32].try_into().unwrap();
                                    if amount0_bytes[0] & 0x80 == 0 {
                                        let amount0 = U256::from_be_bytes(amount0_bytes);
                                        if amount0 > U256::ZERO {
                                            actual_value_f64 =
                                                Some(u256_to_f64(amount0) / 1e18);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                let _ = tx_mpsc
                    .send(StreamInfo {
                        user_id: tx_watcher_info.user_id,
                        stream_type: StreamType::UpdateTransactionSlot {
                            tx_hash: new_tx_request.tx_hash.clone(),
                            slot: receipt.block_number.unwrap_or(0) as i64,
                            value: actual_value_f64.map(f64_to_big_int),
                            tokens: actual_tokens_f64.map(f64_to_big_int),
                        },
                    })
                    .await;

                if is_trade && tx_watcher_info.trade_volume_usdc > 0.0 && receipt.status() {
                    let actual_volume_usdc = if buy {
                        actual_value_f64.unwrap_or(0.0) * tx_watcher_info.eth_price
                    } else {
                        actual_value_f64
                            .map(|v| v * tx_watcher_info.eth_price)
                            .unwrap_or(tx_watcher_info.trade_volume_usdc)
                    };

                    let _ = tx_mpsc
                        .send(StreamInfo {
                            user_id: tx_watcher_info.user_id,
                            stream_type: StreamType::TradeConfirmed {
                                project_id: tx_watcher_info.project_id,
                                volume_usdc: actual_volume_usdc,
                            },
                        })
                        .await;
                }
            }
        });
    }

    Ok(())
}
