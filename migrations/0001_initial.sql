PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS token_events (
  address TEXT PRIMARY KEY,
  launchpad TEXT,
  launch_ts_ms INTEGER NOT NULL,
  graduation_ts_ms INTEGER,
  status TEXT NOT NULL DEFAULT 'launched',
  screened INTEGER NOT NULL DEFAULT 0,
  accepted INTEGER NOT NULL DEFAULT 0,
  reject_reason TEXT
);
CREATE TABLE IF NOT EXISTS positions (
  id TEXT PRIMARY KEY,
  token_address TEXT NOT NULL,
  symbol TEXT NOT NULL,
  entry_price_usd TEXT NOT NULL,
  quantity TEXT NOT NULL,
  cost_basis_usd TEXT NOT NULL,
  highest_price_usd TEXT NOT NULL,
  opened_at_ms INTEGER NOT NULL,
  status TEXT NOT NULL,
  exit_price_usd TEXT,
  closed_at_ms INTEGER,
  realized_pnl_usd TEXT,
  exit_reason TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS uq_one_open_position_per_token ON positions(token_address) WHERE status='OPEN';
CREATE INDEX IF NOT EXISTS idx_positions_status ON positions(status);
CREATE TABLE IF NOT EXISTS trades (
  id TEXT PRIMARY KEY,
  position_id TEXT,
  token_address TEXT NOT NULL,
  symbol TEXT NOT NULL,
  side TEXT NOT NULL,
  price_usd TEXT NOT NULL,
  notional_usd TEXT NOT NULL,
  fee_usd TEXT NOT NULL,
  gas_usd TEXT NOT NULL,
  pnl_usd TEXT,
  ts_ms INTEGER NOT NULL,
  mode TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_trades_ts ON trades(ts_ms);
CREATE TABLE IF NOT EXISTS runtime_state (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at_ms INTEGER NOT NULL
);
