use vm_data::{
    db::Database,
    models::streams::DeleteWalletsRpcPayload,
};
use crate::state::get_state;

pub async fn delete_wallets_req(
    db: &Database,
    _nc: &async_nats::Client,
    user_id: i32,
    payload: DeleteWalletsRpcPayload,
) -> Option<bool> {
    let state = get_state();
    let project = db.get_project(user_id, payload.project_id).await.unwrap_or(None)?;

    let (deleted, wallets) = db
        .delete_project_wallets(user_id, project.id, &payload.ids)
        .await
        .unwrap_or((false, vec![]));

    if deleted {
        // Update caches directly (no JetStream round-trip needed)
        for w in &wallets {
            state.untrack_address(project.user_id, project.id, w.address.clone());
            state.remove_wallet(&w.address);
        }

        // Notify Geyser service to refresh subscription
        state.refresh_geyser_subscription().await;
    }

    Some(deleted)
}
