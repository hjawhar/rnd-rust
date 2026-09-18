use std::{str::FromStr, sync::Arc};

use axsys::{models::AddWatchedProfilePayload, AxsysClient};
use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use models::broadcast::WsBroadcastGeneric;
use reqwest::StatusCode;
use serde_json::json;
use solana_pubkey::Pubkey;
use solana_sdk::{signature::Keypair, signer::Signer};

use crate::{
    models::{
        claims::Claims,
        state::AppState,
        tasks::{NewTask, Task, TaskMinimal, TaskWallet, UpdateTask, UpdateTaskPayload},
    },
    utils::{encryption::decrypt, helpers::f64_to_big_int},
};

pub async fn add_task_req(claims: Claims, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let user_id = claims.id;
    let new_task = NewTask {
        user_id,
        wallet_id: None,
        twitter_id: None,
        twitter_handle: None,
        servers: None,
        block_leaders: None,
        value: None,
        tip: None,
        slippage: 100,
        tries: 1,
        frontrunning_protection: false,
        enable_alerts: true,
        selected_pool: "EXCLUDE_PUMPFUN".to_string(),
        twitter_api: "EXTREME".to_string(),
        twitter_strategy: Some("TWEET".to_string()),
        twitter_handle_checker: None,
        twitter_token_override: None,
        words: None,
    };

    let task = state.get_db().add_task(&new_task).await;
    let payload: WsBroadcastGeneric<TaskWallet> = WsBroadcastGeneric {
        server: state.server_name.clone(),
        r#type: "ADD_TASK".to_string(),
        data: TaskWallet {
            id: task.id.clone(),
            user_id: task.user_id.clone(),
            private_key: None,
            public_key: None,
            nonce_account_address: None,
            twitter_id: task.twitter_id.clone(),
            twitter_handle: task.twitter_handle.clone(),
            servers: task.servers.clone(),
            block_leaders: task.block_leaders.clone(),
            value: task.value.clone(),
            tip: task.tip.clone(),
            slippage: task.slippage.clone(),
            tries: task.tries.clone(),
            frontrunning_protection: task.frontrunning_protection.clone(),
            enable_alerts: task.enable_alerts.clone(),
            selected_pool: task.selected_pool.clone(),
            twitter_api: task.twitter_api.clone(),
            twitter_strategy: task.twitter_strategy.clone(),
            twitter_handle_checker: task.twitter_handle_checker.clone(),
            twitter_token_override: task.twitter_token_override.clone(),
            words: task.words.clone(),
        },
    };
    let msg_string = serde_json::to_string(&payload).unwrap();
    let _ = state.tx_broadcast.send((msg_string, false));

    let body = Json(json!({
        "message": "Successfully added task",
        "data": task
    }));
    return (StatusCode::OK, body);
}

pub async fn get_tasks_req(
    _claims: Claims,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let tasks = state.get_db().get_tasks().await;
    let body = Json(json!({
        "data": tasks
    }));
    return (StatusCode::OK, body);
}

pub async fn get_task_req(
    _claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    match state.get_db().get_task(id).await {
        Some(task) => {
            let body = Json(json!({
                "data": task
            }));
            return (StatusCode::OK, body);
        }
        None => {
            let body = Json(json!({
                "error": "Error task not found"
            }));
            return (StatusCode::NOT_FOUND, body);
        }
    }
}

