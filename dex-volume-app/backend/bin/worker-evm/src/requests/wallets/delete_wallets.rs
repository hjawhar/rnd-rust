use vm_data::{
    db::Database,
    models::streams::DeleteWalletsRpcPayload,
};

use crate::cache;

pub async fn delete_wallets_req(
    db: &Database,
    _nc: &async_nats::Client,
    user_id: i32,
    payload: DeleteWalletsRpcPayload,
) -> Option<bool> {
    let project = db.get_project(user_id, payload.project_id).await.ok().flatten()?;

    let (deleted, wallets) = db
        .delete_project_wallets(user_id, project.id, &payload.ids)
        .await
        .unwrap_or((false, vec![]));

    if deleted {
        for w in &wallets {
            let _ = cache::remove_tracked_address(&w.address).await;
            let _ = cache::delete_wallet(w.id).await;
        }
    }

    Some(deleted)
}
