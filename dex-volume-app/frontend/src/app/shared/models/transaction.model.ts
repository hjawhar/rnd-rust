export type Transaction = {
    id: number,
    project_id: number,
    slot: number,
    sol_price: number,
    value: number,
    tokens: number,
    address: string,
    tx_hash: string,
    tx_type: string,
    token_in: string,
    token_out: string,
    date_added: { secs_since_epoch: number },
}

export type ProjectTransactions = {
    page: number,
    limit: number,
    count: number,
    transactions: Transaction[]
}