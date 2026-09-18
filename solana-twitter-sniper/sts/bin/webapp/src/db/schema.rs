// @generated automatically by Diesel CLI.

diesel::table! {
    tasks (id) {
        id -> Int4,
        user_id -> Int4,
        wallet_id -> Nullable<Int4>,
        twitter_id -> Nullable<Text>,
        twitter_handle -> Nullable<Text>,
        servers -> Nullable<Text>,
        block_leaders -> Nullable<Text>,
        value -> Nullable<Numeric>,
        tip -> Nullable<Numeric>,
        slippage -> Int4,
        tries -> Int4,
        frontrunning_protection -> Bool,
        enable_alerts -> Bool,
        selected_pool -> Text,
        twitter_api -> Text,
        twitter_strategy -> Nullable<Text>,
        twitter_handle_checker -> Nullable<Text>,
        twitter_token_override -> Nullable<Text>,
        words -> Nullable<Text>,
    }
}

diesel::table! {
    users (id) {
        id -> Int4,
        group_id -> Int4,
        #[max_length = 42]
        address -> Varchar,
        nonce -> Text,
        whitelisted -> Bool,
        last_login_date_time -> Nullable<Timestamp>,
    }
}

diesel::table! {
    wallets (id) {
        id -> Int4,
        user_id -> Int4,
        #[max_length = 100]
        address -> Varchar,
        name -> Text,
        comments -> Nullable<Text>,
        pk -> Text,
        nonce_account_address -> Nullable<Text>,
    }
}

diesel::joinable!(tasks -> users (user_id));
diesel::joinable!(tasks -> wallets (wallet_id));
diesel::joinable!(wallets -> users (user_id));

diesel::allow_tables_to_appear_in_same_query!(
    tasks,
    users,
    wallets,
);
