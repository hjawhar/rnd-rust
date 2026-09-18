use vm_data::db::Database;

use crate::cache;

pub async fn delete_project_req(
    db: &Database,
    _nc: &async_nats::Client,
    user_id: i32,
    project_id: i32,
) -> Option<bool> {
    let project = db.get_project(user_id, project_id).await.ok().flatten()?;
    let wallets = db.get_project_wallets(user_id, project.id).await.unwrap_or_default();

    // Remove tracked addresses for each wallet
    for wallet in &wallets {
        let _ = cache::remove_tracked_address(&wallet.address).await;
    }

    // Remove tracked address for the project address itself
    let _ = cache::remove_tracked_address(&project.address).await;

    // Delete wallet caches
    for wallet in &wallets {
        let _ = cache::delete_wallet(wallet.id).await;
    }

    // Delete project from DB (cascades to wallets)
    let deleted = db.delete_project(&project.id).await.unwrap_or(false);

    Some(deleted)
}
