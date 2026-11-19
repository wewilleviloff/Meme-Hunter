//! Read-only diagnostics, independent of the paper-trading bot configuration.

use std::env;

use alloy::primitives::{Address, U256};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use crate::{blockchain::wallet::verify_rpc_chain, trading::uniswap::UniswapClient};

pub const QUOTE_USAGE: &str = "Usage: meme-hunter diagnose-quote <TOKEN_IN> <TOKEN_OUT> <AMOUNT_BASE_UNITS> <SWAPPER> [SLIPPAGE_PCT]\n\
    Set RPC_URL and UNISWAP_API_KEY; UNISWAP_API_BASE is optional.\n\
    Addresses are public 0x-prefixed EVM addresses; slippage defaults to 0.5%.";
const DEFAULT_UNISWAP_API_BASE: &str = "https://trade-api.gateway.uniswap.org/v1";

/// Runs `diagnose-quote` with arguments after the subcommand name.
pub async fn run_quote(args: &[String]) -> Result<()> {
    if matches!(args, [arg] if arg == "--help" || arg == "-h") {
        println!("{QUOTE_USAGE}");
        return Ok(());
    }
    let input = QuoteInput::parse(args)?;
    let rpc_url = required_env("RPC_URL")?;
    let api_key = required_env("UNISWAP_API_KEY")?;
    let api_base = match env::var("UNISWAP_API_BASE") {
        Ok(base) => base,
        Err(env::VarError::NotPresent) => DEFAULT_UNISWAP_API_BASE.into(),
        Err(env::VarError::NotUnicode(_)) => {
            anyhow::bail!("UNISWAP_API_BASE must be valid Unicode")
        }
    };
    let report = diagnose_quote(&input, &rpc_url, &api_base, api_key).await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn required_env(name: &str) -> Result<String> {
    env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("{name} is required for diagnose-quote"))
}

#[derive(Debug)]
struct QuoteInput {
    token_in: Address,
    token_out: Address,
    amount: U256,
    swapper: Address,
    slippage_pct: f64,
}

impl QuoteInput {
    fn parse(args: &[String]) -> Result<Self> {
        ensure!((4..=5).contains(&args.len()), "{QUOTE_USAGE}");
        let token_in = parse_address(&args[0], "TOKEN_IN")?;
        let token_out = parse_address(&args[1], "TOKEN_OUT")?;
        ensure!(token_in != token_out, "TOKEN_IN and TOKEN_OUT must differ");
        let amount = &args[2];
        ensure!(
            !amount.is_empty() && amount.bytes().all(|b| b.is_ascii_digit()),
            "AMOUNT_BASE_UNITS must be an unsigned decimal integer"
        );
        let amount =
            U256::from_str_radix(amount, 10).context("AMOUNT_BASE_UNITS exceeds uint256")?;
        ensure!(amount != U256::ZERO, "AMOUNT_BASE_UNITS must be positive");
        let swapper = parse_address(&args[3], "SWAPPER")?;
        ensure!(swapper != Address::ZERO, "SWAPPER must be nonzero");
        let slippage_pct = args
            .get(4)
            .map_or(Ok(0.5), |value| value.parse::<f64>())
            .context("SLIPPAGE_PCT must be a number")?;
        ensure!(
            slippage_pct > 0.0 && slippage_pct <= 2.0,
            "SLIPPAGE_PCT must be greater than 0 and at most 2"
        );
        Ok(Self {
            token_in,
            token_out,
            amount,
            swapper,
            slippage_pct,
        })
    }
}

fn parse_address(value: &str, label: &str) -> Result<Address> {
    let invalid = || format!("{label} must be a 0x-prefixed 20-byte EVM address");
    ensure!(
        value.starts_with("0x") && value.len() == 42,
        "{}",
        invalid()
    );
    value.parse().with_context(invalid)
}

async fn diagnose_quote(
    input: &QuoteInput,
    rpc_url: &str,
    api_base: &str,
    api_key: String,
) -> Result<Value> {
    let base = url::Url::parse(api_base).context("UNISWAP_API_BASE is not a valid URL")?;
    ensure!(
        matches!(base.scheme(), "http" | "https")
            && base.has_host()
            && base.query().is_none()
            && base.fragment().is_none()
            && base.username().is_empty()
            && base.password().is_none(),
        "UNISWAP_API_BASE must be an HTTP(S) base URL without credentials, query, or fragment"
    );
    let chain_id = verify_rpc_chain(rpc_url).await?;
    let amount = input.amount.to_string();
    let response = UniswapClient::new(base.into(), Some(api_key))
        .quote_exact_input(
            &format!("{:#x}", input.token_in),
            &format!("{:#x}", input.token_out),
            &amount,
            &format!("{:#x}", input.swapper),
            input.slippage_pct,
        )
        .await?;
    let amount_out = response
        .pointer("/quote/output/amount")
        .and_then(Value::as_str);
    let amount_out = amount_out
        .filter(|amount| !amount.is_empty() && amount.bytes().all(|b| b.is_ascii_digit()))
        .context("Uniswap CLASSIC quote is missing a decimal output amount")?;
    Ok(json!({
        "chainId": chain_id,
        "routing": response["routing"],
        "amountInBaseUnits": amount,
        "amountOutBaseUnits": amount_out,
    }))
}

#[cfg(test)]
mod tests {
    use std::{future::Future, time::Duration};

    use tokio::{
        io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
        net::TcpListener,
        time::timeout,
    };

    use super::*;
    use crate::{CHAIN_ID, NATIVE_ETH_ADDRESS};

    const TOKEN_OUT: &str = "0x1111111111111111111111111111111111111111";
    const SWAPPER: &str = "0x2222222222222222222222222222222222222222";

