use std::time::Duration;

use anyhow::{Context, Result, ensure};
use reqwest::{Client, redirect::Policy};
use serde_json::{Value, json};
use tracing::debug;

use crate::{CHAIN_ID, UNISWAP_ROUTER_212};

#[derive(Clone)]
pub struct UniswapClient {
    client: Client,
    base: String,
    api_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ValidatedSwapPayload {
    pub to: String,
    pub from: String,
    pub data: String,
    pub value: String,
    pub chain_id: u64,
    pub gas_limit: Option<String>,
}

impl UniswapClient {
    pub fn new(base: String, api_key: Option<String>) -> Self {
        Self {
            client: Client::builder()
                .redirect(Policy::none())
                .timeout(Duration::from_secs(15))
                .build()
                .expect("could not initialize Uniswap HTTP client"),
            base: base.trim_end_matches('/').to_string(),
            api_key,
        }
    }

    pub async fn quote_exact_input(
        &self,
        token_in: &str,
        token_out: &str,
        amount_base_units: &str,
        swapper: &str,
        slippage_pct: f64,
    ) -> Result<Value> {
        let key = self
            .api_key
            .as_deref()
            .context("UNISWAP_API_KEY is required for quote requests")?;
        ensure!(
            slippage_pct > 0.0 && slippage_pct <= 2.0,
            "slippage outside configured safety bound (0..2%)"
        );
        let mut api_key = reqwest::header::HeaderValue::from_str(key)
            .context("UNISWAP_API_KEY is not a valid HTTP header value")?;
        api_key.set_sensitive(true);
        let response = self
            .client
            .post(format!("{}/quote", self.base))
            .header("x-api-key", api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&json!({
                "type":"EXACT_INPUT",
                "amount":amount_base_units,
                "tokenInChainId":CHAIN_ID,
                "tokenOutChainId":CHAIN_ID,
                "tokenIn":token_in,
                "tokenOut":token_out,
                "swapper":swapper,
                "slippageTolerance":slippage_pct
            }))
            .send()
            .await
            .map_err(reqwest::Error::without_url)
            .context("Uniswap quote request failed")?;
        let status = response.status();
        ensure!(status.is_success(), "Uniswap quote HTTP {status}");
        let body: Value = response
            .json()
            .await
            .map_err(reqwest::Error::without_url)
            .context("invalid Uniswap quote JSON")?;
        let routing = body
            .get("routing")
            .and_then(Value::as_str)
            .unwrap_or_default();
        ensure!(
            routing == "CLASSIC",
            "unsupported Uniswap routing `{routing}`; starter only accepts CLASSIC routes"
        );
        Ok(body)
    }

    pub fn validate_swap_payload(
        &self,
        payload: &Value,
        expected_wallet: &str,
    ) -> Result<ValidatedSwapPayload> {
        let to = payload
            .get("to")
            .and_then(Value::as_str)
            .context("swap transaction missing `to`")?;
        let from = payload
            .get("from")
            .and_then(Value::as_str)
            .context("swap transaction missing `from`")?;
        let data = payload
            .get("data")
            .and_then(Value::as_str)
            .context("swap transaction missing `data`")?;
        let value = payload.get("value").and_then(Value::as_str).unwrap_or("0");
        let chain_id = payload
            .get("chainId")
            .and_then(Value::as_u64)
            .context("swap transaction missing chainId")?;
        ensure!(chain_id == CHAIN_ID, "unexpected swap chain id {chain_id}");
        ensure!(
            to.eq_ignore_ascii_case(UNISWAP_ROUTER_212),
            "swap `to` is not the allow-listed Universal Router 2.1.2 address"
        );
        ensure!(
            from.eq_ignore_ascii_case(expected_wallet),
            "swap sender does not match configured wallet"
        );
        let hex = data
            .strip_prefix("0x")
            .context("swap calldata is missing its 0x prefix")?;
        ensure!(
            hex.len() >= 8
                && hex.len().is_multiple_of(2)
                && hex.bytes().all(|b| b.is_ascii_hexdigit()),
            "swap calldata is malformed"
        );
        debug!(
            chain_id,
            "Uniswap transaction payload passed static validation (not a live-send approval)"
        );
        Ok(ValidatedSwapPayload {
            to: to.to_string(),
            from: from.to_string(),
            data: data.to_string(),
            value: value.to_string(),
            chain_id,
            gas_limit: payload
                .get("gasLimit")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WALLET: &str = "0x2222222222222222222222222222222222222222";

    fn payload() -> Value {
        json!({"to": UNISWAP_ROUTER_212, "from": WALLET, "data": "0x12345678", "value": "0", "chainId": CHAIN_ID})
    }

    fn client() -> UniswapClient {
        UniswapClient::new("https://example.test/v1".into(), None)
    }

    #[test]
    fn rejects_independent_payload_mismatches() {
        let client = client();
        let cases = [
            ("chainId", json!(CHAIN_ID + 1), "unexpected swap chain id"),
            ("to", json!(WALLET), "not the allow-listed"),
            ("from", json!(UNISWAP_ROUTER_212), "sender does not match"),
            ("data", json!("12345678"), "missing its 0x prefix"),
            ("data", json!("0x123456"), "calldata is malformed"),
            ("data", json!("0x123456780"), "calldata is malformed"),
            ("data", json!("0x1234567g"), "calldata is malformed"),
        ];
        for (field, value, expected_error) in cases {
            let mut invalid = payload();
            invalid[field] = value;
            let err = client.validate_swap_payload(&invalid, WALLET).unwrap_err();
            assert!(err.to_string().contains(expected_error), "{field}: {err}");
        }
    }

    #[test]
    fn accepts_expected_router_sender_and_chain() {
        assert!(client().validate_swap_payload(&payload(), WALLET).is_ok());
    }
}
