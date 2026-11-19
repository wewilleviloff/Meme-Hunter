use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result};
use rust_decimal::Decimal;
use tokio::sync::{RwLock, Semaphore, mpsc};
use tracing::{info, warn};

use crate::{
    config::AppConfig,
    crius::{CriusClient, CriusEvent, PriceSubscription},
    storage::Store,
    strategy::{
        CandidateMetrics, Decision, ExitSignal, StrategyConfig, evaluate_candidate, evaluate_exit,
    },
    trading::paper,
};

#[derive(Debug, Default)]
pub struct EngineControl {
    pub paused: bool,
}

#[derive(Clone)]
pub struct ScannerEngine {
    config: AppConfig,
    store: Store,
    crius: CriusClient,
    subscriptions: mpsc::UnboundedSender<PriceSubscription>,
    control: Arc<RwLock<EngineControl>>,
    screening_slots: Arc<Semaphore>,
}

impl ScannerEngine {
    pub fn new(
        config: AppConfig,
        store: Store,
        crius: CriusClient,
        subscriptions: mpsc::UnboundedSender<PriceSubscription>,
        control: Arc<RwLock<EngineControl>>,
    ) -> Self {
        Self {
            config,
            store,
            crius,
            subscriptions,
            control,
            screening_slots: Arc::new(Semaphore::new(4)),
        }
    }

    pub async fn run(&self, mut rx: mpsc::Receiver<CriusEvent>) -> Result<()> {
        let mut last_prices = HashMap::new();
        while let Some(event) = rx.recv().await {
            match event {
                CriusEvent::Launch {
                    address,
                    launchpad,
                    ts_ms,
                } => {
                    if let Err(err) = self
                        .store
                        .record_launch(&address, launchpad.as_deref(), ts_ms)
                        .await
                    {
                        warn!(%address,error=%err,"could not persist launch event");
                    }
                    info!(%address, launchpad=?launchpad, "new token launch detected");
                }
                CriusEvent::Graduation {
                    address,
                    launchpad,
                    ts_ms,
                } => {
                    info!(%address, launchpad=?launchpad, "token graduation detected");
                    if let Err(err) = self
                        .on_graduation(&address, launchpad.as_deref(), ts_ms)
                        .await
                    {
                        warn!(%address,error=%err,"candidate screening failed");
                    }
                }
                CriusEvent::Price {
                    token, usd, ts_ms, ..
                } => {
                    if let Err(err) = self.on_price(&token, usd, ts_ms, &mut last_prices).await {
                        warn!(%token,error=%err,"price processing failed");
                    }
                }
            }
        }
        Ok(())
    }

