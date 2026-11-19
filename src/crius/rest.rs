use anyhow::{Context, Result, ensure};
use reqwest::Client;
use rust_decimal::Decimal;
use serde_json::Value;
use std::str::FromStr;

#[derive(Clone)]
pub struct CriusClient {
    client: Client,
    base: String,
    api_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TokenProfile {
    pub address: String,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub price_usd: Option<Decimal>,
    pub price_confidence: Option<String>,
    pub thin_liquidity: Option<bool>,
    pub market_liquidity_usd: Option<Decimal>,
    pub quote_liquidity_usd: Option<Decimal>,
    pub pools_count: Option<u64>,
    pub holders_count: Option<u64>,
    pub top10_pct: Option<Decimal>,
    pub dev_pct: Option<Decimal>,
    pub launch_status: Option<String>,
    pub launchpad: Option<String>,
    pub volume_5m_usd: Option<Decimal>,
}

#[derive(Debug, Clone)]
pub struct SecurityReport {
    pub score: Option<u8>,
    pub risk: Option<String>,
    pub is_honeypot: Option<bool>,
    pub selective_sell: Option<bool>,
    pub tax_source: Option<String>,
    pub buy_tax_pct: Option<Decimal>,
    pub sell_tax_pct: Option<Decimal>,
    pub realized_sell_samples: Option<u32>,
    pub blacklist: Option<bool>,
    pub mintable: Option<bool>,
    pub pausable: Option<bool>,
    pub upgradeable: Option<bool>,
    pub can_change_tax: Option<bool>,
    pub can_change_limits: Option<bool>,
    pub is_proxy: Option<bool>,
    pub self_destruct: Option<bool>,
    pub hook_present: Option<bool>,
    pub hook_risk_level: Option<String>,
    pub hook_can_block_swaps: Option<bool>,
    pub max_sell_tax_seen_pct: Option<Decimal>,
    pub simulation_note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PriceSnapshot {
    pub block_ts_ms: Option<i64>,
    pub usd: Option<Decimal>,
    pub eth: Option<Decimal>,
    pub liquidity_usd: Option<Decimal>,
    pub confidence: Option<String>,
    pub thin_liquidity: Option<bool>,
}

impl CriusClient {
    pub fn new(base: String, api_key: Option<String>) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(12))
            .user_agent("meme-hunter-rust/0.1")
            .build()?;
        Ok(Self {
            client,
            base: base.trim_end_matches('/').to_string(),
            api_key,
        })
    }

    fn request(&self, url: String) -> reqwest::RequestBuilder {
        let req = self.client.get(url);
        match self.api_key.as_deref() {
            Some(key) => req.header("x-api-key", key),
            None => req,
        }
    }

    async fn get_data(&self, path: &str) -> Result<(Value, Value)> {
        let url = format!("{}{path}", self.base);
        let mut delays = [1, 2].into_iter();
        let response = loop {
            let result = self
                .request(url.clone())
                .send()
                .await
                .and_then(reqwest::Response::error_for_status);
            match result {
                Ok(response) => break response,
                Err(error) => {
                    let transient = error.is_timeout()
                        || error.is_connect()
                        || error.status().is_some_and(|status| {
                            status.as_u16() == 429 || status.is_server_error()
                        });
                    let Some(delay) = delays.next().filter(|_| transient) else {
                        return Err(error).context("Crius request failed");
                    };
                    tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                }
            }
        };
        let body: Value = response
            .json()
            .await
            .context("Crius returned invalid JSON")?;
        let data = body
            .get("data")
            .cloned()
            .context("Crius response has no data field")?;
        let meta = body.get("meta").cloned().unwrap_or(Value::Null);
        Ok((data, meta))
    }

    pub async fn get_profile(&self, address: &str) -> Result<TokenProfile> {
        validate_address(address)?;
        let (v, _) = self.get_data(&format!("/token/{address}")).await?;
        Ok(TokenProfile {
            address: address.to_ascii_lowercase(),
            name: string_at(&v, &["name"]),
            symbol: string_at(&v, &["symbol"]),
            price_usd: decimal_at(&v, &["price", "usd"]),
            price_confidence: string_at(&v, &["price", "confidence"]),
            thin_liquidity: bool_at(&v, &["price", "thin_liquidity"]),
            market_liquidity_usd: decimal_at(&v, &["market", "liquidity_usd"]),
            quote_liquidity_usd: decimal_at(&v, &["market", "quote_liquidity_usd"]),
            pools_count: u64_at(&v, &["market", "pools_count"]),
            holders_count: u64_at(&v, &["holders", "count"]),
            top10_pct: decimal_at(&v, &["holders", "top10_pct"]),
            dev_pct: decimal_at(&v, &["holders", "dev_pct"]),
            launch_status: string_at(&v, &["launch", "status"]),
            launchpad: string_at(&v, &["launch", "launchpad"]),
            volume_5m_usd: decimal_at(&v, &["stats", "volume_usd", "5m"]),
        })
    }

