use alloy::sol;

sol!(
    #[allow(missing_docs)]
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    #[sol(rpc)]
    BaseUniswapV2Router,
    "src/contracts/base_uniswap_v2_router.json"
);

sol!(
    #[allow(missing_docs)]
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    #[sol(rpc)]
    BaseUniswapV3Router,
    "src/contracts/base_uniswap_v3_router.json"
);

sol!(
    #[allow(missing_docs)]
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    #[sol(rpc)]
    BaseUniswapV3Quoter,
    "src/contracts/base_uniswap_v3_quoter.json"
);

sol!(
    #[allow(missing_docs)]
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    #[sol(rpc)]
    interface ERC20Contract {
        function decimals() external view returns (uint256);
        function name() external view returns (string);
        function symbol() external view returns (string);
        function totalSupply() external view returns (uint256);
        function owner() public view returns (address);
        function balanceOf(address account) external view returns (uint256);
        function approve(address spender, uint value) external returns (bool);
        function allowance(address _owner, address spender) external view returns (uint256);
        function transfer(address recipient, uint256 amount) external returns (bool);
    }
);

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    BalanceChecker,
    "src/contracts/eth_balance_checker.json"
);

sol!(
    #[allow(missing_docs)]
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    #[sol(rpc)]
    FindPoolsContract,
    "src/contracts/eth_find_pools_simulator.json"
);

sol!(
    #[allow(missing_docs)]
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    #[sol(rpc)]
    contract EACAggregatorProxy {
        function latestAnswer() external view returns (int256);
    }
);

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    DisperseApp,
    "src/contracts/disperse_app.json"
);

sol!(
    #[allow(missing_docs)]
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    #[sol(rpc)]
    UniswapV4UniversalRouter,
    "src/contracts/uniswap_v4_universal_router.json"
);

sol!(
    #[allow(missing_docs)]
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    #[sol(rpc)]
    UniswapV4PositionManager,
    "src/contracts/uniswap_v4_position_manager.json"
);

sol!(
    #[allow(missing_docs)]
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    #[sol(rpc)]
    UniswapV4StateView,
    "src/contracts/uniswap_v4_state_view.json"
);

sol!(
    #[allow(missing_docs)]
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    #[sol(rpc)]
    Permit2,
    "src/contracts/permit2.json"
);

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    interface Multicall3Contract {
        struct Call3 {
            address target;
            bool allowFailure;
            bytes callData;
        }
        struct Result {
            bool success;
            bytes returnData;
        }
        function aggregate3(Call3[] calldata calls) external payable returns (Result[] memory returnData);
        function getEthBalance(address addr) external view returns (uint256 balance);
    }
);
