use anyhow::{Context, Result, ensure};
use rust_decimal::Decimal;
use std::{env, str::FromStr};

#[derive(Clone)]
pub struct AppConfig {
    pub bot_mode: String,
    pub database_url: String,
    pub crius_rest_base: String,
    pub crius_ws_url: String,
    pub crius_api_key: Option<String>,
    pub telegram_bot_token: Option<String>,
    pub telegram_allowed_user_id: Option<u64>,
    pub buy_usd: Decimal,
    pub take_profit_multiple: Decimal,
    pub stop_loss_pct: Decimal,
    pub min_liquidity_usd: Decimal,
    pub min_token_age_seconds: i64,
    pub max_token_age_seconds: i64,
    pub min_security_score: u8,
    pub max_buy_tax_pct: Decimal,
    pub max_sell_tax_pct: Decimal,
    pub min_realized_sell_samples: u32,
    pub min_holders: u64,
    pub max_top10_holder_pct: Decimal,
    pub max_dev_holder_pct: Decimal,
    pub min_volume_5m_usd: Decimal,
    pub max_open_positions: u32,
    pub max_trades_per_day: u32,
    pub max_daily_loss_usd: Decimal,
    pub paper_buy_fee_pct: Decimal,
    pub paper_sell_fee_pct: Decimal,
    pub paper_gas_usd_per_side: Decimal,
    pub price_poll_seconds: u64,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let cfg = Self {
            bot_mode: get("BOT_MODE", "PAPER").to_ascii_uppercase(),
            database_url: get("DATABASE_URL", "sqlite://data/meme_hunter.db"),
            crius_rest_base: get("CRIUS_REST_BASE", "https://api.crius.sh/v1/robinhood"),
            crius_ws_url: get("CRIUS_WS_URL", "wss://api.crius.sh/v1/robinhood/stream"),
            crius_api_key: optional("CRIUS_API_KEY"),
            telegram_bot_token: optional("TELEGRAM_BOT_TOKEN"),
            telegram_allowed_user_id: optional("TELEGRAM_ALLOWED_USER_ID")
                .map(|v| v.parse())
                .transpose()
                .context("TELEGRAM_ALLOWED_USER_ID must be an integer")?,
            buy_usd: decimal("BUY_USD", "10")?,
            take_profit_multiple: decimal("TAKE_PROFIT_MULTIPLE", "2.0")?,
            stop_loss_pct: decimal("STOP_LOSS_PCT", "35")?,
            min_liquidity_usd: decimal("MIN_LIQUIDITY_USD", "10000")?,
            min_token_age_seconds: integer("MIN_TOKEN_AGE_SECONDS", 60)?,
            max_token_age_seconds: integer("MAX_TOKEN_AGE_SECONDS", 1800)?,
            min_security_score: integer::<u8>("MIN_SECURITY_SCORE", 90)
                .context("MIN_SECURITY_SCORE must be 0..=100")?,
            max_buy_tax_pct: decimal("MAX_BUY_TAX_PCT", "2")?,
            max_sell_tax_pct: decimal("MAX_SELL_TAX_PCT", "2")?,
            min_realized_sell_samples: integer("MIN_REALIZED_SELL_SAMPLES", 3)?,
            min_holders: integer("MIN_HOLDERS", 30)?,
            max_top10_holder_pct: decimal("MAX_TOP10_HOLDER_PCT", "55")?,
            max_dev_holder_pct: decimal("MAX_DEV_HOLDER_PCT", "5")?,
            min_volume_5m_usd: decimal("MIN_VOLUME_5M_USD", "500")?,
            max_open_positions: integer("MAX_OPEN_POSITIONS", 2)?,
            max_trades_per_day: integer("MAX_TRADES_PER_DAY", 5)?,
            max_daily_loss_usd: decimal("MAX_DAILY_LOSS_USD", "10")?,
            paper_buy_fee_pct: decimal("PAPER_BUY_FEE_PCT", "1")?,
            paper_sell_fee_pct: decimal("PAPER_SELL_FEE_PCT", "1")?,
            paper_gas_usd_per_side: decimal("PAPER_GAS_USD_PER_SIDE", "0.10")?,
            price_poll_seconds: integer("PRICE_POLL_SECONDS", 20)?,
        };
        ensure!(
            cfg.bot_mode == "PAPER",
            "only BOT_MODE=PAPER is supported; live execution is disabled"
        );
        ensure!(cfg.buy_usd > Decimal::ZERO, "BUY_USD must be positive");
        ensure!(
            cfg.take_profit_multiple > Decimal::ONE,
            "TAKE_PROFIT_MULTIPLE must exceed 1"
        );
        ensure!(
            cfg.stop_loss_pct > Decimal::ZERO && cfg.stop_loss_pct < Decimal::from(100),
            "STOP_LOSS_PCT must be between 0 and 100"
        );
        ensure!(
            cfg.min_liquidity_usd > Decimal::ZERO,
            "MIN_LIQUIDITY_USD must be positive"
        );
        ensure!(
            cfg.min_security_score <= 100,
            "MIN_SECURITY_SCORE must be 0..=100"
        );
        ensure!(
            cfg.min_token_age_seconds >= 0
                && cfg.max_token_age_seconds >= cfg.min_token_age_seconds,
            "invalid token age range"
        );
        ensure!(
            cfg.max_open_positions > 0
                && cfg.max_trades_per_day > 0
                && cfg.max_daily_loss_usd > Decimal::ZERO,
            "trade limits must be positive"
        );
        for (name, value) in [
            ("MAX_BUY_TAX_PCT", cfg.max_buy_tax_pct),
            ("MAX_SELL_TAX_PCT", cfg.max_sell_tax_pct),
            ("MAX_TOP10_HOLDER_PCT", cfg.max_top10_holder_pct),
            ("MAX_DEV_HOLDER_PCT", cfg.max_dev_holder_pct),
            ("PAPER_BUY_FEE_PCT", cfg.paper_buy_fee_pct),
            ("PAPER_SELL_FEE_PCT", cfg.paper_sell_fee_pct),
        ] {
            ensure!(
                (Decimal::ZERO..=Decimal::from(100)).contains(&value),
                "{name} must be in 0..=100"
            );
        }
        ensure!(
            cfg.min_volume_5m_usd >= Decimal::ZERO && cfg.paper_gas_usd_per_side >= Decimal::ZERO,
            "volume and gas assumptions cannot be negative"
        );
        ensure!(
            cfg.telegram_bot_token.is_some() == cfg.telegram_allowed_user_id.is_some(),
            "set both Telegram token and allowed user ID, or leave both empty"
        );
        Ok(cfg)
    }
}