pub async fn update_task_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
    Json(payload): Json<UpdateTaskPayload>,
) -> impl IntoResponse {
    let current_logged_in_user = state.get_db().get_user_by_id(claims.id).await;
    let current_logged_in_user = match current_logged_in_user {
        Some(user) => user,
        None => {
            let body = Json(json!({
                "error": format!("User not logged in properly")
            }));
            return (StatusCode::BAD_REQUEST, body);
        }
    };
    let task = match state.get_db().get_task(id).await {
        Some(task) => task,
        None => {
            let body = Json(json!({
                "error": "Error task not found"
            }));
            return (StatusCode::NOT_FOUND, body);
        }
    };

    let mut private_key: Option<String> = None;
    let mut public_key: Option<String> = None;
    let mut nonce_account_address: Option<String> = None;

    let mut updated_task = UpdateTask {
        wallet_id: None,
        twitter_id: None,
        twitter_handle: None,
        servers: None,
        block_leaders: None,
        value: None,
        tip: None,
        slippage: payload.slippage,
        tries: payload.tries,
        frontrunning_protection: payload.frontrunning_protection,
        enable_alerts: payload.enable_alerts,
        selected_pool: payload.selected_pool,
        twitter_api: payload.twitter_api.clone(),
        twitter_strategy: payload.twitter_strategy,
        twitter_handle_checker: payload.twitter_handle_checker,
        twitter_token_override: None,
        words: payload.words,
    };

    if let Some(wallet_id) = payload.wallet_id {
        updated_task.wallet_id = Some(wallet_id);
        let wallet = state.get_db().get_wallet(&wallet_id).await;
        if let Some(wallet) = wallet {
            if let Some(task_wallet_id) = task.wallet_id {
                if wallet.id != task_wallet_id {
                    if current_logged_in_user.id != wallet.user_id
                        && current_logged_in_user.group_id == 3
                    {
                        let body = Json(json!({
                            "error": format!("Failed to assign wallet that doesn't belong to you")
                        }));
                        return (StatusCode::BAD_REQUEST, body);
                    }
                }
            }

            if let Ok(pk) = decrypt(wallet.pk.clone()) {
                private_key = Some(pk.clone());
                let encoded = bs58::decode(pk.clone()).into_vec();
                if let Ok(encoded) = encoded {
                    let kp = Keypair::from_bytes(&encoded[..]);
                    if let Ok(kp) = kp {
                        public_key = Some(kp.pubkey().to_string());
                        nonce_account_address = wallet.nonce_account_address.clone();
                    } else {
                        let body = Json(json!({
                            "error": format!("Failed to initiate keypair")
                        }));
                        return (StatusCode::BAD_REQUEST, body);
                    }
                } else {
                    let body = Json(json!({
                        "error": format!("Failed to decode private key")
                    }));
                    return (StatusCode::BAD_REQUEST, body);
                }
            } else {
                let body = Json(json!({
                    "error": format!("Failed to decrypt wallet")
                }));
                return (StatusCode::BAD_REQUEST, body);
            }
        } else {
            let body = Json(json!({
                "error": format!("Assigned wallet not found")
            }));
            return (StatusCode::BAD_REQUEST, body);
        }
    }

    if let Some(servers) = payload.servers {
        updated_task.servers = Some(servers);
    }

    if let Some(block_leaders) = payload.block_leaders {
        updated_task.block_leaders = Some(block_leaders);
    }

    if let Some(twitter_token_override) = payload.twitter_token_override {
        if let Ok(twitter_token_override) = Pubkey::from_str(&twitter_token_override) {
            updated_task.twitter_token_override = Some(twitter_token_override.to_string());
        }
    }

    updated_task.value = f64_to_big_int(payload.value);
    updated_task.tip = f64_to_big_int(payload.tip);

    let mut skip_tweet_handle_checking = false;
    if current_logged_in_user.id != task.user_id && current_logged_in_user.group_id == 3 {
        skip_tweet_handle_checking = true;
    }

    if !skip_tweet_handle_checking {
        if task.twitter_handle.is_some() && payload.twitter_handle.is_some() {
            if task.twitter_handle.eq(&payload.twitter_handle) {
                // tracing::info!("Matching twitter handles...");
                updated_task.twitter_handle = task.twitter_handle.clone();
                updated_task.twitter_id = task.twitter_id.clone();
            }

            if task.twitter_api.ne(&payload.twitter_api) {
                if let Some(twitter_id) = task.clone().twitter_id {
                    let tasks_with_same_tweet_handle_all = state
                        .get_db()
                        .get_tasks_with_tweet_handle(task.clone().twitter_handle.unwrap())
                        .await;

                    // tracing::info!("Twitter id: {twitter_id:#?}");
                    let tasks_with_same_tweet_handle_extreme: Vec<_> =
                        tasks_with_same_tweet_handle_all
                            .iter()
                            .filter(|x| x.twitter_api == "EXTREME")
                            .collect();

                    let tasks_with_same_tweet_handle_normal: Vec<_> =
                        tasks_with_same_tweet_handle_all
                            .iter()
                            .filter(|x| x.twitter_api == "NORMAL")
                            .collect();

                    let mut total_extreme = tasks_with_same_tweet_handle_extreme.len();
                    let mut total_normal = tasks_with_same_tweet_handle_normal.len();

                    if payload.twitter_api == "EXTREME" {
                        total_extreme += 1;
                        total_normal -= 1;
                    } else {
                        total_extreme -= 1;
                        total_normal += 1;
                    }

                    tracing::info!("Normal: {total_normal} - Extreme: {total_extreme}");

                    if total_extreme == 0 {
                        let deleted_watched_profile =
                            AxsysClient::delete_watched_profile(&state.axsys_api_key, &twitter_id)
                                .await;
                        match deleted_watched_profile {
                            Ok(deleted_watched_profile) => {
                                // tracing::info!(
                                //     "Successfully deleted extreme profile: {deleted_watched_profile:#?}"
                                // );
                            }
                            Err(err) => {
                                tracing::info!("Error deleting extreme profile: {err:#?}");
                            }
                        }
                    }

                    if total_normal == 0 {
                        let deleted_watched_profile = AxsysClient::delete_watched_profile(
                            &state.axsys_api_key_normal,
                            &twitter_id,
                        )
                        .await;
                        match deleted_watched_profile {
                            Ok(deleted_watched_profile) => {
                                //     tracing::info!(
                                //     "Successfully deleted normal profile: {deleted_watched_profile:#?}"
                                // );
                            }
                            Err(err) => {
                                tracing::info!("Error deleting normal profile: {err:#?}");
                            }
                        }
                    }
                }

                {
                    let tasks_with_same_tweet_handle_all = state
                        .get_db()
                        .get_tasks_with_tweet_handle(task.clone().twitter_handle.unwrap())
                        .await;

                    let tasks_with_same_tweet_handle_filtered: Vec<_> =
                        tasks_with_same_tweet_handle_all
                            .iter()
                            .filter(|x| x.twitter_api == payload.twitter_api)
                            .collect();

                    if tasks_with_same_tweet_handle_filtered.len() == 0 {
                        if let Some(twitter_handle) = payload.twitter_handle {
                            match AxsysClient::add_watched_profile(
                                if payload.twitter_api == "EXTREME".to_string() {
                                    &state.axsys_api_key
                                } else {
                                    &state.axsys_api_key_normal
                                },
                                AddWatchedProfilePayload {
                                    handle: twitter_handle.trim().to_string(),
                                },
                            )
                            .await
                            {
                                Ok(profile) => {
                                    if let Some(user) = profile.get("user") {
                                        let twitter_id =
                                            user.get("id").unwrap().as_str().unwrap().to_string();
                                        let twitter_handle = user
                                            .get("handle")
                                            .unwrap()
                                            .as_str()
                                            .unwrap()
                                            .to_string();
                                        updated_task.twitter_id = Some(twitter_id);
                                        updated_task.twitter_handle = Some(twitter_handle);
                                    }
                                }
                                Err(_err) => {
                                    let body = Json(json!({
                                        "error": "Failed to add twitter handle"
                                    }));
                                    return (StatusCode::BAD_REQUEST, body);
                                }
                            }
                        }
                    }
                }
            }
        } else if task.twitter_handle.is_some() && payload.twitter_handle.is_none() {
            let tasks_with_same_tweet_handle_all = state
                .get_db()
                .get_tasks_with_tweet_handle(task.clone().twitter_handle.unwrap())
                .await;

            let tasks_with_same_tweet_handle: Vec<_> = tasks_with_same_tweet_handle_all
                .iter()
                .filter(|x| x.twitter_api == task.twitter_api)
                .collect();

            if tasks_with_same_tweet_handle.len() == 1 {
                let deleted_watched_profile = AxsysClient::delete_watched_profile(
                    if task.twitter_api == "EXTREME".to_string() {
                        &state.axsys_api_key
                    } else {
                        &state.axsys_api_key_normal
                    },
                    &task.twitter_id.unwrap(),
                )
                .await;
                match deleted_watched_profile {
                    Ok(_deleted_watched_profile) => {
                        updated_task.twitter_id = None;
                        updated_task.twitter_handle = None;
                    }
                    Err(_err) => {
                        let body = Json(json!({
                            "error": "Failed to delete twitter handle"
                        }));
                        return (StatusCode::BAD_REQUEST, body);
                    }
                }
            }
        } else if task.twitter_handle.is_none() && payload.twitter_handle.is_some() {
            match AxsysClient::add_watched_profile(
                if payload.twitter_api == "EXTREME".to_string() {
                    &state.axsys_api_key
                } else {
                    &state.axsys_api_key_normal
                },
                AddWatchedProfilePayload {
                    handle: payload.twitter_handle.unwrap().trim().to_string(),
                },
            )
            .await
            {
                Ok(profile) => {
                    if let Some(user) = profile.get("user") {
                        let twitter_id = user.get("id").unwrap().as_str().unwrap().to_string();
                        let twitter_handle =
                            user.get("handle").unwrap().as_str().unwrap().to_string();
                        updated_task.twitter_id = Some(twitter_id);
                        updated_task.twitter_handle = Some(twitter_handle);
                    }
                }
                Err(err) => {
                    let body = Json(json!({
                        "error": format!("Failed to add twitter handle {:#?}", err)
                    }));
                    return (StatusCode::BAD_REQUEST, body);
                }
            }
        }
    }

    if current_logged_in_user.id != task.user_id && current_logged_in_user.group_id == 3 {
        if task.twitter_handle.clone().is_some() && updated_task.twitter_handle.is_some() {
            let current_twitter_handle = task.twitter_handle.clone().unwrap();
            let new_twitter_handle = updated_task.twitter_handle.clone().unwrap();
            if !new_twitter_handle.eq(&current_twitter_handle) {
                let body = Json(json!({
                    "error": format!("Cannot change twitter handle")
                }));
                return (StatusCode::BAD_REQUEST, body);
            }
        }

        if updated_task.tip.is_none() {
            let body = Json(json!({
                "error": format!("Cannot delete tip")
            }));
            return (StatusCode::BAD_REQUEST, body);
        }

        if updated_task.value.is_none() {
            let body = Json(json!({
                "error": format!("Cannot delete value")
            }));
            return (StatusCode::BAD_REQUEST, body);
        }

        if updated_task.tries == 0 {
            let body = Json(json!({
                "error": format!("Cannot set tries to 0")
            }));
            return (StatusCode::BAD_REQUEST, body);
        }

        if updated_task.twitter_handle.is_none() {
            let body = Json(json!({
                "error": format!("Cannot change tweet handle")
            }));
            return (StatusCode::BAD_REQUEST, body);
        }
        if task.tip.is_some() && updated_task.tip.is_some() {
            let current_tip = task.tip.clone().unwrap();
            let new_tip = updated_task.tip.clone().unwrap();
            if new_tip.gt(&current_tip) {
                let body = Json(json!({
                    "error": format!("Cannot increase tip")
                }));
                return (StatusCode::BAD_REQUEST, body);
            }
        }

        if task.value.is_some() && updated_task.value.is_some() {
            let current_value = task.value.clone().unwrap();
            let new_value = updated_task.value.clone().unwrap();
            if new_value.gt(&current_value) {
                let body = Json(json!({
                    "error": format!("Cannot increase value")
                }));
                return (StatusCode::BAD_REQUEST, body);
            }
        }

        if task.slippage != updated_task.slippage {
            let body = Json(json!({
                "error": format!("Cannot change slippage")
            }));
            return (StatusCode::BAD_REQUEST, body);
        }

        if task.tries != updated_task.tries {
            let body = Json(json!({
                "error": format!("Cannot change tries")
            }));
            return (StatusCode::BAD_REQUEST, body);
        }

        if task.frontrunning_protection != updated_task.frontrunning_protection {
            let body = Json(json!({
                "error": format!("Cannot change frontrunning protection")
            }));
            return (StatusCode::BAD_REQUEST, body);
        }

        if task.enable_alerts != updated_task.enable_alerts {
            let body = Json(json!({
                "error": format!("Cannot change enable alerts")
            }));
            return (StatusCode::BAD_REQUEST, body);
        }

        if task.twitter_api != updated_task.twitter_api {
            let body = Json(json!({
                "error": format!("Cannot change twitter token override")
            }));
            return (StatusCode::BAD_REQUEST, body);
        }

        if task.twitter_strategy != updated_task.twitter_strategy {
            let body = Json(json!({
                "error": format!("Cannot change twitter token override")
            }));
            return (StatusCode::BAD_REQUEST, body);
        }

        if task.twitter_token_override != updated_task.twitter_token_override {
            let body = Json(json!({
                "error": format!("Cannot change twitter token override")
            }));
            return (StatusCode::BAD_REQUEST, body);
        }

        if task.selected_pool != updated_task.selected_pool {
            let body = Json(json!({
                "error": format!("Cannot change selected pool")
            }));
            return (StatusCode::BAD_REQUEST, body);
        }

        if task.words.ne(&updated_task.words) {
            let body = Json(json!({
                "error": format!("Cannot change keywords")
            }));
            return (StatusCode::BAD_REQUEST, body);
        }
    }

    let _updated = state.get_db().update_task(&task.id, &updated_task).await;
    let task = state.get_db().get_task(task.id).await.unwrap();

    let payload: WsBroadcastGeneric<TaskWallet> = WsBroadcastGeneric {
        server: state.server_name.clone(),
        r#type: "UPDATE_TASK".to_string(),
        data: TaskWallet {
            id: task.id.clone(),
            user_id: task.user_id.clone(),
            private_key,
            public_key,
            nonce_account_address,
            twitter_id: task.twitter_id.clone(),
            twitter_handle: task.twitter_handle.clone(),
            servers: task.servers.clone(),
            block_leaders: task.block_leaders.clone(),
            value: task.value.clone(),
            tip: task.tip.clone(),
            slippage: task.slippage.clone(),
            tries: task.tries.clone(),
            frontrunning_protection: task.frontrunning_protection.clone(),
            enable_alerts: task.enable_alerts.clone(),
            selected_pool: task.selected_pool.clone(),
            twitter_api: task.twitter_api.clone(),
            twitter_strategy: task.twitter_strategy.clone(),
            twitter_handle_checker: task.twitter_handle_checker.clone(),
            twitter_token_override: task.twitter_token_override.clone(),
            words: task.words.clone(),
        },
    };
    let msg_string = serde_json::to_string(&payload).unwrap();
    let _ = state.tx_broadcast.send((msg_string, false));
    let task_minimal = TaskMinimal {
        id: task.id,
        user_id: task.user_id,
        wallet_id: task.wallet_id,
        twitter_id: task.clone().twitter_id,
        twitter_handle: task.clone().twitter_handle,
        servers: task.clone().servers,
        block_leaders: task.clone().block_leaders,
        value: task.clone().value,
        tip: task.clone().tip,
        slippage: task.slippage,
        tries: task.tries,
        frontrunning_protection: task.frontrunning_protection,
        enable_alerts: task.enable_alerts,
        selected_pool: task.selected_pool,
        twitter_api: task.twitter_api.clone(),
        twitter_strategy: task.twitter_strategy.clone(),
        twitter_handle_checker: task.twitter_handle_checker.clone(),
        twitter_token_override: task.twitter_token_override.clone(),
        words: task.words.clone(),
    };

    let body = Json(json!({
        "data": task_minimal,
        "message": "Successfully updated task"
    }));
    return (StatusCode::OK, body);
}

