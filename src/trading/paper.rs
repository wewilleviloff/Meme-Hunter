use anyhow::{Result, ensure};
use rust_decimal::Decimal;

use crate::{
    config::AppConfig,
    storage::{PositionRow, Store},
};

#[derive(Debug, PartialEq, Eq)]
pub enum OpenPaperOutcome {
    Opened(String),
    Rejected(&'static str),
    NotClaimed,
}

pub(crate) struct PaperEntry<'a> {
    pub address: &'a str,
    pub symbol: &'a str,
    pub price: Decimal,
    pub quantity: Decimal,
    pub notional: Decimal,
    pub fee: Decimal,
    pub gas: Decimal,
    pub cost_basis: Decimal,
}

pub(crate) struct PaperLimits {
    pub max_open: u32,
    pub max_entries: u32,
    pub max_daily_loss: Decimal,
}

pub(crate) struct PaperExit<'a> {
    pub price: Decimal,
    pub proceeds: Decimal,
    pub fee: Decimal,
    pub gas: Decimal,
    pub pnl: Decimal,
    pub reason: &'a str,
}

pub async fn open_paper_position(
    store: &Store,
    cfg: &AppConfig,
    address: &str,
    symbol: &str,
    price: Decimal,
) -> Result<OpenPaperOutcome> {
    ensure!(
        price > Decimal::ZERO,
        "cannot open paper position at non-positive price"
    );
    let notional = cfg.buy_usd;
    let quantity = notional / price;
    let fee = notional * cfg.paper_buy_fee_pct / Decimal::from(100);
    let gas = cfg.paper_gas_usd_per_side;
    let cost_basis = notional + fee + gas;
    store
        .open_position(
            PaperEntry {
                address,
                symbol,
                price,
                quantity,
                notional,
                fee,
                gas,
                cost_basis,
            },
            PaperLimits {
                max_open: cfg.max_open_positions,
                max_entries: cfg.max_trades_per_day,
                max_daily_loss: cfg.max_daily_loss_usd,
            },
        )
        .await
}

pub async fn close_paper_position(
    store: &Store,
    cfg: &AppConfig,
    p: &PositionRow,
    exit_price: Decimal,
    reason: &str,
) -> Result<bool> {
    ensure!(
        exit_price > Decimal::ZERO,
        "cannot close paper position at non-positive price"
    );
    let proceeds = p.quantity * exit_price;
    let fee = proceeds * cfg.paper_sell_fee_pct / Decimal::from(100);
    let gas = cfg.paper_gas_usd_per_side;
    let pnl = proceeds - p.cost_basis_usd - fee - gas;
    store
        .close_position(
            p,
            PaperExit {
                price: exit_price,
                proceeds,
                fee,
                gas,
                pnl,
                reason,
            },
        )
        .await
}
