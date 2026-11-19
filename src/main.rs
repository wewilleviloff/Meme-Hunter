use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use tokio::sync::{RwLock, mpsc};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use meme_hunter::{
    config::AppConfig,
    crius::{CriusClient, CriusEvent, CriusStream, PriceSubscription},
    scanner::engine::{EngineControl, ScannerEngine},
    storage::Store,
    telegram,
};

fn initialize_tls() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("failed to install rustls ring provider: already initialized"))
}

#[tokio::main]
async fn main() -> Result<()> {
    initialize_tls()?;
    dotenvy::dotenv().ok();
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("diagnose-quote") => return meme_hunter::diagnostics::run_quote(&args[1..]).await,
        Some(other) => anyhow::bail!(
            "unknown command: {other}; use diagnose-quote or no arguments for PAPER mode"
        ),
        None => {}
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = AppConfig::from_env().context("configuration invalid")?;
    if config.bot_mode != "PAPER" {
        anyhow::bail!("LIVE mode is deliberately disabled in this Rust MVP. Set BOT_MODE=PAPER.");
    }

    let store = Store::connect(&config.database_url).await?;
    let crius = CriusClient::new(config.crius_rest_base.clone(), config.crius_api_key.clone())?;
    let control = Arc::new(RwLock::new(EngineControl::default()));

    anyhow::ensure!(
        config.crius_api_key.is_some(),
        "CRIUS_API_KEY is required and must include the stream scope"
    );
    store.reset_abandoned_screenings().await?;
    let (event_tx, event_rx) = mpsc::channel::<CriusEvent>(512);
    let (subscribe_tx, subscribe_rx) = mpsc::unbounded_channel();
    restore_price_subscriptions(&store, &subscribe_tx).await?;

    let engine = ScannerEngine::new(
        config.clone(),
        store.clone(),
        crius.clone(),
        subscribe_tx,
        control.clone(),
    );
    let engine_task = tokio::spawn(async move {
        if let Err(err) = engine.run(event_rx).await {
            error!(error = %err, "scanner engine stopped");
        }
    });

    replay_graduations(&store, &event_tx).await?;
    let stream = CriusStream::new(config.crius_ws_url.clone(), config.crius_api_key.clone());
    let stream_event_tx = event_tx.clone();
    tokio::spawn(async move {
        if let Err(err) = stream.run(stream_event_tx, subscribe_rx).await {
            error!(error = %err, "Crius stream stopped");
        }
    });

    // Price events arrive on Crius channels when a token trades. This REST poll is a
    // fallback for open paper positions, not a promise of a timely executable exit.
    let poll_store = store.clone();
    let poll_crius = crius.clone();
    let poll_tx = event_tx.clone();
    let poll_seconds = config.price_poll_seconds.max(5);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(poll_seconds));
        loop {
            ticker.tick().await;
            match poll_store.open_position_addresses().await {
                Ok(addresses) => {
                    for address in addresses {
                        match poll_crius.get_price(&address).await {
                            Ok(price) => {
                                if let (Some(usd), Some(ts_ms)) = (price.usd, price.block_ts_ms) {
                                    let _ = poll_tx
                                        .send(CriusEvent::Price {
                                            token: address,
                                            usd,
                                            liquidity_usd: price.liquidity_usd,
                                            ts_ms,
                                        })
                                        .await;
                                }
                            }
                            Err(err) => {
                                warn!(token = %address, error = %err, "price fallback poll failed")
                            }
                        }
                    }
                }
                Err(err) => warn!(error = %err, "could not load open positions for price polling"),
            }
        }
    });

    if let (Some(token), Some(allowed_id)) = (
        config.telegram_bot_token.clone(),
        config.telegram_allowed_user_id,
    ) {
        let telegram_store = store.clone();
        let telegram_control = control.clone();
        let take_profit = config.take_profit_multiple;
        let stop_loss = config.stop_loss_pct;
        tokio::spawn(async move {
            if let Err(err) = telegram::run(
                token,
                allowed_id,
                telegram_store,
                telegram_control,
                take_profit,
                stop_loss,
            )
            .await
            {
                error!(error = %err, "Telegram bot stopped");
            }
        });
    } else {
        warn!("Telegram disabled: set both TELEGRAM_BOT_TOKEN and TELEGRAM_ALLOWED_USER_ID");
    }

    info!(
        chain_id = meme_hunter::CHAIN_ID,
        mode = "PAPER",
        "Meme Hunter Rust started"
    );

    tokio::signal::ctrl_c()
        .await
        .context("waiting for Ctrl+C")?;
    info!("shutdown signal received");
    engine_task.abort();
    Ok(())
}

async fn restore_price_subscriptions(
    store: &Store,
    tx: &mpsc::UnboundedSender<PriceSubscription>,
) -> Result<()> {
    for address in store.open_position_addresses().await? {
        tx.send(PriceSubscription::Add(address))?;
    }
    Ok(())
}

async fn replay_graduations(store: &Store, tx: &mpsc::Sender<CriusEvent>) -> Result<()> {
    for (address, launchpad, ts_ms) in store.pending_graduations().await? {
        tx.send(CriusEvent::Graduation {
            address,
            launchpad,
            ts_ms,
        })
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_client_configuration_builds_with_both_providers_enabled() -> Result<()> {
        initialize_tls()?;
        let _config = rustls::ClientConfig::builder()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        Ok(())
    }

    #[tokio::test]
    async fn replays_more_pending_graduations_than_the_channel_capacity() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let db = format!("sqlite://{}", dir.path().join("replay.db").display());
        let store = Store::connect(&db).await?;
        for i in 0..513 {
            store
                .mark_graduated(&format!("0x{i:040x}"), None, i)
                .await?;
        }
        let reopened = Store::connect(&db).await?;
        let (tx, mut rx) = mpsc::channel(512);
        let consumer = tokio::spawn(async move {
            let mut seen = std::collections::HashSet::new();
            while let Some(CriusEvent::Graduation { address, .. }) = rx.recv().await {
                seen.insert(address);
            }
            seen
        });
        tokio::time::timeout(Duration::from_secs(5), replay_graduations(&reopened, &tx)).await??;
        drop(tx);
        assert_eq!(consumer.await?.len(), 513);
        reopened.close().await;
        store.close().await;
        Ok(())
    }
}
