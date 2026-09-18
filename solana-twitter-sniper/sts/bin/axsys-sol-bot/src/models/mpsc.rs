use super::{
    axsys::{TweetEvent, TwitterMiniTweet},
    buy::BuyRequest,
    monitor::MonitorTx,
};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use yellowstone_grpc_proto::prelude::{Message, Transaction};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MpscTaskTx {
    pub tx_hash: String,
    pub token: Pubkey,
}

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogsType {
    BUY,
    SELL,
    PUMPFUN_TOKEN_CREATION,
    PUMPFUN_RAYDIUM_MIGRATION,
    SPL_TOKEN_CREATION,
    IMAGE,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TxStatus {
    INIT,
    SENT,
    PENDING,
    CONFIRMED,
    UNCONFIRMED,
    FAILED,
    ERROR,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskLogs {
    pub logs_type: LogsType,
    pub tx_hash: Option<String>,
    pub token: Option<String>,
    pub bundle_hash: Option<String>,
    pub sender: Option<String>,
    pub text: String,
    pub confirmed: bool,
    pub status: TxStatus,
    pub timestamp: u128,
}

#[derive(Debug, Clone)]
pub struct GeyserTx {
    pub signature: Vec<u8>,
    pub message: Message,
    pub transaction: Transaction,
    pub index: u64,
    pub slot: u64,
    pub position: usize,
}

#[derive(Debug, Clone)]
pub struct GeyserProcessedTx {
    pub signature: String,
    pub slot: u64,
    pub success: bool,
}

#[derive(Debug, Clone)]
pub enum YellowstonSubscriptionType {
    PumpfunTokenCreation,
    PumpfunRaydiumMigration,
    TokenCreation,
    TxConfirmation,
    Ping,
    Slots,
}

#[derive(Debug, Clone)]
pub enum MpscTask {
    PumpfunTokenCreation(GeyserTx),
    PumpfunRaydiumMigration(GeyserTx),
    TxConfirmation(GeyserProcessedTx),
    TxSent(MonitorTx),
    TokenCreation(GeyserTx),
    Tweet(TweetEvent),
    Retry(BuyRequest),
    Slot(u64),
}

#[derive(Debug, Clone)]
pub enum GeyserTracker {
    Start(String),
    Stop(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TweetLogs {
    pub slot: u64,
    pub tweet: TwitterMiniTweet,
    pub timestamp: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MpscLogs {
    Ping(String),
    Pong(String),
    Tx(TaskLogs),
    Tweet(TweetLogs),
    Slot(u64),
}
