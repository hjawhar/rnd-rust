use std::collections::HashMap;

use alloy::rpc::types::TransactionRequest;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[allow(non_snake_case)]
pub struct EthCallManyPayloadParamsAccountOverrideState {
    pub balance: String,
}

#[allow(non_snake_case)]
pub type EthCallManyPayloadParamsAccountOverride =
    HashMap<String, EthCallManyPayloadParamsAccountOverrideState>;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[allow(non_snake_case)]
pub struct TraceCallManyTx {
    pub value: String,
    pub from: String,
    pub to: String,
    pub data: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SubTraceCallManyTx {
    Transactions(TraceCallManyTx),
    Type(Vec<String>),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[allow(non_snake_case)]
#[serde(untagged)]
pub enum TraceCallManyPayloadParams {
    Transactions(Vec<Vec<SubTraceCallManyTx>>),
    BlockNumber(String),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[allow(non_snake_case)]
pub struct TraceCallManyPayload {
    pub id: i32,
    pub jsonrpc: String,
    pub method: String,
    pub params: Vec<TraceCallManyPayloadParams>,
}

// trace results

#[derive(Serialize, Deserialize, Debug, Clone)]
#[allow(non_snake_case)]
pub struct TraceModelAction {
    pub from: String,
    pub gas: String,
    pub input: Option<String>,
    pub to: Option<String>,
    pub value: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[allow(non_snake_case)]
pub struct TraceModelResult {
    pub gasUsed: String,
    pub output: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[allow(non_snake_case)]
pub struct TraceModel {
    pub action: TraceModelAction,
    pub result: TraceModelResult,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[allow(non_snake_case)]
pub struct TraceCallManyResponseResult {
    pub output: String,
    pub trace: Vec<TraceModel>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[allow(non_snake_case)]
pub struct TraceCallManyResponse {
    pub id: i32,
    pub jsonrpc: String,
    pub result: Vec<TraceCallManyResponseResult>,
}

pub trait TxReqToEthCallManyTx {
    fn convert_trace(&self) -> TraceCallManyTx;
}

impl TxReqToEthCallManyTx for TransactionRequest {
    fn convert_trace(&self) -> TraceCallManyTx {
        TraceCallManyTx {
            value: format!("0x{:x}", self.value.unwrap()),
            from: self.from.unwrap().to_string(),
            to: self.to.unwrap().to().unwrap().to_string(),
            data: self.input.input.clone().unwrap().to_string(),
        }
    }
}
