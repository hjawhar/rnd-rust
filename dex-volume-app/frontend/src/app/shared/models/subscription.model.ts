export type Subscription = {
    id: number,
    project_id: number,
    monthly_rate: string,
    currency: string,
    started_at: {
        nanos_since_epoch: number,
        secs_since_epoch: number
    },
    next_payment_due: {
        nanos_since_epoch: number,
        secs_since_epoch: number
    },
    status: string,
    notes: string | null,
    created_at: {
        nanos_since_epoch: number,
        secs_since_epoch: number
    },
}

export type SubscriptionInfo = {
    monthly_rate: string,
    currency: string,
    next_payment_due: {
        nanos_since_epoch: number,
        secs_since_epoch: number
    },
    status: string,
    started_at: {
        nanos_since_epoch: number,
        secs_since_epoch: number
    },
    days_overdue: number,
    is_overdue: boolean,
}

export type SubscriptionWithProject = {
    subscription: Subscription,
    project: {
        id: number,
        name: string | null,
        symbol: string | null,
        network: string,
        user_id: number,
    },
    days_overdue: number,
    is_overdue: boolean,
}

export type Payment = {
    id: number,
    subscription_id: number,
    amount: string,
    currency: string,
    paid_at: {
        nanos_since_epoch: number,
        secs_since_epoch: number
    },
    recorded_by: number,
    notes: string | null,
    created_at: {
        nanos_since_epoch: number,
        secs_since_epoch: number
    },
}

export type CreateSubscriptionPayload = {
    monthly_rate: number,
    currency: string,
    next_payment_due: string,
    notes?: string,
}

export type UpdateSubscriptionPayload = {
    monthly_rate?: number,
    currency?: string,
    next_payment_due?: string,
    status?: string,
    notes?: string,
}

export type CreatePaymentPayload = {
    amount: number,
    currency: string,
    paid_at?: string,
    notes?: string,
}
