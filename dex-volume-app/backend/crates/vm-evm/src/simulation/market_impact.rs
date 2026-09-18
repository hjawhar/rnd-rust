use std::error::Error;
use std::str::FromStr;

use alloy::{
    network::TransactionBuilder,
    primitives::{Address, Bytes, U256},
    providers::Provider,
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
    sol,
    sol_types::SolCall,
};

use crate::constants::Network;
use crate::simulation::erigon::trace_txs;

fn get_v4_quoter_address(network: &Network) -> Option<Address> {
    match network {
        Network::Ethereum => Address::from_str("0x52f0e24d1c21c8a0cb1e5a5dd6198556bd9e1203").ok(),
        Network::Base => Address::from_str("0x0d5e0f971ed27fbff6c2837bf31316121532048d").ok(),
        _ => None,
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PoolKeyParams {
    pub currency0: Address,
    pub currency1: Address,
    pub fee: u32,
    pub tick_spacing: i32,
    pub hooks: Address,
}

pub async fn calculate_market_impact_v4(
    network: Network,
    pool_key: PoolKeyParams,
    amount_in: U256,
    is_buy: bool,
    _token_price_eth: f64,
    _token_decimals: u8,
) -> Result<u16, Box<dyn Error + Send + Sync>> {
    let quoter_address = get_v4_quoter_address(&network)
        .ok_or("V4 Quoter not available for this network")?;

    let eth_address = Address::ZERO;
    let zero_for_one = if is_buy {
        pool_key.currency0 == eth_address
    } else {
        pool_key.currency0 != eth_address
    };

    let provider = network.get_provider();

    sol! {
        struct PoolKey {
            address currency0;
            address currency1;
            uint24 fee;
            int24 tickSpacing;
            address hooks;
        }

        struct QuoteExactSingleParams {
            PoolKey poolKey;
            bool zeroForOne;
            uint128 exactAmount;
            bytes hookData;
        }

        function quoteExactInputSingle(QuoteExactSingleParams calldata params) external returns (uint256 amountOut, uint256 gasEstimate);
    }

    let pool_key_sol = PoolKey {
        currency0: pool_key.currency0,
        currency1: pool_key.currency1,
        fee: pool_key.fee.try_into()?,
        tickSpacing: pool_key.tick_spacing.try_into()?,
        hooks: pool_key.hooks,
    };

    // Small reference quote to derive spot price (0.01 ETH or 1% of amount_in, whichever is smaller)
    let ref_amount = amount_in / U256::from(100);
    let ref_amount = if ref_amount == U256::ZERO { U256::from(1) } else { ref_amount };

    let ref_params = QuoteExactSingleParams {
        poolKey: pool_key_sol.clone(),
        zeroForOne: zero_for_one,
        exactAmount: ref_amount.try_into()?,
        hookData: Bytes::new(),
    };
    let ref_call_data = quoteExactInputSingleCall { params: ref_params }.abi_encode();

    // Actual trade quote
    let trade_params = QuoteExactSingleParams {
        poolKey: pool_key_sol,
        zeroForOne: zero_for_one,
        exactAmount: amount_in.try_into()?,
        hookData: Bytes::new(),
    };
    let trade_call_data = quoteExactInputSingleCall { params: trade_params }.abi_encode();

    let signer = PrivateKeySigner::random();
    let gas_price = provider.estimate_eip1559_fees().await?;

    let ref_tx = TransactionRequest::default()
        .with_from(signer.address())
        .with_to(quoter_address)
        .with_value(U256::ZERO)
        .with_input(Bytes::from(ref_call_data))
        .with_gas_limit(500_000)
        .with_max_fee_per_gas(gas_price.max_fee_per_gas)
        .with_max_priority_fee_per_gas(gas_price.max_priority_fee_per_gas)
        .with_nonce(0)
        .with_chain_id(network.clone() as u64);

    let trade_tx = TransactionRequest::default()
        .with_from(signer.address())
        .with_to(quoter_address)
        .with_value(U256::ZERO)
        .with_input(Bytes::from(trade_call_data))
        .with_gas_limit(500_000)
        .with_max_fee_per_gas(gas_price.max_fee_per_gas)
        .with_max_priority_fee_per_gas(gas_price.max_priority_fee_per_gas)
        .with_nonce(0)
        .with_chain_id(network.clone() as u64);

    // Batch both quotes in a single trace_txs call
    let response = trace_txs(network, vec![ref_tx, trade_tx], None).await?;

    if response.result.len() < 2 {
        return Err("Incomplete response from quoter".into());
    }

    let ref_data = Bytes::from_str(&response.result[0].output)?;
    let ref_decoded = quoteExactInputSingleCall::abi_decode_returns(&ref_data[..])?;
    let ref_out = ref_decoded.amountOut;

    let trade_data = Bytes::from_str(&response.result[1].output)?;
    let trade_decoded = quoteExactInputSingleCall::abi_decode_returns(&trade_data[..])?;
    let trade_out = trade_decoded.amountOut;

    // Compute impact: compare actual rate vs reference (spot) rate
    // spot_rate = ref_out / ref_amount, fair_output = spot_rate * amount_in
    // impact = 1 - (trade_out / fair_output)
    let ref_amount_f = ref_amount.to::<u128>() as f64;
    let ref_out_f = ref_out.to::<u128>() as f64;
    let amount_in_f = amount_in.to::<u128>() as f64;
    let trade_out_f = trade_out.to::<u128>() as f64;

    if ref_out_f == 0.0 || ref_amount_f == 0.0 {
        return Ok(0);
    }

    let fair_output = (ref_out_f / ref_amount_f) * amount_in_f;
    let impact_bps = if trade_out_f >= fair_output {
        0u16
    } else {
        (((fair_output - trade_out_f) / fair_output) * 10000.0) as u16
    };

    tracing::debug!(
        "V4 market impact: amount_in={}, ref_amount={}, ref_out={}, trade_out={}, fair_output={:.0}, impact={}bps",
        amount_in, ref_amount, ref_out, trade_out, fair_output, impact_bps
    );

    Ok(impact_bps)
}

/// Get spot price for a V4 pool by quoting a small ETH→token swap via the on-chain quoter.
/// Returns `token_price_eth` (how much ETH one token costs).
pub async fn quote_v4_spot_price(
    network: Network,
    pool_key: PoolKeyParams,
    token_decimals: u8,
) -> Result<f64, Box<dyn Error + Send + Sync>> {
    let quoter_address = get_v4_quoter_address(&network)
        .ok_or("V4 Quoter not available for this network")?;

    let eth_address = Address::ZERO;
    // Quote ETH→token (zeroForOne when currency0 is ETH)
    let zero_for_one = pool_key.currency0 == eth_address;

    let provider = network.get_provider();

    sol! {
        struct PoolKey {
            address currency0;
            address currency1;
            uint24 fee;
            int24 tickSpacing;
            address hooks;
        }

        struct QuoteExactSingleParams {
            PoolKey poolKey;
            bool zeroForOne;
            uint128 exactAmount;
            bytes hookData;
        }

        function quoteExactInputSingle(QuoteExactSingleParams calldata params) external returns (uint256 amountOut, uint256 gasEstimate);
    }

    // Quote 0.01 ETH → tokens
    let ref_eth = U256::from(10_000_000_000_000_000u64); // 0.01 ETH in wei

    let params = QuoteExactSingleParams {
        poolKey: PoolKey {
            currency0: pool_key.currency0,
            currency1: pool_key.currency1,
            fee: pool_key.fee.try_into()?,
            tickSpacing: pool_key.tick_spacing.try_into()?,
            hooks: pool_key.hooks,
        },
        zeroForOne: zero_for_one,
        exactAmount: ref_eth.try_into()?,
        hookData: Bytes::new(),
    };

    let call_data = quoteExactInputSingleCall { params }.abi_encode();

    let signer = PrivateKeySigner::random();
    let gas_price = provider.estimate_eip1559_fees().await?;

    let tx_request = TransactionRequest::default()
        .with_from(signer.address())
        .with_to(quoter_address)
        .with_value(U256::ZERO)
        .with_input(Bytes::from(call_data))
        .with_gas_limit(500_000)
        .with_max_fee_per_gas(gas_price.max_fee_per_gas)
        .with_max_priority_fee_per_gas(gas_price.max_priority_fee_per_gas)
        .with_nonce(0)
        .with_chain_id(network.clone() as u64);

    let response = trace_txs(network, vec![tx_request], None).await?;

    if response.result.is_empty() {
        return Err("Empty response from V4 quoter".into());
    }

    let data = Bytes::from_str(&response.result[0].output)?;
    let decoded = quoteExactInputSingleCall::abi_decode_returns(&data[..])?;
    let tokens_out = decoded.amountOut;

    let decimals_factor = 10_f64.powi(token_decimals as i32);
    let tokens_out_f = tokens_out.to::<u128>() as f64 / decimals_factor;

    if tokens_out_f == 0.0 {
        return Err("V4 quoter returned 0 tokens".into());
    }

    // token_price_eth = eth_in / tokens_out
    let token_price_eth = 0.01 / tokens_out_f;
    Ok(token_price_eth)
}

pub fn estimate_impact_from_liquidity(
    amount_in_eth: f64,
    pool_liquidity_eth: f64,
) -> u16 {
    if pool_liquidity_eth <= 0.0 {
        return 10000;
    }
    let impact = amount_in_eth / (2.0 * pool_liquidity_eth);
    (impact * 10000.0).min(10000.0) as u16
}
