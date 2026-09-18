use crate::constants::{ETH_USD_FEED, Network};
use crate::contracts::EACAggregatorProxy;
use std::error::Error;

pub async fn fetch_eth_price(network: &Network) -> Result<f64, Box<dyn Error + Send + Sync>> {
    let provider = network.get_provider();

    let contract = EACAggregatorProxy::new(ETH_USD_FEED, &provider);
    let result = contract.latestAnswer().call().await?;

    let price = result.as_i64() as f64 / 1e8;
    Ok(price)
}
