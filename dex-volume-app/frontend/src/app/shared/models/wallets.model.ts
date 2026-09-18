export type ProjectWalletInfo = {
    "address": string,
    "id": number,
    "main": boolean,
    "native_balance": number,
    "native_usdc": number,
    "token_balance": number,
    "token_usdc": number,
    "total_usdc": number
}

export type ProjectWalletSummary = {
    "daily_volume_target_usdc": number
    "daily_volume_usdc": number,
    "native_price": number,
    "token_price": number,
    "token_price_usdc": number,
    "total_native": number,
    "total_native_usdc": number,
    "total_token_usdc": number,
    "total_tokens": number,
    "total_usdc": number
}

export type ProjectWalletsFinancials = {
    project_id: number,
    wallets: ProjectWalletInfo[],
    summary: ProjectWalletSummary
}