use std::sync::Arc;

use rust_decimal::Decimal;
use teloxide::{prelude::*, types::Message};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::{
    scanner::engine::EngineControl,
    storage::{PositionRow, Store},
    strategy::rules::exit_thresholds,
};

pub async fn run(
    token: String,
    allowed_user_id: u64,
    store: Store,
    control: Arc<RwLock<EngineControl>>,
    take_profit_multiple: Decimal,
    stop_loss_pct: Decimal,
) -> anyhow::Result<()> {
    let bot = Bot::new(token);
    info!(allowed_user_id, "Telegram control bot started");
    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let store=store.clone();
        let control=control.clone();
        async move {
            let Some(user)=msg.from.as_ref() else { return respond(()); };
            if user.id.0 != allowed_user_id { return respond(()); }
            let text=msg.text().unwrap_or("").trim();
            let reply=match text {
                "/start" | "/help" => "Meme Hunter (PAPER)\n/status — state and daily PnL\n/positions — open positions\n/pause — stop new entries\n/resume — resume PAPER entries\n/panic — pause new entries (does not guarantee an on-chain sale)".to_string(),
                "/status" => {
                    let Some((open,trades,pnl)) = paper_data(&bot, &msg, store.summary().await).await? else { return respond(()); };
                    let paused=control.read().await.paused;
                    format!("Mode: PAPER\nPaused: {paused}\nOpen positions: {open}\nEntries today: {trades}\nRealized PnL today: ${pnl}")
                }
                "/positions" => {
                    let Some(positions) = paper_data(&bot, &msg, store.open_positions().await).await? else { return respond(()); };
                    positions_reply(&positions, take_profit_multiple, stop_loss_pct)
                }
                "/pause" => { control.write().await.paused=true; "Paused: no new entries. Existing paper positions continue to be monitored.".into() }
                "/resume" => { control.write().await.paused=false; "Resumed PAPER scanning. LIVE mode remains disabled.".into() }
                "/panic" => { control.write().await.paused=true; "PANIC: new entries paused. This MVP is paper-only; this command is not an on-chain sell guarantee.".into() }
                _ => "Unknown command. Send /help.".into(),
            };
            bot.send_message(msg.chat.id,reply).await?;
            respond(())
        }
    }).await;
    Ok(())
}

fn positions_reply(
    positions: &[PositionRow],
    take_profit_multiple: Decimal,
    stop_loss_pct: Decimal,
) -> String {
    if positions.is_empty() {
        return "No open paper positions.".into();
    }
    positions
        .iter()
        .map(|p| {
            let (tp, sl) = exit_thresholds(p.entry_price_usd, take_profit_multiple, stop_loss_pct);
            format!(
                "{} ({})\nEntry: ${}\nTP: ${}\nSL: ${}\nQty: {}",
                p.symbol, p.token_address, p.entry_price_usd, tp, sl, p.quantity
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

async fn paper_data<T>(
    bot: &Bot,
    msg: &Message,
    result: anyhow::Result<T>,
) -> Result<Option<T>, teloxide::RequestError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) => {
            warn!(%error, "could not load paper trading data for Telegram");
            bot.send_message(
                msg.chat.id,
                "Paper trading data is temporarily unavailable.",
            )
            .await?;
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::{ExitSignal, evaluate_exit};

    #[test]
    fn positions_reply_uses_configured_exit_levels() {
        let entry = Decimal::from(4);
        let multiple = Decimal::new(15, 1);
        let loss_pct = Decimal::from(25);
        let reply = positions_reply(
            &[PositionRow {
                id: "id".into(),
                token_address: "address".into(),
                symbol: "TKN".into(),
                entry_price_usd: entry,
                quantity: Decimal::ONE,
                cost_basis_usd: entry,
                highest_price_usd: entry,
                opened_at_ms: 0,
            }],
            multiple,
            loss_pct,
        );
        let displayed = |label: &str| -> Decimal {
            reply
                .lines()
                .find_map(|line| line.strip_prefix(label))
                .unwrap()
                .parse()
                .unwrap()
        };
        let tp = displayed("TP: $");
        let sl = displayed("SL: $");
        assert_eq!(tp, Decimal::from(6));
        assert_eq!(sl, Decimal::from(3));
        assert_eq!(
            evaluate_exit(entry, tp, multiple, loss_pct),
            Some(ExitSignal::TakeProfit)
        );
        assert_eq!(
            evaluate_exit(entry, sl, multiple, loss_pct),
            Some(ExitSignal::StopLoss)
        );
    }
}
