use crate::{
    cache::{
        price_cache::{add_sol_price, get_sol_price},
        volume_cache::get_daily_volume,
    },
    requests::tokens::token_info::get_token_info_req,
    state::get_state,
};
use vm_data::models::{
    solana::{PortfolioSummary, ProjectWalletInfo, WalletsFinancialsResponse},
    streams::{DailyVolumeInfo, StreamInfo, StreamType},
};
use vm_nats::subjects;
use vm_nats::subscribe_loop::spawn_subscribe_loop;
use vm_solana::utils::helpers::fetch_sol_price;
use vm_solana::{
    markets::generic::pools::get_pool,
    rpc::fetch_project_wallets::fetch_project_wallets_raw,
};
use serde::Serialize;
use std::future::Future;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

/// Spawns an RPC handler task that: queue_subscribes to `subject`, deserializes
/// `StreamInfo`, extracts the payload via `extract`, calls `handler`, wraps the
/// result via `wrap`, and publishes the reply.
pub fn rpc_handler<P, R, Fut, H>(
    nc: &async_nats::Client,
    qg: &str,
    subject: &'static str,
    extract: fn(StreamType) -> Option<P>,
    wrap: fn(R) -> StreamType,
    handler: H,
    shutdown: CancellationToken,
) -> JoinHandle<()>
where
    P: Send + 'static,
    R: Serialize + Send + 'static,
    Fut: Future<Output = R> + Send + 'static,
    H: Fn(async_nats::Client, i32, P) -> Fut + Send + Sync + Clone + 'static,
{
    spawn_subscribe_loop(nc, subject, qg, shutdown, None, move |nc, msg| {
        let handler = handler.clone();
        let span = {
            let trace_id = vm_nats::extract_trace_id(&msg);
            tracing::info_span!("rpc", subject = %subject, trace_id = trace_id.as_deref().unwrap_or("-"))
        };
        async move {
            if let Some(reply) = msg.reply {
                let si: StreamInfo = match serde_json::from_slice(&msg.payload) {
                    Ok(v) => v,
                    Err(_) => return,
                };
                let user_id = si.user_id;
                if let Some(payload) = extract(si.stream_type) {
                    let result = handler(nc.clone(), user_id, payload).await;
                    let response = wrap(result);
                    let bytes = serde_json::to_vec(&response).unwrap_or_default();
                    let _ = nc.publish(reply, bytes.into()).await;
                }
            }
        }
        .instrument(span)
    })
}

pub fn spawn_token_info(nc: &async_nats::Client, qg: &str, shutdown: CancellationToken) -> JoinHandle<()> {
    spawn_subscribe_loop(nc, subjects::rpc::sol::TOKEN_INFO, qg, shutdown, None, |nc, msg| async move {
        if let Some(reply) = msg.reply {
            let token = match serde_json::from_slice::<StreamType>(&msg.payload) {
                Ok(StreamType::RequestTokenInfo(payload)) => payload.token,
                _ => {
                    // Fallback: try raw string for backwards compat
                    String::from_utf8_lossy(&msg.payload).to_string()
                }
            };
            tracing::info!("[TOKEN_INFO] Request for token: {}", token);
            let result = get_token_info_req(token.clone()).await;
            let response = match result {
                Ok(info) => StreamType::ResponseTokenInfo(Some(info)),
                Err(e) => {
                    tracing::error!("[TOKEN_INFO] Failed for {}: {}", token, e);
                    StreamType::ResponseTokenInfo(None)
                }
            };
            let bytes = serde_json::to_vec(&response).unwrap_or_default();
            if let Err(e) = nc.publish(reply, bytes.into()).await {
                tracing::error!("[TOKEN_INFO] Failed to publish reply: {}", e);
            }
        }
    })
}

pub fn spawn_pair_info(nc: &async_nats::Client, qg: &str, shutdown: CancellationToken) -> JoinHandle<()> {
    spawn_subscribe_loop(nc, subjects::rpc::sol::PAIR_INFO, qg, shutdown, None, |_nc, msg| async move {
        let state = get_state();
        let pool = String::from_utf8_lossy(&msg.payload).to_string();
        let _ = state.fetch_pair(pool).await;
    })
}

