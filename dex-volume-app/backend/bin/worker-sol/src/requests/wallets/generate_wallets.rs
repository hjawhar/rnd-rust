use std::time::SystemTime;

use vm_data::{
    db::Database,
    models::{
        streams::GenerateWalletsPayload,
        wallet::{NewWallet, StoredWallet, Wallet},
    },
    utils::encryption::encrypt,
};
use solana_keypair::Keypair;
use solana_signer::Signer;

use crate::state::get_state;

pub async fn generate_wallets_req(
    db: &Database,
    _nc: &async_nats::Client,
    user_id: i32,
    payload: GenerateWalletsPayload,
) -> Option<Vec<Wallet>> {
    let state = get_state();
    let project = db.get_project(user_id, payload.project_id).await.unwrap_or(None)?;

    let mut new_wallets: Vec<NewWallet> = vec![];
    for _ in 1..=payload.count {
        let kp = Keypair::new();
        if let Ok(hash) = encrypt(kp.to_base58_string()) {
            new_wallets.push(NewWallet {
                pk: hash,
                address: kp.pubkey().to_string(),
                project_id: project.id,
                is_main: false,
                date_added: SystemTime::now(),
            });
        }
    }

    let wallets = db.add_wallets_batch(&new_wallets).await.unwrap_or_default();

    // Update caches for new wallets
    let token_program = state.fetch_token_program(&project.address).await
        .unwrap_or(spl_token::ID);
    for w in &wallets {
        state.track_address(project.user_id, project.id, w.address.clone(), project.address.clone(), &token_program);

        state.add_wallet(StoredWallet {
            user_id: project.user_id,
            wallet: w.clone(),
        });
    }

    // Notify Geyser service to refresh subscription
    if !wallets.is_empty() {
        state.refresh_geyser_subscription().await;
    }

    Some(wallets)
}
