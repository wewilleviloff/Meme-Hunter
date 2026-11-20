---
name: figma-dashboard-implementation
description: Implement or refine the Meme Hunter web dashboard from its Figma design. Use when building the dashboard UI, connecting its read-only data views to the Rust PAPER runtime, matching Figma layout or styles, or fixing visual differences between the implementation and Figma.
argument-hint: "[Figma URL, dashboard section, or requested change]"
---

# Figma Dashboard Implementation

Implement the Meme Hunter dashboard as a real, data-driven web interface that follows the approved Figma design. Treat this skill as the project-specific workflow for translating the dashboard into code.

## Source of truth

- Figma file: [Meme Hunter — Trading Dashboard](https://www.figma.com/design/D239oxvHj8UNTZ3skXwMoc/Meme-Hunter-%E2%80%94-Trading-Dashboard?node-id=6-219)
- Target frame: `6:219` — `Meme Hunter — Trading Dashboard`
- Canvas: 1440 × 1240 px.
- The Figma file contains editable layers and reusable components. Recreate the interface with semantic HTML, CSS, and application components. **Never use a screenshot of the whole Figma screen as the implementation**, a page background, or a substitute for editable UI.
- If the user supplies another Figma URL or explicitly changes the source frame, use that newer source for the requested task. Do not silently replace the approved design.

## Project-specific constraints

Read the current repository before making changes. The following describes the initial repository state and may evolve:

- Meme Hunter is a Rust 2024 application targeting Rust 1.94.1.
- The current runtime is PAPER-only. Live swaps, transaction signing, and transaction broadcasting are not supported.
- The runtime uses Tokio, Crius REST/WebSocket, SQLx with SQLite, and Telegram controls.
- There is currently no web server or frontend framework in `Cargo.toml`.
- The current storage layer exposes `Store::open_positions()`, `Store::open_position_addresses()`, and `Store::summary()`. `summary()` returns open-position count, today's entry count, and today's realized PnL — it does **not** return account equity or starting balance.
- `PositionRow` currently includes token address, symbol, entry price, quantity, cost basis, highest observed price, and open timestamp. It does **not** itself contain the current market price.
- The SQLite schema has `positions`, `trades`, `token_events`, and `runtime_state`. The `trades` table stores BUY/SELL events. There is no ready-made public history query on `Store` in the initial implementation.
- Current Crius stream and REST activity is not yet exposed as a complete dashboard health model.

These are repository facts, not permanent assumptions. Recheck the actual code before implementing; do not rely on this list if the project has changed.

### Safety invariants

1. The dashboard is **read-only** for the initial MVP. It may display status, positions, history, balances that can be calculated from persisted facts, and health information. It must not place, close, or modify trades.
2. Never add, enable, or imply LIVE trading. Never add wallet signing, private-key access, swap submission, transaction broadcasting, or trade controls as part of dashboard work.
3. Never expose `.env`, API keys, Telegram bot tokens, RPC credentials, private keys, or other secrets to the browser, logs, JSON responses, or source files.
4. Do not bind the dashboard to a public interface such as `0.0.0.0` by default. Default to localhost. Public/VPS exposure requires an explicit user decision and an authentication/TLS plan.
5. Do not silently change scanner thresholds, PAPER sizing, position limits, fees, risk limits, or strategy behavior to support the UI.
6. UI actions must not suggest that a backend operation succeeded unless a real, successful backend response confirms it. For this read-only MVP, navigation and filters are allowed; trade mutation actions are not.

## Required workflow

### 1. Inspect the current codebase

Before coding:

- Read `README.md`, `Cargo.toml`, `src/main.rs`, `src/lib.rs`, `src/storage/store.rs`, `src/trading/paper.rs`, `src/config/mod.rs`, and the relevant migrations.
- Inspect the actual startup and task lifecycle in `src/main.rs`. Preserve the existing runtime, Ctrl+C shutdown behavior, and background tasks.
- Identify the source of current token prices and Crius connection state. Do not confuse `highest_price_usd` with the current market price.
- Check whether a frontend, HTTP server, API DTO, or health registry has been added since this skill was written. Reuse existing patterns rather than creating parallel systems.
- Avoid broad refactors. Keep dashboard work separate from scanner, strategy, and paper-execution logic.

### 2. Inspect the Figma frame

Use the available Figma MCP integration, when available, to inspect frame `6:219`:

1. Read frame metadata and layer hierarchy.
2. Inspect reusable components and text properties where useful.
3. Capture a screenshot of the target frame for visual comparison.
4. Confirm typography, spacing, dimensions, colors, and component states from the actual file before coding.

If the Figma integration is unavailable, use the Figma link and the design tokens below as a starting point. Do not claim to have inspected live Figma properties when you could not access them. Ask for an exported frame or relevant screenshots only when essential details cannot be determined.

### 3. Choose the smallest maintainable web architecture

The repository starts as a Rust-only application. Unless the repository has since adopted a different frontend architecture, prefer a small dashboard served by the existing Rust process:

- A dedicated Rust web module, separate from scanner and paper trading code.
- A small Tokio-compatible HTTP layer (Axum is a reasonable default if no HTTP framework exists; verify current crate compatibility before adding it).
- Static frontend assets kept in a clear directory such as `web/` and served by the Rust process. Avoid requiring a separate Node server or build pipeline for the first dashboard unless the requested UI complexity makes one necessary.
- Read-only JSON endpoints with explicit response DTOs. Do not serialize internal structs wholesale.
- A simple polling loop from the browser is sufficient for the first version (roughly every 2–5 seconds). Add SSE/WebSocket only when an actual requirement justifies the extra lifecycle and reconnection complexity.

Keep the server startup and graceful shutdown integrated with the existing Tokio runtime. Do not block the scanner while serving HTTP requests.

If another architecture is already present, follow it. Do not introduce a framework solely because this skill names it as a reasonable default.

### 4. Define real data contracts before styling

Map every visible field to a real source. Create explicit DTOs for the dashboard; do not return database rows directly.

Expected read models:

- **Dashboard summary:** PAPER mode, count of open positions, today's closed-trade count or entry count (label the chosen metric accurately), realized PnL today, and last refresh time.
- **Open positions:** symbol, token address, entry price, current/mark price, quantity or invested amount, unrealized net PnL, percentage return, take-profit price, stop-loss price, and opened-at time.
- **Trade history:** one row per completed position / round trip, with token, opened-at, closed-at, duration, exit reason, and realized net PnL. Use the persisted `positions` and `trades` data consistently; document the accounting basis.
- **Service health:** Rust runtime, Crius WebSocket, Crius REST API, and SQLite. Include state and last-success/heartbeat timestamps where meaningful.

Important data rules:

- Current mark price must come from the latest observed price state or an explicit persisted price snapshot. Never substitute entry price, highest price, or a placeholder.
- If no mark price is known, return `null` and render “—” / “Price unavailable”; do not invent a value.
- Do not infer equity from the daily PnL or open-position count. There is no initial paper balance in the initial configuration. Add a configurable starting balance only if the user approves or the project requirements establish it; otherwise omit that KPI or label it unavailable.
- Display precise financial values safely. Keep `Decimal` arithmetic in Rust. Prefer decimal strings in JSON for prices, quantities, fees, and PnL; do not use JavaScript floating-point arithmetic to calculate PnL.
- A completed position is not the same thing as a single trade row: the database records BUY and SELL legs. Aggregate the pair into one history row when matching the Figma design.
- Use UTC timestamps from persisted millisecond fields and label the display timezone clearly. Convert for local display only deliberately and consistently.
- A missing value is not zero. A missing health signal is not “healthy”.
- Keep API errors structured and sanitized. Never send raw SQL errors, credentials, or stack traces to the browser.

### 5. Implement operational health truthfully

The health panel must show observed state, not permanent green placeholders.

- **Rust runtime:** update a heartbeat while the dashboard/runtime is actually operating.
- **Crius WebSocket:** track connection, disconnection, reconnecting, and last successful message/heartbeat based on the real stream lifecycle.
- **Crius REST API:** track the last successful request and a sanitized failure state; do not perform aggressive health probes.
- **SQLite:** verify the database with a lightweight query and expose the last successful check.

Use a small shared health-state abstraction with safe concurrency. State transitions must be updated at the real connection/request boundaries. Do not create one health monitor per browser request.

A service with unknown state should display “Unknown” or “Checking”, not “Connected”. A stale last-success timestamp should not remain green indefinitely.

### 6. Recreate the Figma visual system

Use these inspected design tokens as the baseline. Reconfirm against the Figma file if it has changed.

| Role | Value |
|---|---|
| Canvas | `#0A0F14` |
| Sidebar | `#0D141C` |
| Surface / panel | `#111A23` |
| Raised surface | `#17232E` |
| Border | `#263544` |
| Primary text | `#E8EEF4` |
| Muted text | `#8C9AA8` |
| Faint text | `#637384` |
| Accent / teal | `#62D6C0` |
| Positive | `#3BD49A` |
| Negative | `#F07878` |

Typography:

- Geist for headings, labels, and regular UI copy.
- Geist Mono for timestamps, addresses, prices, quantities, and PnL.
- Preserve the type scale and restrained visual hierarchy from Figma. Load the font locally or use a robust fallback stack; do not make the UI depend on a third-party font request at runtime without a deliberate decision.

Composition and baseline measurements:

- Desktop frame: 1440 × 1240 px.
- Sidebar: 220 px wide.
- Main content: 1220 px wide with approximately 28 px inner horizontal padding.
- Main content order: page header → runtime status banner → four KPI cards → open positions → trade history → system health.
- Use compact dark panels, subtle one-pixel borders, rounded corners, restrained spacing, teal positive/active states, and red negative PnL.
- Keep rows aligned to consistent columns. Prices and PnL should use tabular/monospaced numerals.
- Use clear status labels as well as color; color alone must not convey service state or trade outcome.

### 7. Implement each dashboard section

**Header**
- Page title “Overview” and short description.
- Explicit PAPER mode indicator.
- Show actual runtime timestamp or last refresh time.
- The mockup's “DEMO DATA” pill is a placeholder. Replace it with a truthful data-source label or omit it.

**Runtime banner**
- Runtime state, short explanatory text, and actual last heartbeat.
- Do not hardcode “RUNNING” or “2s ago”.

**KPI cards**
- Only show metrics with a verified calculation and source.
- Use labels that accurately describe the metric window and accounting basis.
- Never copy sample values such as `$124.80`, `+$1.03`, or `+$12.48` from the mockup.

**Open positions**
- Match the Figma columns: token, entry, last price, invested, net PnL, take-profit / stop-loss, and open duration.
- Show a useful empty state when there are no positions.
- Handle long symbols and addresses without breaking the table layout; use truncation with a way to inspect/copy the full address if appropriate.
- Indicate missing/stale prices visibly.

**Trade history**
- Match the Figma columns: token, opened, closed, duration, exit reason, and net PnL.
- Sort newest closed positions first.
- Include a useful empty state and a bounded default page size. Add pagination only if the dataset or user request warrants it.

**System health**
- Show Rust runtime, Crius WebSocket, Crius REST API, and SQLite.
- Make status text and timestamps truthful and readable.
- Distinguish “Connected”, “Degraded”, “Disconnected”, “Checking”, and “Unknown” where appropriate.

### 8. Responsive behavior and accessibility

- Preserve the 1440 px desktop composition as the primary visual target.
- On tablet/mobile, collapse or hide the sidebar behind a clear navigation affordance, stack KPI cards, and allow dense tables to scroll horizontally.
- Do not silently drop important financial columns on narrow screens.
- Use semantic headings, buttons, tables, labels, and accessible focus states.
- Meet reasonable contrast; positive/negative colors must remain readable against dark surfaces.
- Respect reduced-motion preferences. Avoid decorative animation that makes operational status harder to read.

### 9. Verify functionality and visual fidelity

After implementation:

1. Run the repository's documented checks:
   ```powershell
   cargo fmt --all -- --check
   cargo test --locked --all-targets
   cargo clippy --locked --all-targets --all-features -- -D warnings
   cargo build --locked
   ```
2. Add tests for new DTO calculations and database queries, including empty datasets, missing prices, closed positions, malformed persisted decimals, and UTC day boundaries where relevant.
3. Verify that the dashboard starts and stops with the existing runtime and that the scanner continues working while HTTP requests are served.
4. Verify the API does not expose secrets or mutate positions/trades.
5. Capture the implemented page at 1440 px and compare it with Figma frame `6:219`. Fix meaningful differences in spacing, sizing, typography, alignment, colors, and clipping.
6. Inspect at least one narrow viewport. Check loading, stale-data, API-error, empty-position, and empty-history states.
7. Report tests actually run. Do not claim visual parity without a screenshot comparison.

## Completion report

When the task is complete, summarize:

- Files/modules changed.
- How the web server and dashboard data flow are wired.
- Which Figma sections are implemented.
- Which metrics are sourced from real data and which are intentionally omitted because the backend does not yet provide them.
- Test/build results.
- Any remaining visual or functional gaps.

Do not stop after creating static HTML with hardcoded sample numbers. A successful implementation is a working, read-only dashboard connected to the actual PAPER runtime and visually checked against the Figma frame.
