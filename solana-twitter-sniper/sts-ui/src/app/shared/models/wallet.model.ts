export interface Wallet {
    id: number,
    address: string,
    balance: number,
    nonce_account_address: string | null
}

export interface WalletFull {
    id: number,
    user_id: number,
    address: string,
    name: string,
    comments: string | null,
    pk: string,
    nonce_account_address: string | null
}