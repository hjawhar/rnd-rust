use alloy::primitives::{Address, U256};
use alloy::sol_types::SolCall;
use std::error::Error;

use crate::constants::{MULTICALL3, Network};
use crate::contracts::{ERC20Contract, Multicall3Contract};

/// Batch-fetch ETH balances for N addresses in 1 RPC call via Multicall3.
pub async fn batch_get_eth_balances(
    network: &Network,
    addresses: &[Address],
) -> Result<Vec<U256>, Box<dyn Error + Send + Sync>> {
    if addresses.is_empty() {
        return Ok(vec![]);
    }

    let provider = network.get_provider();
    let multicall = Multicall3Contract::new(MULTICALL3, &provider);

    let calls: Vec<Multicall3Contract::Call3> = addresses
        .iter()
        .map(|addr| Multicall3Contract::Call3 {
            target: MULTICALL3,
            allowFailure: true,
            callData: Multicall3Contract::getEthBalanceCall {
                addr: *addr,
            }
            .abi_encode()
            .into(),
        })
        .collect();

    let results = multicall.aggregate3(calls).call().await?;

    let balances: Vec<U256> = results
        .iter()
        .map(|r| {
            if r.success && r.returnData.len() >= 32 {
                U256::from_be_slice(&r.returnData[r.returnData.len() - 32..])
            } else {
                U256::ZERO
            }
        })
        .collect();

    Ok(balances)
}

/// Batch-fetch ERC-20 balances for N addresses in 1 RPC call via Multicall3.
pub async fn batch_get_token_balances(
    network: &Network,
    token: Address,
    addresses: &[Address],
) -> Result<Vec<U256>, Box<dyn Error + Send + Sync>> {
    if addresses.is_empty() {
        return Ok(vec![]);
    }

    let provider = network.get_provider();
    let multicall = Multicall3Contract::new(MULTICALL3, &provider);

    let calls: Vec<Multicall3Contract::Call3> = addresses
        .iter()
        .map(|addr| Multicall3Contract::Call3 {
            target: token,
            allowFailure: true,
            callData: ERC20Contract::balanceOfCall {
                account: *addr,
            }
            .abi_encode()
            .into(),
        })
        .collect();

    let results = multicall.aggregate3(calls).call().await?;

    let balances: Vec<U256> = results
        .iter()
        .map(|r| {
            if r.success && r.returnData.len() >= 32 {
                U256::from_be_slice(&r.returnData[r.returnData.len() - 32..])
            } else {
                U256::ZERO
            }
        })
        .collect();

    Ok(balances)
}

/// Batch-fetch ERC-20 allowances for N (owner, spender) pairs in 1 RPC call via Multicall3.
pub async fn batch_get_token_allowances(
    network: &Network,
    token: Address,
    owners: &[Address],
    spender: Address,
) -> Result<Vec<U256>, Box<dyn Error + Send + Sync>> {
    if owners.is_empty() {
        return Ok(vec![]);
    }

    let provider = network.get_provider();
    let multicall = Multicall3Contract::new(MULTICALL3, &provider);

    let calls: Vec<Multicall3Contract::Call3> = owners
        .iter()
        .map(|owner| Multicall3Contract::Call3 {
            target: token,
            allowFailure: true,
            callData: ERC20Contract::allowanceCall {
                _owner: *owner,
                spender,
            }
            .abi_encode()
            .into(),
        })
        .collect();

    let results = multicall.aggregate3(calls).call().await?;

    let allowances: Vec<U256> = results
        .iter()
        .map(|r| {
            if r.success && r.returnData.len() >= 32 {
                U256::from_be_slice(&r.returnData[r.returnData.len() - 32..])
            } else {
                U256::ZERO
            }
        })
        .collect();

    Ok(allowances)
}
