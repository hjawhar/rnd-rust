// @generated automatically by Diesel CLI.

diesel::table! {
    audit_logs (id) {
        id -> Int4,
        user_id -> Int4,
        project_id -> Nullable<Int4>,
        action -> Text,
        details -> Nullable<Jsonb>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    payments (id) {
        id -> Int4,
        subscription_id -> Int4,
        amount -> Numeric,
        currency -> Text,
        paid_at -> Timestamp,
        recorded_by -> Int4,
        notes -> Nullable<Text>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    project_access (id) {
        id -> Int4,
        project_id -> Int4,
        user_id -> Int4,
        created_at -> Timestamp,
    }
}

diesel::table! {
    projects (id) {
        id -> Int4,
        user_id -> Int4,
        address -> Text,
        pool -> Text,
        pool_type -> Text,
        trading_strategy -> Text,
        trading_interval -> Numeric,
        trading_daily_volume -> Numeric,
        name -> Nullable<Text>,
        symbol -> Nullable<Text>,
        description -> Nullable<Text>,
        image -> Nullable<Text>,
        date_added -> Timestamp,
        fees -> Numeric,
        network -> Text,
        decimals -> Nullable<Int4>,
        pair -> Nullable<Text>,
        max_market_impact_bps -> Nullable<Int4>,
        trade_multiplier -> Nullable<Float8>,
        status -> Text,
        slippage -> Nullable<Float8>,
        bundle_enabled -> Nullable<Bool>,
        jito_tip -> Nullable<Float8>,
        locked -> Bool,
        lock_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    subscriptions (id) {
        id -> Int4,
        project_id -> Int4,
        monthly_rate -> Numeric,
        currency -> Text,
        started_at -> Timestamp,
        next_payment_due -> Timestamp,
        status -> Text,
        notes -> Nullable<Text>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    transactions (id) {
        id -> Int4,
        project_id -> Int4,
        slot -> Int8,
        sol_price -> Numeric,
        value -> Numeric,
        tokens -> Numeric,
        address -> Text,
        tx_hash -> Text,
        tx_type -> Text,
        token_in -> Text,
        token_out -> Text,
        date_added -> Timestamp,
    }
}

diesel::table! {
    uniswap_v4_pools (id) {
        id -> Int4,
        network_id -> Int4,
        token_id -> Int8,
        pool_key -> Text,
        currency0 -> Text,
        currency1 -> Text,
        tick_spacing -> Text,
        fee -> Text,
        hooks -> Text,
    }
}

diesel::table! {
    users (id) {
        id -> Int4,
        group_id -> Int4,
        address -> Text,
        nonce -> Text,
        whitelisted -> Bool,
        date_added -> Timestamp,
        refresh_token -> Nullable<Text>,
        refresh_token_expires_at -> Nullable<Timestamp>,
        session_id -> Nullable<Text>,
    }
}

diesel::table! {
    wallets (id) {
        id -> Int4,
        project_id -> Int4,
        address -> Text,
        pk -> Text,
        is_main -> Bool,
        date_added -> Timestamp,
    }
}

diesel::joinable!(audit_logs -> users (user_id));
diesel::joinable!(payments -> subscriptions (subscription_id));
diesel::joinable!(project_access -> projects (project_id));
diesel::joinable!(project_access -> users (user_id));
diesel::joinable!(projects -> users (user_id));
diesel::joinable!(subscriptions -> projects (project_id));
diesel::joinable!(transactions -> projects (project_id));
diesel::joinable!(wallets -> projects (project_id));

diesel::allow_tables_to_appear_in_same_query!(
    audit_logs,
    payments,
    project_access,
    projects,
    subscriptions,
    transactions,
    uniswap_v4_pools,
    users,
    wallets,
);
