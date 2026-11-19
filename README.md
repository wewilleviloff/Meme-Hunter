# Meme Hunter — Rust PAPER MVP

Meme Hunter scans new tokens on Robinhood Chain (chain ID 4663) and simulates positions. It cannot sign, submit, or broadcast swaps.

## Build and test

A clean checkout contains the Rust sources. Open the checkout in VS Code with rust-analyzer. Rustup selects Rust 1.94.1 with rustfmt and Clippy from `rust-toolchain.toml` (on Windows use MSVC Rust with the Visual C++ build tools and Windows SDK).

```powershell
cargo build --locked
cargo test --locked --all-targets
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
```

`bundle/` is a historical starter snapshot, not used for builds or CI. `restore-mvp.ps1` only restores it into a separate tree without existing source files.

## Run locally

```powershell
Copy-Item .env.example .env
# Set CRIUS_API_KEY with the stream scope. Leave Telegram variables empty if unused.
cargo run --locked
```

At startup the CLI explicitly selects the Rustls `ring` provider because its dependencies enable both `ring` and `aws-lc-rs`.

Crius discovers launches and graduations over WebSocket; REST supplies screening data and a price fallback for open positions. An entry requires a known launch time, a graduated token and sufficient quote-side liquidity, price confidence, holder distribution, realized tax samples and security data. Unknown required screening fields block entry. PAPER defaults are a $10 entry, take profit at 2×, stop loss at −35%, and minimum quote-side liquidity of $10,000. These and the position, trade, fee and risk limits are configurable in `.env.example`. Daily counts and realized PnL use UTC. SQLite stores events, positions and paper trades.

## Telegram controls (optional)

Set `TELEGRAM_BOT_TOKEN` and `TELEGRAM_ALLOWED_USER_ID` together. Only the configured user can use `/help`, `/status`, `/positions`, `/pause`, `/resume`, and `/panic`. `/pause` and `/panic` pause new entries without closing open positions; price monitoring continues. The pause is in memory and is cleared by a restart. `/positions` displays the configured exit thresholds.

## Read-only quote diagnostics

Set `RPC_URL` and `UNISWAP_API_KEY`; `UNISWAP_API_BASE` is optional. Supply public token and swapper addresses and an exact input in base units:

```powershell
cargo run --locked -- diagnose-quote TOKEN_IN TOKEN_OUT AMOUNT_BASE_UNITS SWAPPER
```

Add an optional slippage percent (default 0.5, maximum 2). The command checks the RPC chain ID with Alloy and requests an Uniswap CLASSIC quote. It does not initialize the PAPER runtime or write to SQLite, and never requests transaction construction. Live quote availability on chain 4663 depends on the configured provider and API access.

## Limitations

PAPER is the only supported mode. No private key is needed. Never commit `.env` or put credentials in command arguments. WebSocket replay within a running process is limited to Crius’s replay window; recovery is not a complete historical backfill. Paper fills are estimates; they do not model execution, failed swaps, market impact, MEV, gas spikes, or guaranteed exits. This is experimental software, not financial advice.
