use std::time::Duration;

use alloy::providers::{Provider, ProviderBuilder};
use anyhow::{Context, Result, anyhow, ensure};

/// Checks `eth_chainId` with an unsigned HTTP provider.
pub async fn verify_rpc_chain(rpc_url: &str) -> Result<u64> {
    let url: url::Url = rpc_url.parse().context("RPC_URL is not a valid URL")?;
    ensure!(
        matches!(url.scheme(), "http" | "https") && url.has_host(),
        "RPC_URL must be an HTTP(S) URL"
    );
    let provider = ProviderBuilder::new()
        .disable_recommended_fillers()
        .connect_http(url);
    let chain_id = tokio::time::timeout(Duration::from_secs(15), provider.get_chain_id())
        .await
        .context("RPC chain ID request timed out")?
        .map_err(|_| anyhow!("could not query RPC chain ID"))?;
    ensure!(
        chain_id == crate::CHAIN_ID,
        "RPC chain id mismatch: expected {}, got {chain_id}",
        crate::CHAIN_ID
    );
    Ok(chain_id)
}
