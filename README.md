# Meme Hunter — Rust MVP

Meme Hunter is a paper-trading-first token scanner for Robinhood Chain (chain ID 4663). The repo stores the complete starter source bundle in `bundle/`; restore it once before opening the Rust workspace.

## Restore the source tree (Windows / PowerShell)

```powershell
git clone https://github.com/wewilleviloff/Meme-Hunter.git
cd Meme-Hunter
.\restore-mvp.ps1
code .
```

The restore script concatenates the numbered base64 parts, verifies the SHA-256 of the archive, and extracts the Rust project into the repository root. Linux/macOS users can concatenate the `bundle/meme-hunter-src.tar.gz.b64.part*` files, decode them with `base64 -d`, verify the same SHA-256, and extract with `tar -xzf`.

## Locked strategy

- Robinhood Chain, chain ID `4663`
- Paper entry: $10
- Take profit: 2.0x
- Stop loss: -35%
- Minimum quote-side liquidity: $10,000
- Conservative fail-closed screening
- SQLite persistence
- Crius REST + WebSocket discovery
- Telegram controls
- Uniswap Trading API quote adapter + Alloy EVM validation

## Run locally

Requires Rust 1.94.1 (pinned in `rust-toolchain.toml`) and VS Code with rust-analyzer.

```powershell
Copy-Item .env.example .env
# Add CRIUS_API_KEY and optionally Telegram credentials.
cargo check
cargo test
cargo fmt --check
cargo run
```

Set up Crius API credentials with the stream scope at https://dashboard.crius.sh/. Telegram is optional.

## Safety boundary

**PAPER is the only supported runtime mode in this MVP.** Startup rejects LIVE mode. The Uniswap component requests and validates quotes; it does not sign, submit, or broadcast swaps. No private key is needed for Paper Trading. Keep `.env` local and never commit credentials.

This is an experimental software scaffold, not a profitable strategy or financial advice. Paper fills are estimates and do not replicate router execution, failed swaps, market impact, MEV, gas spikes, token hooks, or guaranteed exits. A stop-loss cannot guarantee execution.

See the full project README after restoring the source tree for architecture, filters, limitations, environment variables, and Telegram commands.
