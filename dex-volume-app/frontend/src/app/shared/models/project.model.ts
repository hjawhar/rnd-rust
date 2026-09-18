export type AddProjectPayload = {
    address: string,
    pool: string,
    trading_strategy: string,
    network: string
}

export type Project = {
    "address": string,
    "date_added": {
        "nanos_since_epoch": number,
        "secs_since_epoch": number
    },
    "decimals": string | null,
    "description": string,
    "fees": string,
    "id": number,
    "image": string,
    "max_market_impact_bps": string | null,
    "name": string,
    "network": string,
    "pair": string | null,
    "pool": string,
    "pool_type": string,
    "status": string,
    "symbol": string,
    "trade_multiplier": number | null,
    "slippage": number | null,
    "bundle_enabled": boolean | null,
    "jito_tip": number | null,
    "locked": boolean,
    "lock_at": { nanos_since_epoch: number, secs_since_epoch: number } | null,
    "trading_daily_volume": number,
    "trading_interval": number,
    "trading_strategy": string,
    "user_id": number,
}

export type UpdateProject = {
    trading_interval: number | null,
    trading_daily_volume: number | null,
    max_market_impact_bps: number | null,
    trade_multiplier: number | null,
    slippage: number | null,
    bundle_enabled: boolean | null,
    jito_tip: number | null
}

export type DailyVolume = {
    project_id: number,
    target_usdc: number,
    volume_usdc: number
}

export type ProjectStatistics = {
    "buy": {
        "native": number,
        "tokens": number,
        "usdc": number
    },
    "daily_volume": {
        "project_id": number,
        "target_usdc": number,
        "volume_usdc": number
    },
    "sell": {
        "native": number,
        "tokens": number,
        "usdc": number
    },
    "total": {
        "native": number,
        "usdc": number
    }
}