use jupiter_swap_api_client::swap::SwapInstructionsResponse;
use jupiter_swap_api_client::ClientError;
use jupiter_swap_api_client::{
    quote::QuoteRequest, swap::SwapRequest, transaction_config::TransactionConfig,
    JupiterSwapApiClient,
};
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;
pub async fn get_swap_instruction(
    endpoint: String,
    api_key: String,
    mint: Pubkey,
    sender: Pubkey,
    amount: u64,
    slippage_bps: u16,
) -> Result<SwapInstructionsResponse, ClientError> {
    tracing::info!("Fetching swap instructions");
    const NATIVE_MINT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");
    let jupiter_swap_api_client = JupiterSwapApiClient::new(endpoint, api_key);

    let quote_request = QuoteRequest {
        amount,
        input_mint: NATIVE_MINT,
        output_mint: mint,
        slippage_bps,
        ..QuoteRequest::default()
    };

    let quote_response = jupiter_swap_api_client.quote(&quote_request).await?;

    let swap_instructions = jupiter_swap_api_client
        .swap_instructions(&SwapRequest {
            user_public_key: sender,
            quote_response,
            config: TransactionConfig::default(),
        })
        .await?;
    tracing::info!("Done fetching swap instructions");

    return Ok(swap_instructions);
}
