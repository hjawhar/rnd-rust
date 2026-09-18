use vm_data::models::task::TaskDisperseETH;
use vm_evm::constants::{DISPERSE_APP, Network};
use vm_evm::contracts::DisperseApp;
use vm_evm::helpers::{str_to_pk, f64_to_wei, wei_to_f64};
use alloy::network::EthereumWallet;
use alloy::primitives::{Address, U256};
use alloy::providers::{Provider, ProviderBuilder};
use std::error::Error;
use std::str::FromStr;
use std::time::Duration;

pub async fn disperse_eth(
    _nc: async_nats::Client,
    task: TaskDisperseETH,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let network = Network::Base;
    let rpc_url: reqwest::Url = network.get_endpoint().parse()?;

    let signer = str_to_pk(&task.sender_pk)?;
    let sender_addr = signer.address();
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(rpc_url);

    // Get sender balance
    let balance = provider.get_balance(sender_addr).await?;
    let gas_buffer = f64_to_wei(0.01); // Keep 0.01 ETH for gas

    if balance <= gas_buffer {
        return Err("Insufficient ETH balance for dispersal".into());
    }

    let distributable = balance - gas_buffer;
    let count = task.recipients_pubkeys.len() as u64;
    let per_recipient = distributable / U256::from(count);

    let recipients: Vec<Address> = task
        .recipients_pubkeys
        .iter()
        .filter_map(|r| Address::from_str(r).ok())
        .collect();

    let values: Vec<U256> = vec![per_recipient; recipients.len()];
    let total_value: U256 = values.iter().copied().sum();

    let disperse = DisperseApp::new(DISPERSE_APP, &provider);
    let tx = disperse
        .disperseEther(recipients, values)
        .value(total_value);

    let pending = tx.send().await?;
    match pending.with_timeout(Some(Duration::from_secs(30))).get_receipt().await {
        Ok(receipt) => {
            tracing::info!(
                "[EVM] Dispersed {:.6} ETH to {} recipients: {:?}",
                wei_to_f64(total_value),
                count,
                receipt.transaction_hash
            );
        }
        Err(e) => {
            tracing::warn!("[EVM] Disperse ETH receipt timeout: {}", e);
        }
    }

    Ok(())
}
