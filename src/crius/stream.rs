use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::str::FromStr;
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest, http::header::HeaderValue},
};
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
pub enum CriusEvent {
    Launch {
        address: String,
        launchpad: Option<String>,
        ts_ms: i64,
    },
    Graduation {
        address: String,
        launchpad: Option<String>,
        ts_ms: i64,
    },
    Price {
        token: String,
        usd: Decimal,
        liquidity_usd: Option<Decimal>,
        ts_ms: i64,
    },
}

pub enum PriceSubscription {
    Add(String),
    Remove(String),
}

pub struct CriusStream {
    url: String,
    api_key: Option<String>,
}

impl CriusStream {
    pub fn new(url: String, api_key: Option<String>) -> Self {
        Self { url, api_key }
    }

    pub async fn run(
        &self,
        event_tx: mpsc::Sender<CriusEvent>,
        mut subscription_rx: mpsc::UnboundedReceiver<PriceSubscription>,
    ) -> Result<()> {
        let mut dynamic_channels: HashSet<String> = HashSet::new();
        let mut last_seq: HashMap<String, u64> = HashMap::new();
        let base_channels = vec!["launches:all".to_string(), "graduations:all".to_string()];
        let mut delay = Duration::from_secs(1);
        while !event_tx.is_closed() {
            while let Ok(change) = subscription_rx.try_recv() {
                apply_subscription(change, &mut dynamic_channels, &mut last_seq);
            }
            let connected_at = tokio::time::Instant::now();
            match self
                .connect_once(
                    &event_tx,
                    &mut subscription_rx,
                    &base_channels,
                    &mut dynamic_channels,
                    &mut last_seq,
                )
                .await
            {
                Ok(()) => warn!("Crius websocket disconnected; reconnecting"),
                Err(err) => warn!(error = %err, "Crius websocket error; reconnecting"),
            }
            if connected_at.elapsed() >= Duration::from_secs(45) {
                delay = Duration::from_secs(1);
            }
            tokio::select! {
                _ = event_tx.closed() => break,
                _ = tokio::time::sleep(delay) => {},
            }
            delay = (delay * 2).min(Duration::from_secs(30));
        }
        Ok(())
    }

