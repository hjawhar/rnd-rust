import { LogsType } from "./logs.enum"

export type BuyRequestPayload = {
    mint_address: string,
    wallet_id: number,
    servers: string,
    block_leaders: string,
    value: number,
    tip: number,
    slippage: number,
    tries: number,
    frontrunning_protection: boolean,
    enable_alerts: boolean,
    selected_pool: string
}

export enum TxStatus {
    INIT = 'INIT',
    SENT = 'SENT',
    PENDING = 'PENDING',
    CONFIRMED = 'CONFIRMED',
    UNCONFIRMED = 'UNCONFIRMED',
    FAILED = 'FAILED',
    ERROR = 'ERROR',
}

export type TaskLogs = {
    logs_type: LogsType,
    tx_hash: string | null,
    token: string | null,
    bundle_hash: string | null,
    sender: string | null,
    text: string,
    confirmed: boolean,
    status: TxStatus,
    timestamp: number,
}