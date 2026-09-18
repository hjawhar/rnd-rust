use vm_data::{
    db::Database,
    models::{
        streams::ViewWalletsRpcPayload,
        wallet::Wallet,
    },
    utils::encryption::decrypt,
};

pub async fn view_wallets_req(
    db: &Database,
    user_id: i32,
    payload: ViewWalletsRpcPayload,
) -> Option<Vec<Wallet>> {
    let project = db.get_project(user_id, payload.project_id).await.ok().flatten()?;

    let wallets = db
        .get_project_wallets_by_ids_full(user_id, project.id, &payload.ids)
        .await
        .unwrap_or_default();

    let mut returned_wallets: Vec<Wallet> = vec![];
    for wallet in wallets {
        if let Ok(pk) = decrypt(wallet.pk) {
            // Validate it's a valid EVM private key
            if vm_evm::helpers::str_to_pk(&pk).is_ok() {
                returned_wallets.push(Wallet { pk, ..wallet });
            }
        }
    }

    Some(returned_wallets)
}