pub async fn delete_task_req(
    _claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    let task = match state.get_db().get_task(id).await {
        Some(task) => task,
        None => {
            let body = Json(json!({
                "error": "Task not found"
            }));
            return (StatusCode::NOT_FOUND, body);
        }
    };

    if let Some(twitter_id) = task.twitter_id {
        if let Some(twitter_handle) = task.twitter_handle.clone() {
            let tasks_with_same_tweet_handle_all = state
                .get_db()
                .get_tasks_with_tweet_handle(twitter_handle)
                .await;

            let tasks_with_same_tweet_handle_extreme: Vec<_> = tasks_with_same_tweet_handle_all
                .iter()
                .filter(|x| x.twitter_api == "EXTREME")
                .collect();

            let tasks_with_same_tweet_handle_normal: Vec<_> = tasks_with_same_tweet_handle_all
                .iter()
                .filter(|x| x.twitter_api == "NORMAL")
                .collect();

            if tasks_with_same_tweet_handle_extreme.len() == 1 {
                let deleted_watched_profile =
                    AxsysClient::delete_watched_profile(&state.axsys_api_key, &twitter_id).await;
                match deleted_watched_profile {
                    Ok(_deleted_watched_profile) => {}
                    Err(err) => {}
                }
            }

            if tasks_with_same_tweet_handle_normal.len() == 1 {
                let deleted_watched_profile =
                    AxsysClient::delete_watched_profile(&state.axsys_api_key_normal, &twitter_id)
                        .await;
                match deleted_watched_profile {
                    Ok(_deleted_watched_profile) => {}
                    Err(err) => {}
                }
            }
        };
    }

    let deleted_task = state.get_db().delete_task(&id).await;
    if deleted_task {
        let payload: WsBroadcastGeneric<i32> = WsBroadcastGeneric {
            server: state.server_name.clone(),
            r#type: "DELETE_TASK".to_string(),
            data: id,
        };
        let msg_string = serde_json::to_string(&payload).unwrap();
        let _ = state.tx_broadcast.send((msg_string, false));
        let body = Json(json!({
            "data": id,
            "message": "Successfully deleted task"
        }));
        return (StatusCode::OK, body);
    } else {
        let body = Json(json!({
            "error": "Error deleting task"
        }));
        return (StatusCode::BAD_REQUEST, body);
    }
}

