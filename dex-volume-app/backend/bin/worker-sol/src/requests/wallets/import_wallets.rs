use std::time::SystemTime;

use vm_data::{
    db::Database,
    models::{
        streams::ImportWalletsPayload,
        wallet::{NewWallet, StoredWallet, Wallet},
    },
    utils::encryption::encrypt,
};
use solana_signer::Signer;

use crate::state::get_state;

pub async fn import_wallets_req(
    db: &Database,
    _nc: &async_nats::Client,
    user_id: i32,
    payload: ImportWalletsPayload,
) -> Option<Vec<Wallet>> {
    let state = get_state();
    let project = db.get_project(user_id, payload.project_id).await.unwrap_or(None)?;

    let split_pks = payload.pks.trim().split(",");
    let mut new_wallets: Vec<NewWallet> = vec![];
    for pk in split_pks {
        let pk = pk.trim();
        if pk.is_empty() { continue; }
        let kp = vm_solana::utils::helpers::str_to_pk(pk.to_string());
        if let Ok(kp) = kp
            && let Ok(encrypted_pk) = encrypt(pk.to_string()) {
                let new_wallet = NewWallet {
                    pk: encrypted_pk,
                    address: kp.pubkey().to_string(),
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
