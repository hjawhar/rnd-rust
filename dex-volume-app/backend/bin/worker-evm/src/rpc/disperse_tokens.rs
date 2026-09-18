use vm_data::models::task::TaskDisperseEvmTokens;
use vm_evm::constants::{DISPERSE_APP, MULTICALL3, Network};
use vm_evm::contracts::{DisperseApp, ERC20Contract, Multicall3Contract};
use vm_evm::helpers::str_to_pk;
use alloy::primitives::{Address, U256};
use alloy::sol_types::SolCall;
use alloy::network::EthereumWallet;
use alloy::providers::ProviderBuilder;
use std::error::Error;
use std::str::FromStr;
use std::time::Duration;

pub async fn disperse_tokens(
    _nc: async_nats::Client,
    task: TaskDisperseEvmTokens,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let network = Network::from_network_name(&task.network)
        .unwrap_or(Network::Base);
    let rpc_url: reqwest::Url = network.get_endpoint().parse()?;
    let token_addr = Address::from_str(&task.token)?;

    let signer = str_to_pk(&task.sender_pk)?;
    let sender_addr = signer.address();
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(rpc_url);

    // Batch balanceOf + allowance in 1 Multicall3 call (saves 1 RPC round-trip)
    let read_provider = network.get_provider();
    let multicall = Multicall3Contract::new(MULTICALL3, &read_provider);
    let calls = vec![
        Multicall3Contract::Call3 {
            target: token_addr,
            allowFailure: false,
            callData: ERC20Contract::balanceOfCall { account: sender_addr }
                .abi_encode()
                .into(),
        },
        Multicall3Contract::Call3 {
            target: token_addr,
            allowFailure: false,
            callData: ERC20Contract::allowanceCall {
                _owner: sender_addr,
                spender: DISPERSE_APP,
            }
            .abi_encode()
            .into(),
        },
    ];
    let results = multicall.aggregate3(calls).call().await?;

    let balance = if results[0].success && results[0].returnData.len() >= 32 {
        U256::from_be_slice(&results[0].returnData[results[0].returnData.len() - 32..])
    } else {
        return Err("Failed to fetch token balance".into());
    };
    if balance == U256::ZERO {
        return Err("No token balance to disperse".into());
    }

    let allowance = if results[1].success && results[1].returnData.len() >= 32 {
        U256::from_be_slice(&results[1].returnData[results[1].returnData.len() - 32..])
    } else {
        U256::ZERO
    };

    // Approve DisperseApp if needed
    let erc20 = ERC20Contract::new(token_addr, &provider);
    if allowance < balance {
        let approve_tx = erc20.approve(DISPERSE_APP, U256::MAX).send().await?;
        approve_tx
            .with_timeout(Some(Duration::from_secs(30)))
            .get_receipt()
            .await?;
    }

    let count = task.recipients_pubkeys.len() as u64;
    let per_recipient = balance / U256::from(count);

    let recipients: Vec<Address> = task
        .recipients_pubkeys
        .iter()
        .filter_map(|r| Address::from_str(r).ok())
        .collect();

    let values: Vec<U256> = vec![per_recipient; recipients.len()];

    let disperse = DisperseApp::new(DISPERSE_APP, &provider);
    let tx = disperse.disperseToken(token_addr, recipients, values);

    let pending = tx.send().await?;
    match pending.with_timeout(Some(Duration::from_secs(30))).get_receipt().await {
        Ok(receipt) => {
            tracing::info!(
                "[EVM] Dispersed tokens to {} recipients: {:?}",
                count,
                receipt.transaction_hash
            );
        }
        Err(e) => {
            tracing::warn!("[EVM] Disperse tokens receipt timeout: {}", e);
        }
    }

    Ok(())
}
