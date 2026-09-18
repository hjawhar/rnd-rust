use vm_data::utils::helpers::f64_to_big_int;
use vm_data::models::{
    streams::{DailyVolumeInfo, StreamType, PoolFinancialsPayload},
    transaction::{TransactionSummaryBuy, TransactionSummaryOverall, TransactionSummarySell},
};
use bigdecimal::BigDecimal;
use vm_nats::subjects;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use axum::{
    Json,
    extract::{Path, Query, State},
    response::IntoResponse,
};
use reqwest::StatusCode;
use serde_json::json;
use crate::models::state::AppState;
use crate::routes::middleware::Claims;
use crate::routes::is_evm_network;
use crate::routes::helpers::nats_rpc;

#[derive(Deserialize)]
pub struct Pagination {
    page: i32,
    limit: i32,
}

#[derive(Deserialize)]
pub struct StatisticsQuery {
    limit: String,
}

pub async fn get_project_wallets_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let refresh_cache = params.get("refresh").map(|v| v == "true").unwrap_or(false);
    // Fetch project and wallets in parallel — both include auth checks
    let (project_result, wallets) = tokio::join!(
        state.read_db().get_project(claims.id, id),
        state.read_db().get_project_wallets(claims.id, id),
    );

    let project = match project_result.ok().flatten() {
        Some(project) => project,
        None => {
            let body = Json(json!({
                "error": "Error project not found"
            }));
            return (StatusCode::NOT_FOUND, body);
        }
    };

    let wallets = wallets.unwrap_or_default();

    let network = project.network.clone();
    let st = match nats_rpc(
        &state.nats_client, &network,
        subjects::rpc::sol::WALLETS_FINANCIALS, subjects::rpc::evm::WALLETS_FINANCIALS,
        claims.id, StreamType::RequestWalletsFinancials((project, wallets, refresh_cache)),
        Duration::from_secs(5),
    ).await {
        Ok(st) => st,
        Err(e) => return e,
    };
    match st {
        StreamType::ResponseWalletsFinancials(res) => (StatusCode::OK, Json(json!({ "data": res }))),
        _ => (StatusCode::OK, Json(json!({ "data": null }))),
    }
}

pub async fn get_project_financials_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    let project = match state.read_db().get_project(claims.id, id).await.ok().flatten() {
        Some(project) => project,
        None => {
            let body = Json(json!({
                "error": "Error project not found"
            }));
            return (StatusCode::NOT_FOUND, body);
        }
    };

    let request_type = if is_evm_network(&project.network) {
        StreamType::RequestPoolFinancials(PoolFinancialsPayload {
            pool: project.pool.clone(),
            token: Some(project.address.clone()),
            network: Some(project.network.clone()),
        })
    } else {
        StreamType::RequestPoolFinancials(PoolFinancialsPayload {
            pool: project.pool.clone(),
            token: None,
            network: None,
        })
    };
    let st = match nats_rpc(
        &state.nats_client, &project.network,
        subjects::rpc::sol::POOL_FINANCIALS, subjects::rpc::evm::POOL_FINANCIALS,
        claims.id, request_type,
        Duration::from_secs(5),
    ).await {
        Ok(st) => st,
        Err(e) => return e,
    };
    match st {
        StreamType::ResponsePoolFinancials(info) => (StatusCode::OK, Json(json!({ "data": info }))),
        _ => (StatusCode::OK, Json(json!({ "data": null }))),
    }
}

pub async fn get_project_transactions_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
    Query(pagination): Query<Pagination>,
) -> impl IntoResponse {
    let project = match state.read_db().get_project(claims.id, id).await.ok().flatten() {
        Some(project) => project,
        None => {
            let body = Json(json!({
                "error": "Error project not found"
            }));
            return (StatusCode::NOT_FOUND, body);
        }
    };
    if pagination.limit < 0 || pagination.limit > 100 || pagination.page < 0 {
        let body = Json(json!({
            "error": "Wrong pagination parameters"
        }));
        return (StatusCode::NOT_FOUND, body);
    }
    let (count, transactions) = tokio::join!(
        state.read_db().get_transactions_count(project.id),
        state.read_db().get_transactions(project.id, pagination.page, pagination.limit),
    );
    let count = count.unwrap_or(0);
    let mut transactions = transactions.unwrap_or_default();
    // Strip :buy/:sell suffixes from bundle tx hashes before sending to clients
    for tx in &mut transactions {
        tx.tx_hash = crate::streams::strip_tx_hash_suffix(&tx.tx_hash);
    }

    let body = Json(json!({
        "data": {
            "page": pagination.page,
            "limit": pagination.limit,
            "count": count,
            "transactions": transactions
        }
    }));
    (StatusCode::OK, body)
}

