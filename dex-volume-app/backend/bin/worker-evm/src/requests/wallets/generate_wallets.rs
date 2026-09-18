use std::time::SystemTime;

use alloy::signers::local::PrivateKeySigner;
use vm_data::{
    db::Database,
    models::{
        streams::GenerateWalletsPayload,
        wallet::{NewWallet, StoredWallet, Wallet},
    },
    utils::encryption::encrypt,
};

use crate::cache;

pub async fn generate_wallets_req(
    db: &Database,
    _nc: &async_nats::Client,
    user_id: i32,
    payload: GenerateWalletsPayload,
) -> Option<Vec<Wallet>> {
    let project = db.get_project(user_id, payload.project_id).await.ok().flatten()?;

    let mut new_wallets: Vec<NewWallet> = vec![];
    for _ in 1..=payload.count {
        let signer = PrivateKeySigner::random();
        let pk_hex = hex::encode(signer.credential().to_bytes());
        if let Ok(hash) = encrypt(pk_hex) {
            new_wallets.push(NewWallet {
                pk: hash,
                address: signer.address().to_string(),
                project_id: project.id,
                is_main: false,
                date_added: SystemTime::now(),
            });
        }
    }

    let wallets = db.add_wallets_batch(&new_wallets).await.unwrap_or_default();

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