pub async fn sync_tasks_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let tasks = state.get_db().get_tasks().await;
    let available_tweet_ids: Vec<_> = tasks
        .iter()
        .filter(|x| x.twitter_handle.clone().is_some() && x.twitter_id.clone().is_some())
        .map(|x| x.twitter_id.clone().unwrap().clone())
        .collect();
    {
        let watched_profiles = AxsysClient::get_watched_profiles(&state.axsys_api_key).await;
        match watched_profiles {
            Ok(watched_profiles) => {
                for (key, value) in watched_profiles.watched {
                    if let None = available_tweet_ids.iter().find(|x| **x == key) {
                        let user_id = claims.id;
                        let new_task = NewTask {
                            user_id,
                            wallet_id: None,
                            twitter_id: Some(key),
                            twitter_handle: Some(value),
                            servers: None,
                            block_leaders: None,
                            value: None,
                            tip: None,
                            slippage: 100,
                            tries: 1,
                            frontrunning_protection: true,
                            enable_alerts: true,
                            selected_pool: "EXCLUDE_PUMPFUN".to_string(),
                            twitter_api: "EXTREME".to_string(),
                            twitter_strategy: Some("TWEET".to_string()),
                            twitter_handle_checker: None,
                            twitter_token_override: None,
                            words: None,
                        };

                        let task = state.get_db().add_task(&new_task).await;
                        let payload: WsBroadcastGeneric<TaskWallet> = WsBroadcastGeneric {
                            server: state.server_name.clone(),
                            r#type: "ADD_TASK".to_string(),
                            data: TaskWallet {
                                id: task.id.clone(),
                                user_id: task.user_id.clone(),
                                private_key: None,
                                public_key: None,
                                nonce_account_address: None,
                                twitter_id: task.twitter_id.clone(),
                                twitter_handle: task.twitter_handle.clone(),
                                servers: task.servers.clone(),
                                block_leaders: task.block_leaders.clone(),
                                value: task.value.clone(),
                                tip: task.tip.clone(),
                                slippage: task.slippage.clone(),
                                tries: task.tries.clone(),
                                frontrunning_protection: task.frontrunning_protection.clone(),
                                enable_alerts: task.enable_alerts.clone(),
                                selected_pool: task.selected_pool.clone(),
                                twitter_api: task.twitter_api.clone(),
                                twitter_strategy: task.twitter_strategy.clone(),
                                twitter_handle_checker: task.twitter_handle_checker.clone(),
                                twitter_token_override: task.twitter_token_override.clone(),
                                words: task.words.clone(),
                            },
                        };
                        let msg_string = serde_json::to_string(&payload).unwrap();
                        let _ = state.tx_broadcast.send((msg_string, false));
                    }
                }
            }
            Err(err) => {
                let body = Json(json!({
                    "error": format!("Error fetching watched profiles {:#?}",err)
                }));
                return (StatusCode::BAD_REQUEST, body);
            }
        }
    }

    {
        let watched_profiles = AxsysClient::get_watched_profiles(&state.axsys_api_key_normal).await;
        match watched_profiles {
            Ok(watched_profiles) => {
                for (key, value) in watched_profiles.watched {
                    if let None = available_tweet_ids.iter().find(|x| **x == key) {
                        let user_id = claims.id;
                        let new_task = NewTask {
                            user_id,
                            wallet_id: None,
                            twitter_id: Some(key),
                            twitter_handle: Some(value),
                            servers: None,
                            block_leaders: None,
                            value: None,
                            tip: None,
                            slippage: 100,
                            tries: 1,
                            frontrunning_protection: true,
                            enable_alerts: true,
                            selected_pool: "EXCLUDE_PUMPFUN".to_string(),
                            twitter_api: "NORMAL".to_string(),
                            twitter_strategy: Some("TWEET".to_string()),
                            twitter_handle_checker: None,
                            twitter_token_override: None,
                            words: None,
                        };

                        let task = state.get_db().add_task(&new_task).await;
                        let payload: WsBroadcastGeneric<TaskWallet> = WsBroadcastGeneric {
                            server: state.server_name.clone(),
                            r#type: "ADD_TASK".to_string(),
                            data: TaskWallet {
                                id: task.id.clone(),
                                user_id: task.user_id.clone(),
                                private_key: None,
                                public_key: None,
                                nonce_account_address: None,
                                twitter_id: task.twitter_id.clone(),
                                twitter_handle: task.twitter_handle.clone(),
                                servers: task.servers.clone(),
                                block_leaders: task.block_leaders.clone(),
                                value: task.value.clone(),
                                tip: task.tip.clone(),
                                slippage: task.slippage.clone(),
                                tries: task.tries.clone(),
                                frontrunning_protection: task.frontrunning_protection.clone(),
                                enable_alerts: task.enable_alerts.clone(),
                                selected_pool: task.selected_pool.clone(),
                                twitter_api: task.twitter_api.clone(),
                                twitter_strategy: task.twitter_strategy.clone(),
                                twitter_handle_checker: task.twitter_handle_checker.clone(),
                                twitter_token_override: task.twitter_token_override.clone(),
                                words: task.words.clone(),
                            },
                        };
                        let msg_string = serde_json::to_string(&payload).unwrap();
                        let _ = state.tx_broadcast.send((msg_string, false));
                    }
                }
            }
            Err(err) => {
                let body = Json(json!({
                    "error": format!("Error fetching watched profiles {:#?}",err)
                }));
                return (StatusCode::BAD_REQUEST, body);
            }
        }
    }
    let tasks = state.get_db().get_tasks().await;
    let body = Json(json!({
        "data": tasks,
        "message": "Successfully synced tasks"
    }));
    return (StatusCode::OK, body);
}