    pub async fn get_security(
        &self,
        address: &str,
        amount_usd: Decimal,
    ) -> Result<(SecurityReport, Value)> {
        validate_address(address)?;
        let (v, meta) = self
            .get_data(&format!(
                "/token/{address}/security?amount_usd={amount_usd}"
            ))
            .await?;
        let report = SecurityReport {
            score: u64_at(&v, &["score"]).and_then(|n| u8::try_from(n).ok()),
            risk: string_at(&v, &["risk"]),
            is_honeypot: bool_at(&v, &["honeypot", "is_honeypot"]),
            selective_sell: bool_at(&v, &["honeypot", "selective"]),
            tax_source: string_at(&v, &["tax", "source"]),
            buy_tax_pct: decimal_at(&v, &["tax", "buy_pct"]),
            sell_tax_pct: decimal_at(&v, &["tax", "sell_pct"]),
            realized_sell_samples: u64_at(&v, &["tax", "realized", "samples", "sells"])
                .and_then(|n| u32::try_from(n).ok()),
            blacklist: bool_at(&v, &["contract", "blacklist"]),
            mintable: bool_at(&v, &["contract", "mintable"]),
            pausable: bool_at(&v, &["contract", "pausable"]),
            upgradeable: bool_at(&v, &["contract", "upgradeable"]),
            can_change_tax: bool_at(&v, &["contract", "can_change_tax"]),
            can_change_limits: bool_at(&v, &["contract", "can_change_limits"]),
            is_proxy: bool_at(&v, &["contract", "is_proxy"]),
            self_destruct: bool_at(&v, &["contract", "self_destruct"]),
            hook_present: match v.get("hook") {
                Some(Value::Null) => Some(false),
                Some(hook) => hook
                    .get("address")
                    .and_then(Value::as_str)
                    .filter(|address| validate_address(address).is_ok())
                    .map(|address| address != crate::NATIVE_ETH_ADDRESS),
                None => None,
            },
            hook_risk_level: string_at(&v, &["hook", "risk_level"]),
            hook_can_block_swaps: bool_at(&v, &["hook", "capabilities", "can_block_swaps"]),
            max_sell_tax_seen_pct: decimal_at(&v, &["tax", "realized", "max_sell_pct_seen"]),
            simulation_note: string_at(&meta, &["simulation"]),
        };
        Ok((report, meta))
    }

    pub async fn get_price(&self, address: &str) -> Result<PriceSnapshot> {
        validate_address(address)?;
        let (v, meta) = self.get_data(&format!("/token/{address}/price")).await?;
        Ok(PriceSnapshot {
            block_ts_ms: meta.get("block_ts").and_then(Value::as_i64),
            usd: decimal_at(&v, &["usd"]),
            eth: decimal_at(&v, &["eth"]),
            liquidity_usd: decimal_at(&v, &["liquidity_usd"]),
            confidence: string_at(&v, &["confidence"]),
            thin_liquidity: bool_at(&v, &["thin_liquidity"]),
        })
    }
}

pub fn validate_address(address: &str) -> Result<()> {
    ensure!(
        address.len() == 42
            && address.starts_with("0x")
            && address[2..].bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid EVM address"
    );
    Ok(())
}

fn at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |cur, key| cur.get(*key))
}
fn string_at(v: &Value, p: &[&str]) -> Option<String> {
    at(v, p)?.as_str().map(str::to_string)
}
fn bool_at(v: &Value, p: &[&str]) -> Option<bool> {
    at(v, p)?.as_bool()
}
fn u64_at(v: &Value, p: &[&str]) -> Option<u64> {
    at(v, p)?.as_u64()
}
fn decimal_at(v: &Value, p: &[&str]) -> Option<Decimal> {
    let v = at(v, p)?;
    let s = if let Some(s) = v.as_str() {
        s.to_string()
    } else {
        v.as_number()?.to_string()
    };
    Decimal::from_str(&s).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_profile_fields_from_crius_shape() {
        let value = serde_json::json!({"name":"Test","symbol":"TST","price":{"usd":0.012,"confidence":"high","thin_liquidity":false},"market":{"liquidity_usd":20000,"quote_liquidity_usd":10000,"pools_count":1},"holders":{"count":40,"top10_pct":20.5,"dev_pct":1},"launch":{"status":"graduated"},"stats":{"volume_usd":{"5m":750}}});
        assert_eq!(
            decimal_at(&value, &["market", "liquidity_usd"]),
            Some(Decimal::from(20000))
        );
        assert_eq!(u64_at(&value, &["holders", "count"]), Some(40));
        assert_eq!(
            string_at(&value, &["launch", "status"]).as_deref(),
            Some("graduated")
        );
    }
}