pub async fn get_project_statistics_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
    Query(limit): Query<StatisticsQuery>,
) -> impl IntoResponse {
    let project = match state.read_db().get_project(claims.id, id).await.ok().flatten() {
        Some(project) => project,
        None => {
            let body = Json(json!({
                "error": "Error project not found"
            }));
            return (StatusCode::NOT_FOUND, body);
        }
    };

    if limit.limit != "all" {
        let interval: String;
        if limit.limit == "5m" {
            interval = "5 minutes".to_string();
        } else if limit.limit == "1hr" {
            interval = "1 hour ".to_string();
        } else if limit.limit == "6hr" {
            interval = "6 hours".to_string();
        } else if limit.limit == "24hr" {
            interval = "1 day".to_string();
        } else {
            let body = Json(json!({
                "error": "Wrong interval"
            }));
            return (StatusCode::NOT_FOUND, body);
        }
        let (res, daily_volume) = tokio::join!(
            state.read_db().get_project_statistics_overall_interval(project.id, interval),
            fetch_daily_volume(&state, claims.id, project.id, &project.network),
        );
        let res = res.unwrap_or_else(|_| (
            TransactionSummaryOverall { value: BigDecimal::from(0), total: BigDecimal::from(0) },
            TransactionSummaryBuy { value: BigDecimal::from(0), tokens: BigDecimal::from(0), total: BigDecimal::from(0) },
            TransactionSummarySell { value: BigDecimal::from(0), tokens: BigDecimal::from(0), total: BigDecimal::from(0) },
        ));
        let daily_volume = daily_volume
            .unwrap_or(DailyVolumeInfo { project_id: project.id, volume_usdc: 0.0, target_usdc: 0.0 });
        let body = Json(json!({
            "data": {
                "buy": {
                    "native": res.1.value,
                    "tokens": res.1.tokens,
                    "usdc": res.1.total,
                },
                "sell": {
                    "native": res.2.value,
                    "tokens": res.2.tokens,
                    "usdc": res.2.total,
                },
                "total": {
                    "native": res.0.value,
                    "usdc": res.0.total
                },
                "daily_volume": daily_volume
            }
        }));
        (StatusCode::OK, body)
    } else {
        let (overall, buy, sell, daily_volume) = tokio::join!(
            state.read_db().get_project_statistics_overall(project.id),
            state.read_db().get_project_statistics_buy(project.id),
            state.read_db().get_project_statistics_sell(project.id),
            fetch_daily_volume(&state, claims.id, project.id, &project.network),
        );
        let daily_volume = daily_volume
            .unwrap_or(DailyVolumeInfo { project_id: project.id, volume_usdc: 0.0, target_usdc: 0.0 });

        let (native_total, usdc_total) = overall.unwrap_or((None, None));
        let native_total = native_total.unwrap_or(f64_to_big_int(0.0));
        let usdc_total = usdc_total.unwrap_or(f64_to_big_int(0.0));

        let (buy_value, buy_tokens, buy_usdc) = buy.unwrap_or((None, None, None));
        let buy_native = buy_value.unwrap_or(f64_to_big_int(0.0));
        let buy_tokens = buy_tokens.unwrap_or(f64_to_big_int(0.0));
        let buy_usdc = buy_usdc.unwrap_or(f64_to_big_int(0.0));

        let (sell_value, sell_tokens, sell_usdc) = sell.unwrap_or((None, None, None));
        let sell_native = sell_value.unwrap_or(f64_to_big_int(0.0));
        let sell_tokens = sell_tokens.unwrap_or(f64_to_big_int(0.0));
        let sell_usdc = sell_usdc.unwrap_or(f64_to_big_int(0.0));
        let body = Json(json!({
            "data": {
                "buy": {
                    "native": buy_native,
                    "tokens": buy_tokens,
                    "usdc": buy_usdc,
                },
                "sell": {
                    "native": sell_native,
                    "tokens": sell_tokens,
                    "usdc": sell_usdc,
                },
                "total": {
                    "native": native_total,
                    "usdc": usdc_total
                },
                "daily_volume": daily_volume
            }
        }));
        (StatusCode::OK, body)
    }
}

/// Fetch daily volume info from chain worker via NATS RPC.
/// Returns None on failure (non-fatal).
async fn fetch_daily_volume(state: &Arc<AppState>, user_id: i32, project_id: i32, network: &str) -> Option<DailyVolumeInfo> {
    let st = nats_rpc(
        &state.nats_client, network,
        subjects::rpc::sol::DAILY_VOLUME, subjects::rpc::evm::DAILY_VOLUME,
        user_id, StreamType::RequestDailyVolume(project_id),
        Duration::from_secs(3),
    ).await.ok()?;
    match st {
        StreamType::ResponseDailyVolume(info) => info,
        _ => None,
    }

}