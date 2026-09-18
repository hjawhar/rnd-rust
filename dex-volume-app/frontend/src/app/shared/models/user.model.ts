export type AddUserPayload = {
    address: string,
    nonce: number
}

export type User = {
    "address": string,
    "group_id": number,
    "id": number,
    "last_login_date_time": {
        "nanos_since_epoch": number,
        "secs_since_epoch": number
    } | null,
    "nonce": string,
    "whitelisted": boolean
}

export type ProjectAccess = {
    id: number,
    project_id: number,
    user_id: number,
    created_at: {
        nanos_since_epoch: number,
        secs_since_epoch: number
    }
}