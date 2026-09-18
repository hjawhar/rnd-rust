use alloy::primitives::{Address, address};
use alloy::providers::RootProvider;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Cached read-only HTTP providers per network — avoids creating a new provider per call.
static PROVIDERS: OnceLock<Mutex<HashMap<u64, RootProvider>>> = OnceLock::new();

fn providers_map() -> &'static Mutex<HashMap<u64, RootProvider>> {
    PROVIDERS.get_or_init(|| Mutex::new(HashMap::new()))
}

// Base
pub const BASE_CHAIN_ID: u64 = 8453;
pub const BASE_WETH: Address = address!("4200000000000000000000000000000000000006");
pub const BASE_USDC: Address = address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913");
pub const BASE_V2_ROUTER: Address = address!("4752ba5DBc23f44D87826276BF6Fd6b1C372aD24");
pub const BASE_V3_ROUTER: Address = address!("2626664c2603336E57B271c5C0b26F421741e481");
pub const BASE_V3_QUOTER: Address = address!("3d4e44Eb1374240CE5F1B871ab261CD16335B76a");
pub const BASE_UNIVERSAL_ROUTER: Address = address!("6fF5693b99212Da76ad316178A184AB56D299b43");
pub const BASE_POOL_FINDER: Address = address!("B8F0AF63a6a6054445B576D0Baf616DeB316423e");
pub const BASE_V4_STATE_VIEW: Address = address!("a3c0c9b65bad0b08107aa264b0f3db444b867a71");
pub const BASE_V4_POSITION_MANAGER: Address = address!("7c5f5a4bbd8fd63184577525326123b519429bdc");
pub const BASE_V4_QUOTER: Address = address!("0d5e0f971ed27fbff6c2837bf31316121532048d");
pub const BASE_BALANCE_CHECKER: Address = address!("ED0C5dCeA29dbBe3dD4B81D51946A2b76c288C0C");
pub const BASE_ERIGON_BALANCE_CHECKER: Address =
    address!("11f7c74830FC96e18B1b0722B80026D9CE37A5E2");

// Ethereum
pub const ETH_WETH: Address = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
pub const ETH_V2_ROUTER: Address = address!("7a250d5630B4cF539739dF2C5dAcb4c659F2488D");
pub const ETH_V3_ROUTER: Address = address!("E592427A0AEce92De3Edee1F18E0157C05861564");
pub const ETH_UNIVERSAL_ROUTER: Address = address!("66a9893cc07d91d95644aedd05d03f95e1dba8af");
pub const ETH_V4_STATE_VIEW: Address = address!("a3c0c9b65bad0b08107aa264b0f3db444b867a71");
pub const ETH_V4_POSITION_MANAGER: Address = address!("bd216513d74c8cf14cf4747e6aaa6420ff64ee9e");
pub const ETH_V4_QUOTER: Address = address!("52f0e24d1c21c8a0cb1e5a5dd6198556bd9e1203");
pub const ETH_ERIGON_BALANCE_CHECKER: Address =
    address!("7770BaD1d7F58D5e6EB1E84A61C4676AaF99f7Cf");
pub const ETH_USD_FEED: Address = address!("5f4eC3Df9cbd43714FE2740f5E3616155c5b8419");
pub const ETH_POOL_FINDER: Address = address!("723f603332A3Ed3e1fD565BC6E2046fcF43b368A");

// BSC
pub const BSC_WBNB: Address = address!("bb4CdB9CBd36B01bD1cBaEBF2De08d9173bc095c");
pub const BSC_V2_ROUTER: Address = address!("10ED43C718714eb63d5aA57B78B54704E256024E");

// Arbitrum
pub const ARB_WETH: Address = address!("82aF49447D8a07e3bd95BD0d56f35241523fBab1");
pub const ARB_V2_ROUTER: Address = address!("4752ba5DBc23f44D87826276BF6Fd6b1C372aD24");

// Avalanche
pub const AVAX_WAVAX: Address = address!("B31f66AA3C1e785363F0875A1B74E27b85FD66c7");
pub const AVAX_V2_ROUTER: Address = address!("60aE616a2155Ee3d9A68541Ba4544862310933d4");

