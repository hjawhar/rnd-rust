use vm_data::db::Database;
use vm_data::utils::encryption::decrypt;
use vm_data::models::{
    streams::{StreamInfo, StreamType},
    task::TaskDisperseTokens,
};
use vm_nats::subjects;

pub async fn disperse_tokens_req(
    db: &Database,
    js: &async_nats::jetstream::Context,
    user_id: i32,
    project_id: i32,
) -> Option<bool> {
    tracing::info!("[RPC] DISPERSE_TOKENS_REQ user_id={} project_id={}", user_id, project_id);

    let project = match db.get_project(user_id, project_id).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            tracing::warn!("[RPC] DISPERSE_TOKENS_REQ project not found user_id={} project_id={}", user_id, project_id);
            return None;
        }
        Err(e) => {
            tracing::error!("[RPC] DISPERSE_TOKENS_REQ db error: {}", e);
            return None;
        }
    };
    tracing::info!("[RPC] DISPERSE_TOKENS_REQ project found — token={} network={}", project.address, project.network);

    let all_wallets = db.get_project_wallets(user_id, project.id).await.unwrap_or_default();
    let main_wallet = match all_wallets.iter().find(|x| x.is_main).cloned() {
        Some(w) => w,
        None => {
            tracing::warn!("[RPC] DISPERSE_TOKENS_REQ no main wallet found for project_id={}", project_id);
            return None;
        }
    };
    let non_main: Vec<_> = all_wallets.iter().filter(|x| !x.is_main).cloned().collect();
    tracing::info!("[RPC] DISPERSE_TOKENS_REQ main_wallet={} non_main_count={}", main_wallet.address, non_main.len());

    if non_main.is_empty() {
        tracing::warn!("[RPC] DISPERSE_TOKENS_REQ no non-main wallets, skipping");
        return Some(false);
    }

    let decrypted_main_pk = match decrypt(main_wallet.pk) {
        Ok(pk) => pk,
        Err(e) => {
            tracing::error!("[RPC] DISPERSE_TOKENS_REQ failed to decrypt main wallet pk: {}", e);
            return Some(false);
        }
    };

    let recipients: Vec<String> = non_main.iter().map(|x| x.address.clone()).collect();
    tracing::info!("[RPC] DISPERSE_TOKENS_REQ publishing to JetStream — token={} recipients={:?}", project.address, recipients);

    let serialized = serde_json::to_vec(&StreamInfo {
        stream_type: StreamType::ProcessDisperseTokens(TaskDisperseTokens {
            token: project.address.clone(),
            sender_pk: decrypted_main_pk,
            recipients_pubkeys: recipients,
        }),
        user_id: project.user_id,
    })
    .unwrap_or_default();

    match js
        .publish(subjects::cmd::sol::DISPERSE_TOKENS, serialized.into())
        .await
    {
        Ok(_) => tracing::info!("[RPC] DISPERSE_TOKENS_REQ published successfully"),
        Err(e) => tracing::error!("[RPC] DISPERSE_TOKENS_REQ JetStream publish failed: {}", e),
    }

    Some(true)
}
