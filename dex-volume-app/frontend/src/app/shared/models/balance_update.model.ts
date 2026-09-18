
export enum BalanceType {
    USDC = "USDC",
    WSOL = "WSOL",
    TOKENS = "TOKENS",
    BASE = "BASE",
    QUOTE = "QUOTE",
}

export type WalletRelation = {
    user_id: number,
    project_id: number,
    owner: string,
    mint: string,
    address: String,
    balance_type: BalanceType,
}

export type BalanceUpdate = {
    balance: number,
    relation: WalletRelation,
}