// Shared
pub const DISPERSE_APP: Address = address!("D152f549545093347A162Dce210e7293f1452150");
pub const MULTICALL3: Address = address!("cA11bde05977b3631167028862bE2a173976CA11");
pub const PERMIT2: Address = address!("000000000022D473030F116dDEE9F6B43aC78BA3");

pub static MAX_UINT_HEX: &str =
    "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
pub static DEAD_ADDRESS: Address = address!("0000000000000000000000000000000000000000");

#[derive(Eq, Hash, PartialEq, Clone, Debug, Serialize, Deserialize)]
pub enum Network {
    Ethereum = 1,
    BSC = 56,
    Base = 8453,
    Arbitrum = 42161,
    Avalanche = 43114,
}

impl Network {
    pub fn from_chain_id(n: u64) -> Option<Network> {
        match n {
            1 => Some(Network::Ethereum),
            56 => Some(Network::BSC),
            8453 => Some(Network::Base),
            42161 => Some(Network::Arbitrum),
            43114 => Some(Network::Avalanche),
            _ => None,
        }
    }

    pub fn from_network_name(name: &str) -> Option<Network> {
        match name {
            "ethereum" => Some(Network::Ethereum),
            "bsc" => Some(Network::BSC),
            "base" => Some(Network::Base),
            "arbitrum" => Some(Network::Arbitrum),
            "avalanche" => Some(Network::Avalanche),
            _ => None,
        }
    }

    pub fn chain_id(&self) -> u64 {
        match self {
            Network::Ethereum => 1,
            Network::BSC => 56,
            Network::Base => 8453,
            Network::Arbitrum => 42161,
            Network::Avalanche => 43114,
        }
    }

    pub fn get_endpoint(&self) -> String {
        dotenv::dotenv().ok();
        match self {
            Network::Ethereum => std::env::var("ETH_HTTP").expect("ETH_HTTP is required"),
            Network::BSC => std::env::var("BSC_HTTP").expect("BSC_HTTP is required"),
            Network::Base => std::env::var("BASE_HTTP").expect("BASE_HTTP is required"),
            Network::Arbitrum => std::env::var("ARB_HTTP").expect("ARB_HTTP is required"),
            Network::Avalanche => std::env::var("AVAX_HTTP").expect("AVAX_HTTP is required"),
        }
    }

    pub fn get_base_token(&self) -> Address {
        match self {
            Network::Ethereum => ETH_WETH,
            Network::BSC => BSC_WBNB,
            Network::Base => BASE_WETH,
            Network::Arbitrum => ARB_WETH,
            Network::Avalanche => AVAX_WAVAX,
        }
    }

    pub fn get_v2_router(&self) -> Address {
        match self {
            Network::Ethereum => ETH_V2_ROUTER,
            Network::BSC => BSC_V2_ROUTER,
            Network::Base => BASE_V2_ROUTER,
            Network::Arbitrum => ARB_V2_ROUTER,
            Network::Avalanche => AVAX_V2_ROUTER,
        }
    }

    pub fn get_v3_router(&self) -> Address {
        match self {
            Network::Ethereum => ETH_V3_ROUTER,
            Network::Base => BASE_V3_ROUTER,
            _ => BASE_V3_ROUTER, // fallback
        }
    }

    /// Get a cached read-only HTTP provider for this network.
    /// First call per network creates the provider; subsequent calls return a clone (Arc-backed).
    pub fn get_provider(&self) -> RootProvider {
        let chain_id = self.chain_id();
        let map = providers_map();
        let mut guard = map.lock().unwrap();
        if let Some(provider) = guard.get(&chain_id) {
            return provider.clone();
        }
        let endpoint: reqwest::Url = self.get_endpoint().parse().expect("Invalid RPC endpoint URL");
        let provider = RootProvider::new_http(endpoint);
        guard.insert(chain_id, provider.clone());
        provider
    }

    pub fn name(&self) -> &'static str {
        match self {
            Network::Ethereum => "ethereum",
            Network::BSC => "bsc",
            Network::Base => "base",
            Network::Arbitrum => "arbitrum",
            Network::Avalanche => "avalanche",
        }
    }
}