    async fn connect_once(
        &self,
        event_tx: &mpsc::Sender<CriusEvent>,
        subscription_rx: &mut mpsc::UnboundedReceiver<PriceSubscription>,
        base_channels: &[String],
        dynamic_channels: &mut HashSet<String>,
        last_seq: &mut HashMap<String, u64>,
    ) -> Result<()> {
        let mut request = self.url.clone().into_client_request()?;
        if let Some(key) = &self.api_key {
            request.headers_mut().insert(
                "x-api-key",
                HeaderValue::from_str(key).context("invalid Crius API key header")?,
            );
        } else {
            anyhow::bail!("Crius WebSocket requires an API key with `stream` scope");
        }
        let (socket, _) = tokio::time::timeout(Duration::from_secs(15), connect_async(request))
            .await
            .context("websocket connection timed out")?
            .context("websocket connect failed")?;
        let (mut write, mut read) = socket.split();
        let mut channels = base_channels.to_vec();
        channels.extend(dynamic_channels.iter().cloned());
        write
            .send(Message::Text(
                json!({"op":"subscribe","channels":channels,"finality":"safe"})
                    .to_string()
                    .into(),
            ))
            .await?;
        if !last_seq.is_empty() {
            write
                .send(Message::Text(
                    json!({"op":"resume","seqs":last_seq}).to_string().into(),
                ))
                .await?;
        }
        info!("connected to Crius WebSocket; subscribed at safe finality");
        let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
        let mut last_message = tokio::time::Instant::now();
        loop {
            tokio::select! {
                _ = event_tx.closed() => break,
                _ = tokio::time::sleep_until(last_message + Duration::from_secs(45)) => anyhow::bail!("Crius websocket idle timeout"),
                incoming = read.next() => {
                    let Some(incoming) = incoming else { break; };
                    last_message = tokio::time::Instant::now();
                    match incoming? {
                        Message::Text(text) => {
                            let Ok(envelope) = serde_json::from_str::<Value>(&text) else {
                                warn!("ignored invalid Crius websocket JSON");
                                continue;
                            };
                            let channel = envelope.get("ch").and_then(Value::as_str).unwrap_or_default();
                            if channel == "error" {
                                let code = envelope.pointer("/data/code").and_then(Value::as_str).unwrap_or("unknown");
                                warn!(%code, "Crius websocket reported an error");
                                if code == "lagged" && !last_seq.is_empty() {
                                    write.send(Message::Text(json!({"op":"resume","seqs":last_seq}).to_string().into())).await?;
                                }
                                continue;
                            }
                            if !base_channels.iter().any(|base| base == channel) && !dynamic_channels.contains(channel) {
                                continue;
                            }
                            let seq = envelope.get("seq").and_then(Value::as_u64);
                            if seq.zip(last_seq.get(channel)).is_some_and(|(current, last)| current <= *last) {
                                continue;
                            }
                            if let Some(event) = parse_event(&envelope) {
                                event_tx.send(event).await.context("event receiver closed")?;
                            }
                            if let Some(seq) = seq {
                                last_seq.insert(channel.to_string(), seq);
                            }
                        }
                        Message::Ping(payload) => write.send(Message::Pong(payload)).await?,
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
                change = subscription_rx.recv() => {
                    let Some(change) = change else { break; };
                    if let Some((op, channel)) = apply_subscription(change, dynamic_channels, last_seq) {
                        write.send(Message::Text(json!({"op":op,"channels":[channel]}).to_string().into())).await?;
                        debug!(%op, %channel, "updated Crius price subscription");
                    }
                }
                _ = heartbeat.tick() => {
                    write.send(Message::Text(json!({"op":"ping"}).to_string().into())).await?;
                }
            }
        }
        Ok(())
    }
}

fn apply_subscription(
    change: PriceSubscription,
    channels: &mut HashSet<String>,
    last_seq: &mut HashMap<String, u64>,
) -> Option<(&'static str, String)> {
    match change {
        PriceSubscription::Add(address) => {
            let channel = format!("price:{}", address.to_ascii_lowercase());
            channels
                .insert(channel.clone())
                .then_some(("subscribe", channel))
        }
        PriceSubscription::Remove(address) => {
            let channel = format!("price:{}", address.to_ascii_lowercase());
            last_seq.remove(&channel);
            channels
                .remove(&channel)
                .then_some(("unsubscribe", channel))
        }
    }
}

fn parse_event(v: &Value) -> Option<CriusEvent> {
    let channel = v.get("ch").and_then(Value::as_str)?;
    let data = v.get("data")?;
    if data.get("removed").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    // Untimestamped price snapshots cannot be ordered against replayed events.
    let ts_ms = v.get("ts").and_then(Value::as_i64).filter(|ts| *ts > 0)?;
    let launchpad = || {
        data.get("launchpad")
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let address = |value: &str| {
        super::rest::validate_address(value)
            .ok()
            .map(|_| value.to_ascii_lowercase())
    };
    if channel.starts_with("launches:") {
        return Some(CriusEvent::Launch {
            address: address(data.get("address")?.as_str()?)?,
            launchpad: launchpad(),
            ts_ms,
        });
    }
    if channel.starts_with("graduations:") {
        return Some(CriusEvent::Graduation {
            address: address(data.get("token")?.as_str()?)?,
            launchpad: launchpad(),
            ts_ms,
        });
    }
    Some(CriusEvent::Price {
        token: address(channel.strip_prefix("price:")?)?,
        usd: decimal_from_value(data.get("usd"))?,
        liquidity_usd: decimal_from_value(data.get("liquidity_usd")),
        ts_ms,
    })
}

fn decimal_from_value(v: Option<&Value>) -> Option<Decimal> {
    let v = v?;
    let s = v
        .as_str()
        .map(str::to_string)
        .or_else(|| v.as_number().map(ToString::to_string))?;
    Decimal::from_str(&s).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscribes_safely_resumes_after_lag_and_unsubscribes() -> Result<()> {
        const TOKEN: &str = "0x1111111111111111111111111111111111111111";
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let channel = format!("price:{TOKEN}");
        let stream = CriusStream::new(
            format!("ws://{}", listener.local_addr()?),
            Some("local-test".into()),
        );
        let (events, mut incoming) = mpsc::channel(8);
        let (subscriptions, changes) = mpsc::unbounded_channel();
        subscriptions.send(PriceSubscription::Add(TOKEN.into()))?;
        let task = tokio::spawn(async move { stream.run(events, changes).await });
        let (socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept()).await??;
        let mut socket = tokio_tungstenite::accept_async(socket).await?;
        let expected = channel.clone();
        tokio::time::timeout(Duration::from_secs(5), async {
            let subscribe: Value = serde_json::from_str(socket.next().await.context("subscription missing")??.to_text()?)?;
            assert_eq!(subscribe["finality"], "safe");
            assert!(subscribe["channels"].as_array().unwrap().contains(&json!(expected)));
            let price = |seq, usd| json!({"ch":channel, "ts":1790000000000i64, "seq":seq, "data":{"usd":usd}});
            for frame in [json!({"ch":channel,"snapshot":true,"data":{"usd":4}}), price(1, 1), json!({"ch":"error","data":{"code":"lagged"}})] {
                socket.send(Message::Text(frame.to_string().into())).await?;
            }
            loop {
                let frame: Value = serde_json::from_str(socket.next().await.context("resume missing")??.to_text()?)?;
                if frame["op"] == "resume" {
                    assert_eq!(frame["seqs"][channel.as_str()], 1);
                    break;
                }
            }
            for frame in [price(1, 4), price(2, 2)] {
                socket.send(Message::Text(frame.to_string().into())).await?;
            }
            for expected in [Decimal::ONE, Decimal::TWO] {
                assert!(matches!(incoming.recv().await, Some(CriusEvent::Price { usd, .. }) if usd == expected));
            }
            subscriptions.send(PriceSubscription::Remove(TOKEN.into()))?;
            loop {
                let frame: Value = serde_json::from_str(socket.next().await.context("unsubscribe missing")??.to_text()?)?;
                if frame["op"] == "unsubscribe" {
                    assert_eq!(frame["channels"], json!([channel]));
                    break;
                }
            }
            Ok::<_, anyhow::Error>(())
        }).await??;
        drop(incoming);
        tokio::time::timeout(Duration::from_secs(5), task).await???;
        Ok(())
    }

    #[test]
    fn parses_launch_and_price_envelopes() {
        let launch = r#"{"ch":"launches:all","seq":1,"ts":1790000000000,"data":{"address":"0x1111111111111111111111111111111111111111","launchpad":"pons"}}"#;
        assert!(
            matches!(parse_event(&serde_json::from_str(launch).unwrap()), Some(CriusEvent::Launch { launchpad: Some(p), .. }) if p == "pons")
        );
        let price = r#"{"ch":"price:0x1111111111111111111111111111111111111111","seq":2,"ts":1790000001000,"data":{"usd":0.25,"liquidity_usd":15000}}"#;
        assert!(
            matches!(parse_event(&serde_json::from_str(price).unwrap()), Some(CriusEvent::Price { usd, .. }) if usd == Decimal::new(25,2))
        );
        let removed = r#"{"ch":"graduations:all","seq":2,"ts":1790000000000,"data":{"token":"0x1111111111111111111111111111111111111111","removed":true}}"#;
        assert!(parse_event(&serde_json::from_str(removed).unwrap()).is_none());
    }
}
