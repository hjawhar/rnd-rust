use vm_data::db::Database;
use vm_data::models::{
    streams::{StreamInfo, StreamType},
    task::TaskDisperseEvmTokens,
};
use vm_data::utils::encryption::decrypt;
use vm_nats::subjects;

pub async fn disperse_tokens_req(
    db: &Database,
    nc: &async_nats::Client,
    user_id: i32,
    project_id: i32,
) -> Option<bool> {
    let project = db.get_project(user_id, project_id).await.ok().flatten()?;

    let all_wallets = db.get_project_wallets(user_id, project.id).await.unwrap_or_default();
    let main_wallet = all_wallets.iter().find(|x| x.is_main).cloned()?;
    let non_main: Vec<_> = all_wallets.iter().filter(|x| !x.is_main).cloned().collect();

    if non_main.is_empty() {
        return Some(false);
    }

    let decrypted_main_pk = match decrypt(main_wallet.pk) {
        Ok(pk) => pk,
        Err(_) => return Some(false),
    };

    let serialized = serde_json::to_vec(&StreamInfo {
        stream_type: StreamType::ProcessDisperseEvmTokens(TaskDisperseEvmTokens {
            project_id: project.id,
            user_id: project.user_id,
            network: project.network.clone(),
            decimals: project.decimals.unwrap_or(18),
            token: project.address.clone(),
            sender_pk: decrypted_main_pk,
            recipients_pubkeys: non_main.iter().map(|x| x.address.clone()).collect(),
        }),
        user_id: project.user_id,
    }).unwrap_or_default();

    let _ = nc
        .publish(subjects::cmd::evm::DISPERSE_TOKENS, serialized.into())
        .await;

    Some(true)
}
