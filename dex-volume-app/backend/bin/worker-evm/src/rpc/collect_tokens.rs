use vm_data::models::task::TaskCollectEvmTokens;
use vm_evm::constants::Network;
use vm_evm::contracts::ERC20Contract;
use vm_evm::helpers::str_to_pk;
use vm_evm::simulation::multicall::batch_get_token_balances;
use alloy::network::EthereumWallet;
use alloy::primitives::{Address, U256};
use alloy::providers::ProviderBuilder;
use alloy::signers::k256;
use alloy::signers::local::LocalSigner;
use std::error::Error;
use std::str::FromStr;
use std::time::Duration;

pub async fn collect_tokens(
    _nc: async_nats::Client,
    task: TaskCollectEvmTokens,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let network = Network::from_network_name(&task.network)
        .unwrap_or(Network::Base);
    let rpc_url: reqwest::Url = network.get_endpoint().parse()?;
    let token_addr = Address::from_str(&task.token)?;

    let recipient_signer = str_to_pk(&task.recipient_pk)?;
    let recipient_addr = recipient_signer.address();

    // Pre-parse all PKs upfront
    let mut wallet_info: Vec<(Address, LocalSigner<k256::ecdsa::SigningKey>, String)> = Vec::new();
    for sender_pk in &task.senders_pks {
        match str_to_pk(sender_pk) {
            Ok(signer) => wallet_info.push((signer.address(), signer, sender_pk.clone())),
            Err(e) => tracing::warn!("[EVM] Invalid sender PK: {}", e),
        }
    }

    if wallet_info.is_empty() {
        return Ok(());
    }

    let addresses: Vec<Address> = wallet_info.iter().map(|(a, _, _)| *a).collect();

    // Batch fetch all token balances in 1 Multicall3 call
    let balances = batch_get_token_balances(&network, token_addr, &addresses).await?;

    // Only spawn send tasks for wallets with balance > 0
    for ((addr, signer, _), balance) in wallet_info.into_iter().zip(balances) {
        if balance == U256::ZERO {
            continue;
        }

        let rpc_url = rpc_url.clone();

        tokio::spawn(async move {
            let wallet = EthereumWallet::from(signer);
            let provider = ProviderBuilder::new()
                .wallet(wallet)
                .connect_http(rpc_url);

            let erc20 = ERC20Contract::new(token_addr, &provider);

            match erc20.transfer(recipient_addr, balance).send().await {
                Ok(pending) => {
                    match pending.with_timeout(Some(Duration::from_secs(30))).get_receipt().await {
                        Ok(receipt) => {
                            tracing::info!(
                                "[EVM] Collected tokens from {:?}: {:?}",
                                addr,
                                receipt.transaction_hash
                            );
                        }
                        Err(e) => tracing::warn!("[EVM] Collect tokens receipt timeout for {:?}: {}", addr, e),
                    }
                }
                Err(e) => tracing::warn!("[EVM] Collect tokens send failed for {:?}: {}", addr, e),
            }
        });
    }

    Ok(())
}
