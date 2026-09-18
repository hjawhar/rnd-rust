use std::time::SystemTime;

use vm_data::{
    db::Database,
    models::{
        streams::ImportWalletsPayload,
        wallet::{NewWallet, StoredWallet, Wallet},
    },
    utils::encryption::encrypt,
};

use crate::cache;

pub async fn import_wallets_req(
    db: &Database,
    _nc: &async_nats::Client,
    user_id: i32,
    payload: ImportWalletsPayload,
) -> Option<Vec<Wallet>> {
    let project = db.get_project(user_id, payload.project_id).await.ok().flatten()?;

    let split_pks = payload.pks.trim().split(",");
    let mut new_wallets: Vec<NewWallet> = vec![];
    for pk in split_pks {
        let pk = pk.trim();
        if let Ok(signer) = vm_evm::helpers::str_to_pk(pk)
            && let Ok(encrypted_pk) = encrypt(pk.to_string()) {
                let new_wallet = NewWallet {
                    pk: encrypted_pk,
                    address: signer.address().to_string(),
                    project_id: project.id,
                    is_main: false,
                    date_added: SystemTime::now(),
                };
                new_wallets.push(new_wallet);
            }
    }

    let wallet_addresses = new_wallets
        .iter()
        .map(|x| x.address.clone())
        .collect::<Vec<String>>();
    let db_wallets = db
        .get_project_wallet_by_addresses(user_id, project.id, &wallet_addresses)
        .await
        .unwrap_or_default();

    let filtered: Vec<NewWallet> = new_wallets
        .into_iter()
        .filter(|nw| !db_wallets.iter().any(|dw| dw.address == nw.address))
        .collect();

    let wallets = db.add_wallets_batch(&filtered).await.unwrap_or_default();

    // Update caches for new wallets
    for w in &wallets {
        let _ = cache::track_address(
            project.user_id,
            project.id,
            w.address.clone(),
            project.address.clone(),
        ).await;

        let _ = cache::add_wallet(StoredWallet {
            user_id: project.user_id,
            wallet: w.clone(),
        }).await;
    }

    Some(wallets)
}