pub fn spawn_pool_financials(nc: &async_nats::Client, qg: &str, shutdown: CancellationToken) -> JoinHandle<()> {
    spawn_subscribe_loop(nc, subjects::rpc::sol::POOL_FINANCIALS, qg, shutdown, None, |nc, msg| async move {
        if let Some(reply) = msg.reply {
            let pool = match serde_json::from_slice::<StreamInfo>(&msg.payload) {
                Ok(si) => match si.stream_type {
                    StreamType::RequestPoolFinancials(payload) => payload.pool,
                    _ => return,
                },
                Err(_) => return,
            };
            let state = get_state();
            let pool_info = get_pool(state.rpc.clone(), pool).await;
            let response = match pool_info {
                Ok(Some(info)) => {
                    StreamType::ResponsePoolFinancials(Some(
                        crate::requests::tokens::token_info::to_pool_info(info),
                    ))
                }
                _ => StreamType::ResponsePoolFinancials(None),
            };
            let bytes = serde_json::to_vec(&response).unwrap_or_default();
            let _ = nc.publish(reply, bytes.into()).await;
        }
    })
}

pub fn spawn_wallets_financials(nc: &async_nats::Client, qg: &str, shutdown: CancellationToken) -> JoinHandle<()> {
    spawn_subscribe_loop(nc, subjects::rpc::sol::WALLETS_FINANCIALS, qg, shutdown, None, |nc, msg| async move {
        if let Some(reply) = msg.reply {
            let si: StreamInfo = match serde_json::from_slice(&msg.payload) {
                Ok(v) => v,
                Err(_) => return,
            };
            if let StreamType::RequestWalletsFinancials((project, wallets, refresh_cache)) =
                si.stream_type
            {
                let state = get_state();
                let token_program = state.fetch_token_program(&project.address).await
                    .unwrap_or(spl_token::ID);
                let wallets_addresses =
                    wallets.iter().map(|x| x.address.clone()).collect();
                let wallets_info = fetch_project_wallets_raw(
                    state.rpc.clone(),
                    project.address.clone(),
                    wallets_addresses,
                    token_program,
                )
                .await;

                if refresh_cache {
                    // Write fresh balances back to cache so future
                    // WebSocket pushes (cache-only) use up-to-date values
                    let decimals = project.decimals.unwrap_or(6);
                    let decimals_factor = 10f64.powi(decimals);
                    let token_mint = solana_pubkey::Pubkey::from_str_const(&project.address);
                    for info in &wallets_info {
                        state.set_balance(info.address.clone(), info.sol);
                        let ata = spl_associated_token_account::get_associated_token_address_with_program_id(
                            &solana_pubkey::Pubkey::from_str_const(&info.address),
                            &token_mint,
                            &token_program,
                        );
                        state.set_balance(ata.to_string(), info.tokens * decimals_factor);
                    }
                }

                let sol_price =
                    get_sol_price().await.ok().flatten().unwrap_or(0.0);
                let token_decimals = project.decimals.unwrap_or(6) as u8;
                let token_price =
                    state.get_token_price(project.pool.clone(), token_decimals).await.unwrap_or(0.0);

                let mut wallets_financials: Vec<ProjectWalletInfo> = vec![];

                let token_price_usdc = token_price * sol_price;

                wallets_info.iter().for_each(|x| {
                    let found = wallets.iter().find(|w| w.address == x.address);
                    if let Some(found) = found {
                        let native_usdc = x.sol * sol_price;
                        let token_usdc = x.tokens * token_price_usdc;

                        wallets_financials.push(ProjectWalletInfo {
                            id: found.id,
                            main: found.is_main,
                            address: x.address.clone(),
                            native_balance: x.sol,
                            token_balance: x.tokens,
                            native_usdc,
                            token_usdc,
                            total_usdc: native_usdc + token_usdc,
                        });
                    }
                });

                let daily_vol = get_daily_volume(project.id).await
                    .ok()
                    .flatten()
                    .unwrap_or(0.0);
                let daily_target = vm_data::utils::helpers::big_int_to_f64(
                    project.trading_daily_volume.clone(),
                );

                let summary = PortfolioSummary::from_wallets(
                    &wallets_financials,
                    sol_price,
                    token_price_usdc,
                    daily_vol,
                    daily_target,
                );

                let response = StreamType::ResponseWalletsFinancials(
                    WalletsFinancialsResponse {
                        project_id: project.id,
                        wallets: wallets_financials,
                        summary,
                    },
                );
                let bytes = serde_json::to_vec(&response).unwrap_or_default();
                let _ = nc.publish(reply, bytes.into()).await;
            }
        }
    })
}

