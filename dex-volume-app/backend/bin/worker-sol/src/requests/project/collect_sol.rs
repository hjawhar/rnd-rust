use vm_data::db::Database;
use vm_data::utils::encryption::decrypt;
use vm_data::models::{
    streams::{StreamInfo, StreamType},
    task::TaskCollectSOL,
};
use vm_nats::subjects;

pub async fn collect_sol_req(
    db: &Database,
    js: &async_nats::jetstream::Context,
    user_id: i32,
    project_id: i32,
) -> Option<bool> {
    let project = db.get_project(user_id, project_id).await.unwrap_or(None)?;

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

    let mut decrypted_pks = vec![];
    for wallet in non_main {
        if let Ok(decrypted) = decrypt(wallet.pk) {
            decrypted_pks.push(decrypted);
        }
    }

    let serialized = serde_json::to_vec(&StreamInfo {
        stream_type: StreamType::ProcessCollectSOL(TaskCollectSOL {
            senders_pks: decrypted_pks,
            recipient_pk: decrypted_main_pk,
        }),
        user_id: project.user_id,
    })
    .unwrap_or_default();

    let _ = js
        .publish(subjects::cmd::sol::COLLECT_SOL, serialized.into())
        .await;

    Some(true)
}
