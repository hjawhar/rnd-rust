//! NATS subject constants for inter-service messaging.
//!
//! Naming convention:
//! - `vm.{type}.{chain}.{domain}.{action}` — Chain-scoped subjects
//!
//! Subject types:
//! - `rpc`      — Request-Reply (sync, queue group)
//! - `cmd`      — JetStream commands (fire-and-forget, WorkQueue)
//! - `events`   — Core Pub-Sub (real-time, all subscribers)
//! - `internal` — Internal coordination (all subscribers)

pub mod rpc {
    pub mod sol {
        pub const TOKEN_INFO: &str = "vm.rpc.sol.token.info";
        pub const POOL_FINANCIALS: &str = "vm.rpc.sol.pool.financials";
        pub const WALLETS_FINANCIALS: &str = "vm.rpc.sol.wallets.financials";
        pub const PROJECT_NEW: &str = "vm.rpc.sol.project.new";
        pub const PRICE: &str = "vm.rpc.sol.price";
        pub const PRICE_REFRESH: &str = "vm.rpc.sol.price.refresh";
        pub const PAIR_INFO: &str = "vm.rpc.sol.pair.info";
        pub const INIT_DATA: &str = "vm.rpc.sol.init.data";
        pub const DAILY_VOLUME: &str = "vm.rpc.sol.daily.volume";
        pub const WALLETS_IMPORT: &str = "vm.rpc.sol.wallets.import";
        pub const WALLETS_GENERATE: &str = "vm.rpc.sol.wallets.generate";
        pub const WALLETS_VIEW: &str = "vm.rpc.sol.wallets.view";
        pub const WALLETS_DELETE: &str = "vm.rpc.sol.wallets.delete";
        pub const PROJECT_START_TASK: &str = "vm.rpc.sol.project.start.task";
        pub const PROJECT_STOP_TASK: &str = "vm.rpc.sol.project.stop.task";
        pub const PROJECT_DELETE: &str = "vm.rpc.sol.project.delete";
        pub const PROJECT_COLLECT_SOL: &str = "vm.rpc.sol.project.collect.sol";
        pub const PROJECT_COLLECT_TOKENS: &str = "vm.rpc.sol.project.collect.tokens";
        pub const PROJECT_DISPERSE_SOL: &str = "vm.rpc.sol.project.disperse.sol";
        pub const PROJECT_DISPERSE_TOKENS: &str = "vm.rpc.sol.project.disperse.tokens";
        pub const VERIFY_SIGNATURE: &str = "vm.rpc.sol.verify.signature";
    }

    pub mod evm {
        pub const TOKEN_INFO: &str = "vm.rpc.evm.token.info";
        pub const POOL_FINANCIALS: &str = "vm.rpc.evm.pool.financials";
        pub const WALLETS_FINANCIALS: &str = "vm.rpc.evm.wallets.financials";
        pub const PROJECT_NEW: &str = "vm.rpc.evm.project.new";
        pub const PRICE: &str = "vm.rpc.evm.price";
        pub const PRICE_REFRESH: &str = "vm.rpc.evm.price.refresh";
        pub const INIT_DATA: &str = "vm.rpc.evm.init.data";
        pub const DAILY_VOLUME: &str = "vm.rpc.evm.daily.volume";
        pub const WALLETS_IMPORT: &str = "vm.rpc.evm.wallets.import";
        pub const WALLETS_GENERATE: &str = "vm.rpc.evm.wallets.generate";
        pub const WALLETS_VIEW: &str = "vm.rpc.evm.wallets.view";
        pub const WALLETS_DELETE: &str = "vm.rpc.evm.wallets.delete";
        pub const PROJECT_START_TASK: &str = "vm.rpc.evm.project.start.task";
        pub const PROJECT_STOP_TASK: &str = "vm.rpc.evm.project.stop.task";
        pub const PROJECT_DELETE: &str = "vm.rpc.evm.project.delete";
        pub const PROJECT_COLLECT_ETH: &str = "vm.rpc.evm.project.collect.eth";
        pub const PROJECT_COLLECT_TOKENS: &str = "vm.rpc.evm.project.collect.tokens";
        pub const PROJECT_DISPERSE_ETH: &str = "vm.rpc.evm.project.disperse.eth";
        pub const PROJECT_DISPERSE_TOKENS: &str = "vm.rpc.evm.project.disperse.tokens";
        pub const VERIFY_SIGNATURE: &str = "vm.rpc.evm.verify.signature";
    }
}

