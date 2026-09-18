use std::error::Error;

use alloy::rpc::types::TransactionRequest;
use serde_json::json;

use crate::constants::Network;
use crate::models::simulation::{
    SubTraceCallManyTx, TraceCallManyPayload, TraceCallManyPayloadParams, TraceCallManyResponse,
    TxReqToEthCallManyTx,
};

pub async fn trace_txs(
    network: Network,
    params: Vec<TransactionRequest>,
    block_number: Option<u64>,
) -> Result<TraceCallManyResponse, Box<dyn Error + Send + Sync>> {
    dotenv::dotenv().ok();
    let endpoint = network.get_endpoint();
    let mut txs: Vec<Vec<SubTraceCallManyTx>> = vec![];
    for param in params {
        let build_txs = vec![
            SubTraceCallManyTx::Transactions(param.convert_trace()),
            SubTraceCallManyTx::Type(vec!["trace".to_string()]),
        ];
        txs.push(build_txs);
    }

    let params: Vec<TraceCallManyPayloadParams> = vec![
        TraceCallManyPayloadParams::Transactions(txs),
        TraceCallManyPayloadParams::BlockNumber(if let Some(block_number) = block_number {
            format!("0x{:x}", block_number)
        } else {
            "latest".to_string()
        }),
    ];

    let payload = TraceCallManyPayload {
        id: 1,
        jsonrpc: "2.0".to_string(),
        method: "trace_callMany".to_string(),
        params,
    };

    let client = reqwest::Client::new();
    let resp = client.post(endpoint).json(&json!(payload)).send().await?;
    let response = resp.text().await?;
    let response_json = serde_json::from_str::<TraceCallManyResponse>(response.as_str())?;
    Ok(response_json)
}