    fn args() -> Vec<String> {
        [NATIVE_ETH_ADDRESS, TOKEN_OUT, "1000", SWAPPER]
            .map(str::to_owned)
            .to_vec()
    }

    #[test]
    fn rejects_invalid_inputs_independently() {
        for (index, value, expected) in [
            (0, "deadbeef", "TOKEN_IN"),
            (1, NATIVE_ETH_ADDRESS, "must differ"),
            (2, "0", "must be positive"),
            (2, "1.5", "unsigned decimal integer"),
            (2, "0xff", "unsigned decimal integer"),
            (
                2,
                "115792089237316195423570985008687907853269984665640564039457584007913129639936",
                "exceeds uint256",
            ),
            (3, NATIVE_ETH_ADDRESS, "must be nonzero"),
            (3, "0x222222222222222222222222222222222222222g", "SWAPPER"),
            (4, "NaN", "SLIPPAGE_PCT"),
            (4, "0", "SLIPPAGE_PCT"),
            (4, "2.1", "SLIPPAGE_PCT"),
        ] {
            let mut args = args();
            if index == 4 {
                args.push(value.into());
            } else {
                args[index] = value.into();
            }
            let err = QuoteInput::parse(&args).unwrap_err();
            assert!(err.to_string().contains(expected), "{value}: {err}");
        }
    }

    async fn mock_server(
        reply: impl FnOnce(&str, Value) -> Value,
    ) -> (String, impl Future<Output = ()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        (url, async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                assert_ne!(reader.read_line(&mut line).await.unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
                headers.push_str(&line);
            }
            let len = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            let mut body = vec![0; len];
            reader.read_exact(&mut body).await.unwrap();
            let body = reply(&headers, serde_json::from_slice(&body).unwrap()).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            reader
                .get_mut()
                .write_all(response.as_bytes())
                .await
                .unwrap();
        })
    }

    fn rpc_reply(headers: &str, request: Value, chain: u64) -> Value {
        assert!(headers.starts_with("POST / HTTP/1.1\r\n"), "{headers}");
        assert_eq!(request["method"], "eth_chainId");
        assert!(
            request
                .get("params")
                .is_none_or(|params| params == &json!([]))
        );
        json!({"jsonrpc": "2.0", "id": request["id"], "result": format!("{chain:#x}")})
    }

    #[tokio::test]
    async fn checks_chain_and_sends_only_read_only_quote_request() {
        let (rpc, rpc_server) =
            mock_server(|headers, body| rpc_reply(headers, body, CHAIN_ID)).await;
        let (quote, quote_server) = mock_server(|headers, body| {
            assert!(
                headers.starts_with("POST /v1/quote HTTP/1.1\r\n"),
                "{headers}"
            );
            assert!(
                headers
                    .lines()
                    .any(|line| line.eq_ignore_ascii_case("x-api-key: mock-key"))
            );
            assert_eq!(
                body,
                json!({
                    "type": "EXACT_INPUT", "amount": "1000",
                    "tokenInChainId": CHAIN_ID, "tokenOutChainId": CHAIN_ID,
                    "tokenIn": NATIVE_ETH_ADDRESS, "tokenOut": TOKEN_OUT,
                    "swapper": SWAPPER, "slippageTolerance": 0.5,
                })
            );
            json!({"routing": "CLASSIC", "quote": {"output": {"amount": "2000"}}, "permitData": {}})
        })
        .await;
        let input = QuoteInput::parse(&args()).unwrap();
        let quote = format!("{quote}/v1/");
        let ((), (), result) = timeout(Duration::from_secs(5), async {
            tokio::join!(
                rpc_server,
                quote_server,
                diagnose_quote(&input, &rpc, &quote, "mock-key".into())
            )
        })
        .await
        .unwrap();
        assert_eq!(
            result.unwrap(),
            json!({
                "chainId": CHAIN_ID, "routing": "CLASSIC",
                "amountInBaseUnits": "1000", "amountOutBaseUnits": "2000",
            })
        );
    }

    #[tokio::test]
    async fn rejects_unsupported_or_incomplete_quotes() {
        let input = QuoteInput::parse(&args()).unwrap();
        for (body, expected) in [
            (
                json!({"routing": "DUTCH_V2"}),
                "unsupported Uniswap routing",
            ),
            (
                json!({"routing": "CLASSIC"}),
                "missing a decimal output amount",
            ),
        ] {
            let (rpc, rpc_server) =
                mock_server(|headers, body| rpc_reply(headers, body, CHAIN_ID)).await;
            let (quote, quote_server) = mock_server(move |_, _| body).await;
            let ((), (), result) = timeout(Duration::from_secs(5), async {
                tokio::join!(
                    rpc_server,
                    quote_server,
                    diagnose_quote(&input, &rpc, &quote, "mock-key".into())
                )
            })
            .await
            .unwrap();
            let err = result.unwrap_err();
            assert!(err.to_string().contains(expected), "{err}");
        }
    }

    #[tokio::test]
    async fn chain_mismatch_stops_before_quote() {
        let (rpc, rpc_server) = mock_server(|headers, body| rpc_reply(headers, body, 1)).await;
        let quote_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let quote = format!("http://{}", quote_listener.local_addr().unwrap());
        let input = QuoteInput::parse(&args()).unwrap();
        let ((), result) = timeout(Duration::from_secs(5), async {
            tokio::join!(
                rpc_server,
                diagnose_quote(&input, &rpc, &quote, "mock-key".into())
            )
        })
        .await
        .unwrap();
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("RPC chain id mismatch")
        );
        assert!(
            timeout(Duration::from_millis(50), quote_listener.accept())
                .await
                .is_err()
        );
    }
}