fn get(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_string())
}
fn optional(name: &str) -> Option<String> {
    env::var(name).ok().filter(|s| !s.trim().is_empty())
}
fn decimal(name: &str, default: &str) -> Result<Decimal> {
    Decimal::from_str(&get(name, default))
        .with_context(|| format!("{name} must be a decimal number"))
}
fn integer<T: FromStr>(name: &str, default: i64) -> Result<T>
where
    T::Err: std::fmt::Display,
{
    get(name, &default.to_string())
        .parse()
        .map_err(|e| anyhow::anyhow!("{name}: {e}"))
}

#[cfg(test)]
impl AppConfig {
    pub(crate) fn test_config() -> AppConfig {
        AppConfig {
            bot_mode: "PAPER".into(),
            database_url: String::new(),
            crius_rest_base: String::new(),
            crius_ws_url: String::new(),
            crius_api_key: None,
            telegram_bot_token: None,
            telegram_allowed_user_id: None,
            buy_usd: "10".parse().unwrap(),
            take_profit_multiple: "2".parse().unwrap(),
            stop_loss_pct: "35".parse().unwrap(),
            min_liquidity_usd: "10000".parse().unwrap(),
            min_token_age_seconds: 60,
            max_token_age_seconds: 1800,
            min_security_score: 90,
            max_buy_tax_pct: "2".parse().unwrap(),
            max_sell_tax_pct: "2".parse().unwrap(),
            min_realized_sell_samples: 3,
            min_holders: 30,
            max_top10_holder_pct: "55".parse().unwrap(),
            max_dev_holder_pct: "5".parse().unwrap(),
            min_volume_5m_usd: "500".parse().unwrap(),
            max_open_positions: 2,
            max_trades_per_day: 5,
            max_daily_loss_usd: "10".parse().unwrap(),
            paper_buy_fee_pct: "1".parse().unwrap(),
            paper_sell_fee_pct: "1".parse().unwrap(),
            paper_gas_usd_per_side: "0.10".parse().unwrap(),
            price_poll_seconds: 20,
        }
    }
}
