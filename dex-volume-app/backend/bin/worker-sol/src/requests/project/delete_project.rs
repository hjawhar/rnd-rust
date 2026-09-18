use vm_data::db::Database;

use crate::state::get_state;

pub async fn delete_project_req(
    db: &Database,
    _nc: &async_nats::Client,
    user_id: i32,
    project_id: i32,
) -> Option<bool> {
    let state = get_state();
    let project = db.get_project(user_id, project_id).await.unwrap_or(None)?;

    let wallets = db.get_project_wallets(user_id, project.id).await.unwrap_or_default();

    // Remove tracked addresses for each wallet
    for wallet in &wallets {
        state.untrack_address(project.user_id, project.id, wallet.address.clone());
    }

    // Remove tracked address for the project address itself
    state.untrack_address(project.user_id, project.id, project.address.clone());

    // Delete wallet caches
    for wallet in &wallets {
        state.remove_wallet(&wallet.address);
    }

    // Delete project from DB (cascades to wallets)
    let deleted = db.delete_project(&project.id).await.unwrap_or(false);

    // Notify Geyser service to refresh subscription
    state.refresh_geyser_subscription().await;

    Some(deleted)
}
