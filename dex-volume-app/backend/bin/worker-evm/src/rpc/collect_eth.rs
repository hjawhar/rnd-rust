use vm_data::models::task::TaskCollectETH;
use vm_evm::constants::Network;
use vm_evm::helpers::{str_to_pk, wei_to_f64};
use vm_evm::simulation::multicall::batch_get_eth_balances;
use alloy::network::EthereumWallet;
use alloy::primitives::{Address, U256};
use alloy::providers::ProviderBuilder;
use alloy::rpc::types::TransactionRequest;
use alloy::signers::k256;
use alloy::signers::local::LocalSigner;
use alloy::providers::Provider;
use std::error::Error;
use std::time::Duration;

pub async fn collect_eth(
    _nc: async_nats::Client,
    task: TaskCollectETH,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let network = Network::from_network_name(&task.network)
        .unwrap_or(Network::Base);
    let rpc_url: reqwest::Url = network.get_endpoint().parse()?;

    // Fetch gas price ONCE before the loop (saves N-1 RPC calls)
    let read_provider = network.get_provider();
    let gas_price = read_provider.get_gas_price().await?;
    let gas_limit = 21000u64;
    // 15% buffer on gas price
    let fee = gas_price + gas_price * 15 / 100;
    // L2 chains (Base/OP/Arb) charge an L1 data fee on top of L2 gas
    let l1_data_fee_buffer = U256::from(50_000_000_000u64); // 50k gwei
    let total_cost = U256::from(gas_limit) * U256::from(fee) + l1_data_fee_buffer;

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

    // Batch fetch all balances in 1 Multicall3 call
    let balances = batch_get_eth_balances(&network, &addresses).await?;

    // Only spawn send tasks for funded wallets
    for ((addr, signer, _), balance) in wallet_info.into_iter().zip(balances) {
        if balance == U256::ZERO || balance <= total_cost {
            if balance > U256::ZERO {
                tracing::warn!("[EVM] Insufficient balance for gas in {:?}", addr);
            }
            continue;
        }

        let send_amount = balance - total_cost;
        let rpc_url = rpc_url.clone();

        tokio::spawn(async move {
            let wallet = EthereumWallet::from(signer);
            let provider = ProviderBuilder::new()
                .wallet(wallet)
                .connect_http(rpc_url);

            let tx = TransactionRequest::default()
                .to(recipient_addr)
                .value(send_amount)
                .gas_limit(gas_limit)
                .max_fee_per_gas(fee)
                .max_priority_fee_per_gas(fee);

            match provider.send_transaction(tx).await {
                Ok(pending) => {
                    match pending.with_timeout(Some(Duration::from_secs(30))).get_receipt().await {
                        Ok(receipt) => {
                            tracing::info!(
                                "[EVM] Collected {:.6} ETH from {:?}: {:?}",
                                wei_to_f64(send_amount),
                                addr,
                                receipt.transaction_hash
                            );
                        }
                        Err(e) => tracing::warn!("[EVM] Collect ETH receipt timeout for {:?}: {}", addr, e),
                    }
                }
                Err(e) => tracing::warn!("[EVM] Collect ETH send failed for {:?}: {}", addr, e),
            }
        });
    }

    Ok(())
}