pub async fn update_tasks_bots(state: Arc<AppState>) {
    let tasks = state.get_db().get_tasks().await;
    let wallets = state.get_db().get_all_wallets().await;

    let tasks: Vec<TaskWallet> = tasks
        .iter()
        .map(|task| {
            let mut private_key = None;
            let mut public_key = None;
            let mut nonce_account_address = None;
            if let Some(wallet_id) = task.wallet_id {
                let found_wallet = wallets.iter().find(|wallet| wallet.id == wallet_id);
                if let Some(wallet) = found_wallet {
                    if let Ok(pk) = decrypt(wallet.pk.clone()) {
                        private_key = Some(pk.clone());
                        let encoded = bs58::decode(pk.clone()).into_vec();
                        if let Ok(encoded) = encoded {
                            let kp = Keypair::from_bytes(&encoded[..]);
                            if let Ok(kp) = kp {
                                public_key = Some(kp.pubkey().to_string());
                                nonce_account_address = wallet.nonce_account_address.clone();
                            }
                        }
                    }
                }
            }

            return TaskWallet {
                id: task.id.clone(),
                user_id: task.user_id.clone(),
                private_key,
                nonce_account_address,
                public_key,
                twitter_id: task.twitter_id.clone(),
                twitter_handle: task.twitter_handle.clone(),
                servers: task.servers.clone(),
                block_leaders: task.block_leaders.clone(),
                value: task.value.clone(),
                tip: task.tip.clone(),
                slippage: task.slippage.clone(),
                tries: task.tries.clone(),
                frontrunning_protection: task.frontrunning_protection.clone(),
                enable_alerts: task.enable_alerts.clone(),
                selected_pool: task.selected_pool.clone(),
                twitter_api: task.twitter_api.clone(),
                twitter_strategy: task.twitter_strategy.clone(),
                twitter_handle_checker: task.twitter_handle_checker.clone(),
                twitter_token_override: task.twitter_token_override.clone(),
                words: task.words.clone(),
            };
        })
        .collect();

    let payload: WsBroadcastGeneric<Vec<TaskWallet>> = WsBroadcastGeneric {
        server: state.server_name.clone(),
        r#type: "SET_TASKS".to_string(),
        data: tasks.clone(),
    };
    let msg_string = serde_json::to_string(&payload).unwrap();
    let _ = state.tx_broadcast.send((msg_string, false));
}
