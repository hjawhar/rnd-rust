use bigdecimal::BigDecimal;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolInfo {
    pub name: String,
    pub pool_address: String,
    pub token0: String,
    pub token1: String,
    pub fee: f64,
    pub balance0: f64,
    pub balance1: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenMetadata {
    pub supply: u64,
    pub decimals: u8,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
}

use crate::models::{
    project::{NewProjectPayload, Project},
    solana::WalletsFinancialsResponse,
    task::{
        EvmProcessBundleBuySell, EvmProcessBuy, EvmProcessSell, EvmProcessTask,
        ProcessBundleBuySell, ProcessBuy, ProcessSell, ProcessTask, TaskCollectETH,
        TaskCollectEvmTokens, TaskCollectSOL, TaskCollectTokens, TaskDisperseETH,
        TaskDisperseEvmTokens, TaskDisperseSOL, TaskDisperseTokens,
    },
    transaction::{NewTransaction, Transaction},
    wallet::Wallet,
    wallet_relation::BalanceUpdate,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackTokenPayload {
    pub project_id: i32,
    pub user_id: i32,
    pub pool: String,
    pub mint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackAddressPayload {
    pub project_id: i32,
    pub user_id: i32,
    pub address: String,
    pub mint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UntrackAddressPayload {
    pub project_id: i32,
    pub user_id: i32,
    pub address: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoreWalletPayload {
    pub user_id: i32,
    pub wallet: Wallet,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportWalletsPayload {
    pub project_id: i32,
    pub pks: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenerateWalletsPayload {
    pub project_id: i32,
    pub count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ViewWalletsRpcPayload {
    pub project_id: i32,
    pub ids: Vec<i32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeleteWalletsRpcPayload {
    pub project_id: i32,
    pub ids: Vec<i32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerifySignaturePayload {
    pub user_pubkey: String,
    pub message: String,
    pub signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskStatusInfo {
    pub project_id: i32,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DailyVolumeInfo {
    pub project_id: i32,
    pub volume_usdc: f64,
    pub target_usdc: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InitDataPayload {
    pub projects: Vec<Project>,
    pub wallets_projects: Vec<(Wallet, Project)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenInfoPayload {
    pub token: String,
    pub network: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolFinancialsPayload {
    pub pool: String,
    pub token: Option<String>,
    pub network: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum StreamType {
    // ── Shared project operations (both SOL and EVM use these) ────────────
    RequestNewProject(NewProjectPayload),
    ResponseNewProject(Option<Project>),

    RequestTokenInfo(TokenInfoPayload),
    ResponseTokenInfo(Option<Vec<PoolInfo>>),

    RequestPoolFinancials(PoolFinancialsPayload),
    ResponsePoolFinancials(Option<PoolInfo>),

    TrackToken(TrackTokenPayload),
    TrackAddresses(Vec<TrackAddressPayload>),
    StoreWallets(Vec<StoreWalletPayload>),

    UntrackAddresses(Vec<UntrackAddressPayload>),
    DeleteAddresses(Vec<String>),

    /// Bool flag: true = write fresh balances back to cache (hard refresh)
    RequestWalletsFinancials((Project, Vec<Wallet>, bool)),
    ResponseWalletsFinancials(WalletsFinancialsResponse),

    // ── Shared events ────────────────────────────────────────────────────
    BalanceUpdate(BalanceUpdate),
    NewTransactionRequest(NewTransaction),
    NewTransactionsRequest(Vec<NewTransaction>),
    NewTransactionResponse(Transaction),
    UpdateTransactionSlot {
        tx_hash: String,
        slot: i64,
        value: Option<BigDecimal>,
        tokens: Option<BigDecimal>,
    },
    UpdateTransactionResponse(Transaction),

    // ── Shared task control ──────────────────────────────────────────────
    StopTask(i32),

    // ── Solana trading (chain-specific payload types) ────────────────────
    StartTask(ProcessTask),
    Sell(ProcessSell),
    Buy(ProcessBuy),
    BundleBuySell(ProcessBundleBuySell),

    // ── Shared price operations ──────────────────────────────────────────
    ResponsePrice(f64),

    // ── Solana collect/disperse (chain-specific payload types) ────────────
    ProcessCollectSOL(TaskCollectSOL),
    ProcessCollectTokens(TaskCollectTokens),
    ProcessDisperseSOL(TaskDisperseSOL),
    ProcessDisperseTokens(TaskDisperseTokens),

    // ── Shared init ──────────────────────────────────────────────────────
    RequestInitData,
    ResponseInitData(InitDataPayload),

    // ── Shared daily volume ──────────────────────────────────────────────
    RequestDailyVolume(i32),
    ResponseDailyVolume(Option<DailyVolumeInfo>),
    DailyVolumeUpdate(DailyVolumeInfo),

    // ── Shared wallet operations ─────────────────────────────────────────
    RequestImportWallets(ImportWalletsPayload),
    ResponseImportWallets(Option<Vec<Wallet>>),

    RequestGenerateWallets(GenerateWalletsPayload),
    ResponseGenerateWallets(Option<Vec<Wallet>>),

    RequestViewWallets(ViewWalletsRpcPayload),
    ResponseViewWallets(Option<Vec<Wallet>>),

    RequestDeleteWallets(DeleteWalletsRpcPayload),
    ResponseDeleteWallets(Option<bool>),

    // ── Shared project RPC operations ────────────────────────────────────
    RequestStartTask(i32),
    ResponseStartTask(Option<bool>),

    RequestStopTask(i32),
    ResponseStopTask(Option<bool>),

    RequestDeleteProject(i32),
    ResponseDeleteProject(Option<bool>),

    RequestCollectNative(i32),
    ResponseCollectNative(Option<bool>),

    RequestCollectTokens(i32),
    ResponseCollectTokens(Option<bool>),

    RequestDisperseNative(i32),
    ResponseDisperseNative(Option<bool>),

    RequestDisperseTokens(i32),
    ResponseDisperseTokens(Option<bool>),

    // ── Shared verify signature ──────────────────────────────────────────
    RequestVerifySignature(VerifySignaturePayload),
    ResponseVerifySignature(Option<bool>),

    // ── EVM trading (chain-specific payload types) ───────────────────────
    EvmStartTask(EvmProcessTask),
    EvmBuy(EvmProcessBuy),
    EvmSell(EvmProcessSell),
    EvmBundleBuySell(EvmProcessBundleBuySell),

    // ── EVM collect/disperse (chain-specific payload types) ──────────────
    ProcessCollectETH(TaskCollectETH),
    ProcessCollectEvmTokens(TaskCollectEvmTokens),
    ProcessDisperseETH(TaskDisperseETH),
    ProcessDisperseEvmTokens(TaskDisperseEvmTokens),

    // ── Trade confirmed (volume tracking on tx confirmation) ─────────────
    TradeConfirmed { project_id: i32, volume_usdc: f64 },

    // ── Task status updates (running/stopped) ────────────────────────────
    TaskStatusUpdate(TaskStatusInfo),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamInfo {
    pub user_id: i32,
    pub stream_type: StreamType,
}