    async fn on_graduation(
        &self,
        address: &str,
        launchpad: Option<&str>,
        ts_ms: i64,
    ) -> Result<()> {
        let launch_ts = self.store.mark_graduated(address, launchpad, ts_ms).await?;
        if !self.store.claim_screening(address).await? {
            return Ok(());
        }
        if launch_ts <= 0 {
            self.store
                .mark_screened(
                    address,
                    false,
                    "launch event not observed; token age unknown",
                )
                .await?;
            return Ok(());
        }
        let age = chrono::Utc::now()
            .timestamp_millis()
            .saturating_sub(launch_ts)
            .max(0)
            / 1000;
        let delay = self.config.min_token_age_seconds.saturating_sub(age).max(0) as u64;
        let engine = self.clone();
        let token = address.to_string();
        tokio::spawn(async move {
            if delay > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            }
            let Ok(_slot) = engine.screening_slots.acquire().await else {
                return;
            };
            if let Err(err) = engine.screen_graduated(&token).await {
                warn!(%token, error=%err, "candidate screening failed");
            }
        });
        Ok(())
    }

    async fn screen_graduated(&self, address: &str) -> Result<()> {
        let Some(launch_ts) = self.store.get_launch_ts(address).await? else {
            self.store
                .mark_screened(address, false, "launch timestamp unavailable")
                .await?;
            return Ok(());
        };
        if launch_ts <= 0 {
            self.store
                .mark_screened(
                    address,
                    false,
                    "launch event not observed; token age unknown",
                )
                .await?;
            return Ok(());
        }
        let age = || {
            chrono::Utc::now()
                .timestamp_millis()
                .saturating_sub(launch_ts)
                / 1000
        };
        if launch_ts > chrono::Utc::now().timestamp_millis()
            || age() > self.config.max_token_age_seconds
        {
            self.store
                .mark_screened(address, false, "token age outside configured window")
                .await?;
            return Ok(());
        }
        let data = async {
            let profile = self
                .crius
                .get_profile(address)
                .await
                .context("fetch token profile")?;
            let (security, _) = self
                .crius
                .get_security(address, self.config.buy_usd)
                .await
                .context("fetch token security")?;
            Ok::<_, anyhow::Error>((profile, security))
        }
        .await;
        let (profile, security) = match data {
            Ok(data) => data,
            Err(error) => {
                self.store
                    .mark_screened(address, false, "Crius screening data unavailable")
                    .await?;
                return Err(error);
            }
        };
        let metrics = CandidateMetrics {
            age_seconds: age(),
            graduated: profile.launch_status.as_deref() == Some("graduated"),
            liquidity_usd: profile.quote_liquidity_usd,
            price_usd: profile.price_usd,
            price_confidence: profile.price_confidence.clone(),
            thin_liquidity: profile.thin_liquidity,
            pools_count: profile.pools_count,
            holders_count: profile.holders_count,
            top10_pct: profile.top10_pct,
            dev_pct: profile.dev_pct,
            volume_5m_usd: profile.volume_5m_usd,
            security_score: security.score,
            security_risk: security.risk.clone(),
            is_honeypot: security.is_honeypot,
            selective_sell: security.selective_sell,
            tax_source: security.tax_source.clone(),
            buy_tax_pct: security.buy_tax_pct,
            sell_tax_pct: security.sell_tax_pct,
            realized_sell_samples: security.realized_sell_samples,
            blacklist: security.blacklist,
            mintable: security.mintable,
            pausable: security.pausable,
            upgradeable: security.upgradeable,
            can_change_tax: security.can_change_tax,
            can_change_limits: security.can_change_limits,
            is_proxy: security.is_proxy,
            self_destruct: security.self_destruct,
            hook_present: security.hook_present,
            hook_risk_level: security.hook_risk_level.clone(),
            hook_can_block_swaps: security.hook_can_block_swaps,
            max_sell_tax_seen_pct: security.max_sell_tax_seen_pct,
        };
        let strategy = StrategyConfig {
            min_age_seconds: self.config.min_token_age_seconds,
            max_age_seconds: self.config.max_token_age_seconds,
            min_liquidity_usd: self.config.min_liquidity_usd,
            min_security_score: self.config.min_security_score,
            min_holders: self.config.min_holders,
            max_top10_pct: self.config.max_top10_holder_pct,
            max_dev_pct: self.config.max_dev_holder_pct,
            min_volume_5m_usd: self.config.min_volume_5m_usd,
            max_buy_tax_pct: self.config.max_buy_tax_pct,
            max_sell_tax_pct: self.config.max_sell_tax_pct,
            min_realized_sell_samples: self.config.min_realized_sell_samples,
        };
        match evaluate_candidate(&metrics, &strategy) {
            Decision::Reject(reason) => {
                self.store.mark_screened(address, false, &reason).await?;
                info!(%address, %reason, "token rejected by strategy");
            }
            Decision::Accept => {
                let control = self.control.read().await;
                if control.paused {
                    self.store
                        .mark_screened(address, false, "engine paused")
                        .await?;
                    info!(%address, "candidate passed filters but engine is paused");
                    return Ok(());
                }
                let Some(price) = profile.price_usd else {
                    self.store
                        .mark_screened(address, false, "price unavailable at entry")
                        .await?;
                    return Ok(());
                };
                let symbol = profile.symbol.as_deref().unwrap_or("UNKNOWN");
                let outcome =
                    paper::open_paper_position(&self.store, &self.config, address, symbol, price)
                        .await?;
                drop(control);
                match outcome {
                    paper::OpenPaperOutcome::Opened(position_id) => {
                        let _ = self
                            .subscriptions
                            .send(PriceSubscription::Add(address.to_owned()));
                        info!(%address, %symbol, %price, %position_id, "paper position opened");
                    }
                    paper::OpenPaperOutcome::Rejected(reason) => {
                        info!(%address, %reason, "paper entry rejected")
                    }
                    paper::OpenPaperOutcome::NotClaimed => {}
                }
            }
        }
        Ok(())
    }

    async fn on_price(
        &self,
        address: &str,
        price: Decimal,
        ts_ms: i64,
        last_prices: &mut HashMap<String, i64>,
    ) -> Result<()> {
        if price <= Decimal::ZERO || ts_ms > chrono::Utc::now().timestamp_millis() {
            return Ok(());
        }
        for position in self.store.open_positions().await? {
            if position.token_address != address
                || ts_ms <= position.opened_at_ms
                || last_prices
                    .get(&position.id)
                    .is_some_and(|last| ts_ms < *last)
            {
                continue;
            }
            if price > position.highest_price_usd {
                self.store.update_highest_price(&position.id, price).await?;
            }
            let signal = evaluate_exit(
                position.entry_price_usd,
                price,
                self.config.take_profit_multiple,
                self.config.stop_loss_pct,
            );
            if let Some(signal) = signal {
                let reason = match signal {
                    ExitSignal::TakeProfit => "TAKE_PROFIT",
                    ExitSignal::StopLoss => "STOP_LOSS",
                };
                if paper::close_paper_position(&self.store, &self.config, &position, price, reason)
                    .await?
                {
                    let _ = self
                        .subscriptions
                        .send(PriceSubscription::Remove(address.to_owned()));
                    info!(%address, %price, reason, "paper position closed");
                }
                last_prices.remove(&position.id);
            } else {
                last_prices.insert(position.id, ts_ms);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    #[tokio::test]
    async fn graduation_opens_once_and_fresh_price_closes_after_restart() -> Result<()> {
        const TOKEN: &str = "0x1111111111111111111111111111111111111111";
        let fixture = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let crius = CriusClient::new(format!("http://{}", fixture.local_addr()?), None)?;
        let mock = tokio::spawn(async move {
            for (suffix, data) in [
                (
                    "",
                    json!({"symbol":"TKN", "launch":{"status":"graduated"}, "price":{"usd":"2", "confidence":"high", "thin_liquidity":false}, "market":{"quote_liquidity_usd":10000, "pools_count":1}, "holders":{"count":40,"top10_pct":30,"dev_pct":1}, "stats":{"volume_usd":{"5m":1000}}}),
                ),
                (
                    "/security?amount_usd=10",
                    json!({"score":95,"risk":"low","honeypot":{"is_honeypot":false,"selective":false},"tax":{"source":"realized","buy_pct":0,"sell_pct":0,"realized":{"samples":{"sells":4},"max_sell_pct_seen":0}},"contract":{"blacklist":false,"mintable":false,"pausable":false,"upgradeable":false,"can_change_tax":false,"can_change_limits":false,"is_proxy":false,"self_destruct":false},"hook":null}),
                ),
            ] {
                let (mut socket, _) = fixture.accept().await?;
                let mut reader = BufReader::new(&mut socket);
                let mut line = String::new();
                reader.read_line(&mut line).await?;
                assert!(line.starts_with(&format!("GET /token/{TOKEN}{suffix} HTTP/")));
                loop {
                    line.clear();
                    if reader.read_line(&mut line).await? == 0 || line == "\r\n" {
                        break;
                    }
                }
                let body = json!({"data":data}).to_string();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await?;
            }
            Ok::<_, anyhow::Error>(())
        });
        let dir = tempfile::tempdir()?;
        let url = format!("sqlite://{}", dir.path().join("scanner.db").display());
        let store = Store::connect(&url).await?;
        let cfg = AppConfig::test_config();
        let control = Arc::new(RwLock::new(EngineControl::default()));
        let (subscriptions, mut changes) = mpsc::unbounded_channel();
        let engine = ScannerEngine::new(
            cfg.clone(),
            store.clone(),
            crius.clone(),
            subscriptions.clone(),
            control.clone(),
        );
        let (tx, rx) = mpsc::channel(8);
        let task = tokio::spawn(async move { engine.run(rx).await });
        let now = chrono::Utc::now().timestamp_millis();
        tx.send(CriusEvent::Launch {
            address: TOKEN.into(),
            launchpad: None,
            ts_ms: now - 120_000,
        })
        .await?;
        for _ in 0..2 {
            tx.send(CriusEvent::Graduation {
                address: TOKEN.into(),
                launchpad: None,
                ts_ms: now - 1_000,
            })
            .await?;
        }
        assert!(
            matches!(tokio::time::timeout(Duration::from_secs(5), changes.recv()).await?, Some(PriceSubscription::Add(token)) if token == TOKEN)
        );
        tokio::time::timeout(Duration::from_secs(5), mock).await???;
        drop(tx);
        task.await??;
        store.close().await;

        let reopened = Store::connect(&url).await?;
        let position = reopened
            .open_positions()
            .await?
            .pop()
            .context("expected a persisted position")?;
        assert_eq!(reopened.summary().await?.1, 1);
        control.write().await.paused = true;
        let engine = ScannerEngine::new(cfg, reopened.clone(), crius, subscriptions, control);
        let mut last_prices = HashMap::new();
        engine
            .on_price(
                TOKEN,
                Decimal::from(4),
                position.opened_at_ms,
                &mut last_prices,
            )
            .await?;
        assert_eq!(reopened.open_positions().await?.len(), 1);
        tokio::time::sleep(Duration::from_millis(15)).await;
        let now = chrono::Utc::now().timestamp_millis();
        engine
            .on_price(TOKEN, Decimal::from(3), now, &mut last_prices)
            .await?;
        engine
            .on_price(TOKEN, Decimal::from(4), now - 1, &mut last_prices)
            .await?;
        assert_eq!(reopened.open_positions().await?.len(), 1);
        engine
            .on_price(TOKEN, Decimal::from(4), now, &mut last_prices)
            .await?;
        assert!(matches!(changes.try_recv()?, PriceSubscription::Remove(token) if token == TOKEN));
        assert_eq!(reopened.open_positions().await?.len(), 0);
        assert_eq!(reopened.summary().await?.2, "9.5".parse::<Decimal>()?);
        reopened.close().await;
        Ok(())
    }
}