pub fn spawn_sol_price(nc: &async_nats::Client, qg: &str, shutdown: CancellationToken) -> JoinHandle<()> {
    spawn_subscribe_loop(nc, subjects::rpc::sol::PRICE, qg, shutdown, None, |nc, msg| async move {
        if let Some(reply) = msg.reply {
            let sol_price = get_sol_price().await;
            let sol_price = match sol_price {
                Ok(Some(p)) => p,
                _ => {
                    let fetched = fetch_sol_price().await;
                    if let Ok(p) = fetched {
                        let _ = add_sol_price(p).await;
                        p
                    } else {
                        tracing::error!("[SOL_PRICE] Failed to fetch: {:?}", fetched);
                        return;
                    }
                }
            };

            let response = StreamType::ResponsePrice(sol_price);
            let bytes = serde_json::to_vec(&response).unwrap_or_default();
            if let Err(e) = nc.publish(reply, bytes.into()).await {
                tracing::error!("[SOL_PRICE] Failed to publish reply: {}", e);
            }
        }
    })
}

pub fn spawn_sol_price_refresh(nc: &async_nats::Client, qg: &str, shutdown: CancellationToken) -> JoinHandle<()> {
    spawn_subscribe_loop(nc, subjects::rpc::sol::PRICE_REFRESH, qg, shutdown, None, |nc, msg| async move {
        let sol_price = fetch_sol_price().await;
        if let Ok(sol_price) = sol_price {
            let _ = add_sol_price(sol_price).await;

            // Reply if requested
            if let Some(reply) = msg.reply {
                let response = StreamType::ResponsePrice(sol_price);
                let bytes = serde_json::to_vec(&response).unwrap_or_default();
                if let Err(e) = nc.publish(reply, bytes.into()).await {
                    tracing::error!("[SOL_PRICE_REFRESH] Failed to publish reply: {}", e);
                }
            }

            // Broadcast to event subscribers (api-server WS)
            let broadcast = StreamInfo {
                user_id: -1,  // -1 means broadcast to all WebSocket clients
                stream_type: StreamType::ResponsePrice(sol_price),
            };
            let bytes = serde_json::to_vec(&broadcast).unwrap_or_default();
            if let Err(e) = nc.publish(subjects::events::sol::PRICE, bytes.into()).await {
                tracing::error!("[SOL_PRICE_REFRESH] Failed to broadcast: {}", e);
            }
        } else {
            tracing::error!("[SOL_PRICE_REFRESH] Failed to fetch price: {:?}", sol_price);
        }
    })
}

pub fn spawn_daily_volume(nc: &async_nats::Client, qg: &str, shutdown: CancellationToken) -> JoinHandle<()> {
    spawn_subscribe_loop(nc, subjects::rpc::sol::DAILY_VOLUME, qg, shutdown, None, |nc, msg| async move {
        if let Some(reply) = msg.reply {
            let stream_info: StreamInfo = match serde_json::from_slice(&msg.payload) {
                Ok(v) => v,
                Err(_) => return,
            };
            if let StreamType::RequestDailyVolume(project_id) = stream_info.stream_type {
                let state = get_state();
                let volume = get_daily_volume(project_id).await
                    .ok()
                    .flatten()
                    .unwrap_or(0.0);
                let target = state.get_task(project_id)
                    .map(|t| vm_data::utils::helpers::big_int_to_f64(
                        t.project.trading_daily_volume.clone(),
                    ))
                    .unwrap_or(0.0);
                let info = DailyVolumeInfo {
                    project_id,
                    volume_usdc: volume,
                    target_usdc: target,
                };
                let response = StreamType::ResponseDailyVolume(Some(info));
                let bytes = serde_json::to_vec(&response).unwrap_or_default();
                let _ = nc.publish(reply, bytes.into()).await;
            }
        }
    })
}

pub fn spawn_verify_signature(nc: &async_nats::Client, qg: &str, shutdown: CancellationToken) -> JoinHandle<()> {
    spawn_subscribe_loop(nc, subjects::rpc::sol::VERIFY_SIGNATURE, qg, shutdown, None, |nc, msg| async move {
        if let Some(reply) = msg.reply {
            let stream_type: StreamType = match serde_json::from_slice(&msg.payload) {
                Ok(v) => v,
                Err(_) => return,
            };
            if let StreamType::RequestVerifySignature(payload) = stream_type {
                let result = (|| -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
                    use std::str::FromStr;
                    let pubkey = solana_pubkey::Pubkey::from_str(&payload.user_pubkey)?;
                    let sig = solana_signature::Signature::from_str(&payload.signature)?;
                    Ok(sig.verify(&pubkey.to_bytes(), payload.message.as_bytes()))
                })();
                let verified = match result {
                    Ok(v) => Some(v),
                    Err(_) => Some(false),
                };
                let response = StreamType::ResponseVerifySignature(verified);
                let bytes = serde_json::to_vec(&response).unwrap_or_default();
                let _ = nc.publish(reply, bytes.into()).await;
            }
        }
    })
}