pub mod cmd {
    pub mod sol {
        pub const CLMM_FETCH: &str = "vm.cmd.sol.clmm.fetch";
        pub const TASK_START: &str = "vm.cmd.sol.task.start";
        pub const TASK_STOP: &str = "vm.cmd.sol.task.stop";
        pub const TRADE_BUY: &str = "vm.cmd.sol.trade.buy";
        pub const TRADE_SELL: &str = "vm.cmd.sol.trade.sell";
        pub const TRADE_BUNDLE_BUY_SELL: &str = "vm.cmd.sol.trade.bundle.buysell";
        pub const TRACK_TOKEN: &str = "vm.cmd.sol.track.token";
        pub const TRACK_ADDRESS: &str = "vm.cmd.sol.track.address";
        pub const TRACK_UNTRACK: &str = "vm.cmd.sol.track.untrack";
        pub const TRACK_DELETE: &str = "vm.cmd.sol.track.delete";
        pub const WALLET_STORE: &str = "vm.cmd.sol.wallet.store";
        pub const COLLECT_SOL: &str = "vm.cmd.sol.collect.sol";
        pub const COLLECT_TOKENS: &str = "vm.cmd.sol.collect.tokens";
        pub const DISPERSE_SOL: &str = "vm.cmd.sol.disperse.sol";
        pub const DISPERSE_TOKENS: &str = "vm.cmd.sol.disperse.tokens";

        /// Wildcard for JetStream stream subscription (all Solana commands)
        pub const ALL: &str = "vm.cmd.sol.>";
    }

    pub mod evm {
        pub const TASK_START: &str = "vm.cmd.evm.task.start";
        pub const TASK_STOP: &str = "vm.cmd.evm.task.stop";
        pub const TRADE_BUY: &str = "vm.cmd.evm.trade.buy";
        pub const TRADE_SELL: &str = "vm.cmd.evm.trade.sell";
        pub const TRADE_BUNDLE_BUY_SELL: &str = "vm.cmd.evm.trade.bundle.buysell";
        pub const TRACK_ADDRESS: &str = "vm.cmd.evm.track.address";
        pub const TRACK_UNTRACK: &str = "vm.cmd.evm.track.untrack";
        pub const TRACK_DELETE: &str = "vm.cmd.evm.track.delete";
        pub const WALLET_STORE: &str = "vm.cmd.evm.wallet.store";
        pub const COLLECT_ETH: &str = "vm.cmd.evm.collect.eth";
        pub const COLLECT_TOKENS: &str = "vm.cmd.evm.collect.tokens";
        pub const DISPERSE_ETH: &str = "vm.cmd.evm.disperse.eth";
        pub const DISPERSE_TOKENS: &str = "vm.cmd.evm.disperse.tokens";

        /// Wildcard for JetStream stream subscription (all EVM commands)
        pub const ALL: &str = "vm.cmd.evm.>";
    }
}

pub mod events {
    pub mod sol {
        pub const BALANCE_UPDATE: &str = "vm.events.sol.balance.update";
        pub const TRANSACTION_NEW: &str = "vm.events.sol.transaction.new";
        pub const PRICE: &str = "vm.events.sol.price";
        pub const DAILY_VOLUME: &str = "vm.events.sol.daily.volume";
        pub const TASK_STATUS: &str = "vm.events.sol.task.status";
        pub const WALLETS_FINANCIALS: &str = "vm.events.sol.wallets.financials";
    }

    pub mod evm {
        pub const BALANCE_UPDATE: &str = "vm.events.evm.balance.update";
        pub const TRANSACTION_NEW: &str = "vm.events.evm.transaction.new";
        pub const PRICE: &str = "vm.events.evm.price";
        pub const DAILY_VOLUME: &str = "vm.events.evm.daily.volume";
        pub const TASK_STATUS: &str = "vm.events.evm.task.status";
        pub const WALLETS_FINANCIALS: &str = "vm.events.evm.wallets.financials";
    }
}

pub mod discord {
    pub const MESSAGE: &str = "vm.events.discord.message";
}

pub mod system {
    pub const ALERT: &str = "vm.events.system.alert";
}

pub mod internal {
    pub mod evm {
        pub const WALLET_TRACKER: &str = "vm.internal.evm.wallet_tracker";
    }
}
