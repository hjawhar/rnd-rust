# solana-rpc-tester

A Rust tool for testing Solana RPC endpoint performance. Sends transactions on a timed interval via SWQoS or Jito bundles, then tracks confirmation latency through a Yellowstone/Geyser gRPC subscription.

## How It Works

1. Connects to a Geyser gRPC endpoint and subscribes to slots, transactions, and account updates for your wallet.
2. Every 5 seconds, builds a self-transfer transaction (sends SOL from your wallet back to itself).
3. Routes the transaction through either:
   - **Jito bundles** (private transactions with a tip) -- when `private_tx = true`
   - **SWQoS** (Stake-Weighted Quality of Service with priority fee) -- when `private_tx = false`
4. Logs when each transaction is confirmed on-chain, showing submission slot vs. confirmation slot.

## Prerequisites

- Rust 1.75+ (edition 2024)
- A funded Solana wallet (mainnet)
- Access to:
  - A Solana RPC endpoint
  - A SWQoS endpoint
  - A Yellowstone/Geyser gRPC endpoint with token
  - (Optional) Jito block engine API key

## Setup

1. Clone the repository:
   ```bash
   git clone https://github.com/hjawhar/solana-rpc-tester.git
   cd solana-rpc-tester
   ```

2. Copy the environment template and fill in your credentials:
   ```bash
   cp .env.example .env
   ```

3. Edit `.env` with your values:
   ```
   RPC_ENDPOINT="https://your-rpc-endpoint"
   SWQOS_ENDPOINT="https://your-swqos-endpoint"
   GEYSER_ENDPOINT="https://your-geyser-endpoint"
   GEYSER_TOKEN="your-geyser-token"
   pk="your-base58-private-key"

   JITO_API_ENDPOINT="https://frankfurt.mainnet.block-engine.jito.wtf/api/v1/bundles"
   JITO_API_KEY="your-jito-api-key"
   ```

4. Build and run:
   ```bash
   cargo build --release
   cargo run --release
   ```

## Configuration

| Variable | Required | Description |
|---|---|---|
| `RPC_ENDPOINT` | Yes | Solana RPC URL for blockhash queries |
| `SWQOS_ENDPOINT` | Yes | SWQoS endpoint for public transaction submission |
| `GEYSER_ENDPOINT` | Yes | Yellowstone gRPC endpoint for transaction tracking |
| `GEYSER_TOKEN` | Yes | Authentication token for the Geyser endpoint |
| `pk` | Yes | Base58-encoded wallet private key |
| `JITO_API_ENDPOINT` | For Jito | Jito block engine bundle API URL |
| `JITO_API_KEY` | For Jito | Jito API authentication key |

## Transaction Modes

### Private (Jito Bundles)

Set `private_tx = true` in `main.rs`. Transactions include a Jito tip and are sent as bundles through the Jito block engine. This provides MEV protection and priority landing.

### Public (SWQoS)

Set `private_tx = false` in `main.rs`. Transactions include a compute unit price (priority fee) and are sent through the SWQoS endpoint.

## License

MIT -- see [LICENSE](LICENSE) for details.
