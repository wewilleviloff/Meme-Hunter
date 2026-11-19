use anyhow::{Context, Result, ensure};
use rust_decimal::Decimal;
use sqlx::{
    Row, SqliteConnection, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::{path::Path, str::FromStr, time::Duration};
use uuid::Uuid;

use crate::trading::paper::{OpenPaperOutcome, PaperEntry, PaperExit, PaperLimits};

struct UtcDay {
    start_ms: i64,
    end_ms: i64,
}

impl UtcDay {
    fn at(now: chrono::DateTime<chrono::Utc>) -> Self {
        let start = now.date_naive().and_time(chrono::NaiveTime::MIN).and_utc();
        Self {
            start_ms: start.timestamp_millis(),
            end_ms: (start + chrono::Days::new(1)).timestamp_millis(),
        }
    }
}

async fn count_open_positions(conn: &mut SqliteConnection) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT COUNT(*) FROM positions WHERE status='OPEN'")
            .fetch_one(conn)
            .await?,
    )
}

async fn count_entries(conn: &mut SqliteConnection, day: &UtcDay) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM trades WHERE mode='PAPER' AND side='BUY' AND ts_ms>=? AND ts_ms<?",
    )
    .bind(day.start_ms)
    .bind(day.end_ms)
    .fetch_one(conn)
    .await?)
}

async fn realized_pnl(conn: &mut SqliteConnection, day: &UtcDay) -> Result<Decimal> {
    let values: Vec<String> = sqlx::query_scalar(
        "SELECT pnl_usd FROM trades WHERE mode='PAPER' AND side='SELL' AND ts_ms>=? AND ts_ms<? AND pnl_usd IS NOT NULL",
    )
    .bind(day.start_ms)
    .bind(day.end_ms)
    .fetch_all(conn)
    .await?;
    values.into_iter().try_fold(Decimal::ZERO, |total, value| {
        let pnl = Decimal::from_str(&value).context("invalid persisted PAPER PnL")?;
        total.checked_add(pnl).context("PAPER PnL sum overflow")
    })
}

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

#[derive(Debug, Clone)]
pub struct PositionRow {
    pub id: String,
    pub token_address: String,
    pub symbol: String,
    pub entry_price_usd: Decimal,
    pub quantity: Decimal,
    pub cost_basis_usd: Decimal,
    pub highest_price_usd: Decimal,
    pub opened_at_ms: i64,
}

impl Store {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let url = database_url
            .strip_prefix("sqlite://")
            .unwrap_or(database_url);
        if let Some(parent) = Path::new(url).parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent).await?;
        }
        let options = SqliteConnectOptions::from_str(database_url)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;
        for statement in include_str!("../../migrations/0001_initial.sql")
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .context("initialize SQLite schema")?;
        }
        Ok(Self { pool })
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }

    pub async fn record_launch(
        &self,
        address: &str,
        launchpad: Option<&str>,
        ts_ms: i64,
    ) -> Result<()> {
        sqlx::query("INSERT INTO token_events(address, launchpad, launch_ts_ms, status) VALUES(?,?,?,'launched') ON CONFLICT(address) DO UPDATE SET launchpad=COALESCE(excluded.launchpad, token_events.launchpad), launch_ts_ms=CASE WHEN token_events.launch_ts_ms=0 THEN excluded.launch_ts_ms ELSE MIN(token_events.launch_ts_ms, excluded.launch_ts_ms) END")
            .bind(address).bind(launchpad).bind(ts_ms).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn mark_graduated(
        &self,
        address: &str,
        launchpad: Option<&str>,
        ts_ms: i64,
    ) -> Result<i64> {
        // 0 is a sentinel: if we never observed the launch event, age is unknown and
        // the strategy rejects the token rather than pretending it launched at graduation.
        sqlx::query("INSERT INTO token_events(address, launchpad, launch_ts_ms, graduation_ts_ms, status) VALUES(?,?,0,?,'graduated') ON CONFLICT(address) DO UPDATE SET launchpad=COALESCE(excluded.launchpad, token_events.launchpad), graduation_ts_ms=excluded.graduation_ts_ms, status='graduated'")
            .bind(address).bind(launchpad).bind(ts_ms).execute(&self.pool).await?;
        let ts: i64 = sqlx::query_scalar("SELECT launch_ts_ms FROM token_events WHERE address=?")
            .bind(address)
            .fetch_one(&self.pool)
            .await?;
        Ok(ts)
    }

    pub async fn get_launch_ts(&self, address: &str) -> Result<Option<i64>> {
        Ok(
            sqlx::query_scalar("SELECT launch_ts_ms FROM token_events WHERE address=?")
                .bind(address)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn mark_screened(&self, address: &str, accepted: bool, reason: &str) -> Result<()> {
        sqlx::query(
            "UPDATE token_events SET screened=1, accepted=?, reject_reason=? WHERE address=?",
        )
        .bind(accepted as i64)
        .bind(reason)
        .bind(address)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Claim a candidate exactly once before any deferred screening work is spawned.
    /// 2 means in-progress; 1 means completed. Startup resets abandoned claims.
    pub async fn claim_screening(&self, address: &str) -> Result<bool> {
        let result =
            sqlx::query("UPDATE token_events SET screened=2 WHERE address=? AND screened=0")
                .bind(address)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn reset_abandoned_screenings(&self) -> Result<()> {
        sqlx::query("UPDATE token_events SET screened=0 WHERE screened=2")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn pending_graduations(&self) -> Result<Vec<(String, Option<String>, i64)>> {
        let rows = sqlx::query("SELECT address, launchpad, graduation_ts_ms FROM token_events WHERE graduation_ts_ms IS NOT NULL AND screened=0")
            .fetch_all(&self.pool).await?;
        let mut pending = Vec::with_capacity(rows.len());
        for row in rows {
            pending.push((
                row.try_get("address")?,
                row.try_get("launchpad")?,
                row.try_get("graduation_ts_ms")?,
            ));
        }
        Ok(pending)
    }

    pub(crate) async fn open_position(
        &self,
        entry: PaperEntry<'_>,
        limits: PaperLimits,
    ) -> Result<OpenPaperOutcome> {
        // Acquire the writer lock before taking the snapshot used by every limit.
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let claimed: Option<i64> =
            sqlx::query_scalar("SELECT screened FROM token_events WHERE address=?")
                .bind(entry.address)
                .fetch_optional(&mut *tx)
                .await?;
        if claimed != Some(2) {
            tx.rollback().await?;
            return Ok(OpenPaperOutcome::NotClaimed);
        }

        let now = chrono::Utc::now();
        let now_ms = now.timestamp_millis();
        let day = UtcDay::at(now);
        let rejection = if count_open_positions(&mut tx).await? >= i64::from(limits.max_open) {
            Some("max open positions reached")
        } else if count_entries(&mut tx, &day).await? >= i64::from(limits.max_entries) {
            Some("daily trade limit reached")
        } else if realized_pnl(&mut tx, &day).await? <= -limits.max_daily_loss {
            Some("daily loss limit reached")
        } else if sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM positions WHERE token_address=? AND status='OPEN')",
        )
        .bind(entry.address)
        .fetch_one(&mut *tx)
        .await?
        {
            Some("position already open")
        } else {
            None
        };

        let (outcome, accepted, reason) = if let Some(reason) = rejection {
            (OpenPaperOutcome::Rejected(reason), false, reason)
        } else {
            let id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO positions(id,token_address,symbol,entry_price_usd,quantity,\
                cost_basis_usd,highest_price_usd,opened_at_ms,status) VALUES(?,?,?,?,?,?,?,?,'OPEN')",
            )
            .bind(&id)
            .bind(entry.address)
            .bind(entry.symbol)
            .bind(entry.price.to_string())
            .bind(entry.quantity.to_string())
            .bind(entry.cost_basis.to_string())
            .bind(entry.price.to_string())
            .bind(now_ms)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO trades(id,position_id,token_address,symbol,side,price_usd,notional_usd,\
                fee_usd,gas_usd,ts_ms,mode) VALUES(?,?,?,?,'BUY',?,?,?,?,?,'PAPER')",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(&id)
            .bind(entry.address)
            .bind(entry.symbol)
            .bind(entry.price.to_string())
            .bind(entry.notional.to_string())
            .bind(entry.fee.to_string())
            .bind(entry.gas.to_string())
            .bind(now_ms)
            .execute(&mut *tx)
            .await?;
            (
                OpenPaperOutcome::Opened(id),
                true,
                "accepted and paper position opened",
            )
        };
        let finalized = sqlx::query(
            "UPDATE token_events SET screened=1, accepted=?, reject_reason=? WHERE address=? AND screened=2",
        )
        .bind(accepted)
        .bind(reason)
        .bind(entry.address)
        .execute(&mut *tx)
        .await?;
        ensure!(
            finalized.rows_affected() == 1,
            "could not finalize PAPER screening claim"
        );
        tx.commit().await?;
        Ok(outcome)
    }

    pub async fn open_positions(&self) -> Result<Vec<PositionRow>> {
        let rows = sqlx::query("SELECT id,token_address,symbol,entry_price_usd,quantity,cost_basis_usd,highest_price_usd,opened_at_ms FROM positions WHERE status='OPEN'").fetch_all(&self.pool).await?;
        let mut out = Vec::new();
        for r in rows {
            out.push(PositionRow {
                id: r.try_get("id")?,
                token_address: r.try_get("token_address")?,
                symbol: r.try_get("symbol")?,
                entry_price_usd: Decimal::from_str(&r.try_get::<String, _>("entry_price_usd")?)?,
                quantity: Decimal::from_str(&r.try_get::<String, _>("quantity")?)?,
                cost_basis_usd: Decimal::from_str(&r.try_get::<String, _>("cost_basis_usd")?)?,
                highest_price_usd: Decimal::from_str(
                    &r.try_get::<String, _>("highest_price_usd")?,
                )?,
                opened_at_ms: r.try_get("opened_at_ms")?,
            });
        }
        Ok(out)
    }

    pub async fn update_highest_price(&self, id: &str, price: Decimal) -> Result<()> {
        sqlx::query("UPDATE positions SET highest_price_usd=? WHERE id=? AND status='OPEN'")
            .bind(price.to_string())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub(crate) async fn close_position(
        &self,
        p: &PositionRow,
        exit: PaperExit<'_>,
    ) -> Result<bool> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let ts_ms = chrono::Utc::now().timestamp_millis();
        let changed = sqlx::query(
            "UPDATE positions SET status='CLOSED', exit_price_usd=?, closed_at_ms=?, \
            realized_pnl_usd=?, exit_reason=? WHERE id=? AND status='OPEN'",
        )
        .bind(exit.price.to_string())
        .bind(ts_ms)
        .bind(exit.pnl.to_string())
        .bind(exit.reason)
        .bind(&p.id)
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() == 0 {
            tx.rollback().await?;
            return Ok(false);
        }
        sqlx::query(
            "INSERT INTO trades(id,position_id,token_address,symbol,side,price_usd,notional_usd,\
            fee_usd,gas_usd,pnl_usd,ts_ms,mode) VALUES(?,?,?,?,'SELL',?,?,?,?,?,?,'PAPER')",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&p.id)
        .bind(&p.token_address)
        .bind(&p.symbol)
        .bind(exit.price.to_string())
        .bind(exit.proceeds.to_string())
        .bind(exit.fee.to_string())
        .bind(exit.gas.to_string())
        .bind(exit.pnl.to_string())
        .bind(ts_ms)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    pub async fn open_position_addresses(&self) -> Result<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT token_address FROM positions WHERE status='OPEN'")
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn summary(&self) -> Result<(u32, u32, Decimal)> {
        let mut tx = self.pool.begin().await?;
        let day = UtcDay::at(chrono::Utc::now());
        let open = count_open_positions(&mut tx).await?;
        let trades = count_entries(&mut tx, &day).await?;
        let pnl = realized_pnl(&mut tx, &day).await?;
        tx.commit().await?;
        Ok((open.try_into()?, trades.try_into()?, pnl))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::AppConfig,
        trading::paper::{close_paper_position, open_paper_position},
    };

    fn d(value: &str) -> Decimal {
        value.parse().unwrap()
    }

    async fn claimed(store: &Store, address: &str) -> Result<()> {
        store.mark_graduated(address, None, 1).await?;
        assert!(store.claim_screening(address).await?);
        Ok(())
    }

    async fn candidate_state(store: &Store, address: &str) -> Result<(i64, bool, Option<String>)> {
        Ok(sqlx::query_as(
            "SELECT screened, accepted, reject_reason FROM token_events WHERE address=?",
        )
        .bind(address)
        .fetch_one(&store.pool)
        .await?)
    }

    #[tokio::test]
    async fn persists_accounting_and_closes_only_once() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let url = format!("sqlite://{}", dir.path().join("paper.db").display());
        let store = Store::connect(&url).await?;
        let cfg = AppConfig::test_config();
        claimed(&store, "token").await?;
        let (left, right) = tokio::try_join!(
            open_paper_position(&store, &cfg, "token", "TKN", d("2")),
            open_paper_position(&store, &cfg, "token", "TKN", d("2")),
        )?;
        let id = match (left, right) {
            (OpenPaperOutcome::Opened(id), OpenPaperOutcome::NotClaimed)
            | (OpenPaperOutcome::NotClaimed, OpenPaperOutcome::Opened(id)) => id,
            outcomes => panic!("unexpected outcomes: {outcomes:?}"),
        };
        assert_eq!(
            candidate_state(&store, "token").await?,
            (1, true, Some("accepted and paper position opened".into()))
        );
        let p = store.open_positions().await?.pop().unwrap();
        assert_eq!(p.id, id);
        assert_eq!(p.quantity, d("5"));
        assert_eq!(p.cost_basis_usd, d("10.20"));
        let (left, right) = tokio::try_join!(
            close_paper_position(&store, &cfg, &p, d("4"), "TAKE_PROFIT"),
            close_paper_position(&store, &cfg, &p, d("4"), "TAKE_PROFIT"),
        )?;
        assert_ne!(left, right);
        store.close().await;

        let reopened = Store::connect(&url).await?;
        assert_eq!(reopened.summary().await?, (0, 1, d("9.50")));
        let (status, reason, pnl): (String, String, String) =
            sqlx::query_as("SELECT status,exit_reason,realized_pnl_usd FROM positions WHERE id=?")
                .bind(&id)
                .fetch_one(&reopened.pool)
                .await?;
        assert_eq!(
            (status.as_str(), reason.as_str(), d(&pnl)),
            ("CLOSED", "TAKE_PROFIT", d("9.50"))
        );
        let trades = sqlx::query("SELECT side, notional_usd, fee_usd, gas_usd, pnl_usd FROM trades WHERE position_id=? ORDER BY side")
            .bind(&id).fetch_all(&reopened.pool).await?;
        assert_eq!(trades.len(), 2);
        for (row, (side, notional, fee, pnl)) in trades.iter().zip([
            ("BUY", d("10"), d("0.10"), None),
            ("SELL", d("20"), d("0.20"), Some(d("9.50"))),
        ]) {
            assert_eq!(row.try_get::<String, _>("side")?, side);
            assert_eq!(d(&row.try_get::<String, _>("notional_usd")?), notional);
            assert_eq!(d(&row.try_get::<String, _>("fee_usd")?), fee);
            assert_eq!(d(&row.try_get::<String, _>("gas_usd")?), d("0.10"));
            assert_eq!(
                row.try_get::<Option<String>, _>("pnl_usd")?
                    .as_deref()
                    .map(d),
                pnl
            );
        }
        reopened.close().await;
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_entries_obey_open_and_utc_daily_limits() -> Result<()> {
        for (max_open, max_entries, reason) in [
            (1, 5, "max open positions reached"),
            (2, 1, "daily trade limit reached"),
        ] {
            let dir = tempfile::tempdir()?;
            let url = format!("sqlite://{}", dir.path().join("limits.db").display());
            let store = Store::connect(&url).await?;
            let other = Store::connect(&url).await?;
            let mut cfg = AppConfig::test_config();
            cfg.max_open_positions = max_open;
            cfg.max_trades_per_day = max_entries;
            claimed(&store, "first").await?;
            claimed(&store, "second").await?;
            let (a, b) = tokio::try_join!(
                open_paper_position(&store, &cfg, "first", "ONE", d("2")),
                open_paper_position(&other, &cfg, "second", "TWO", d("2")),
            )?;
            let rejected = match (a, b) {
                (OpenPaperOutcome::Opened(_), OpenPaperOutcome::Rejected(why)) => {
                    assert_eq!(why, reason);
                    "second"
                }
                (OpenPaperOutcome::Rejected(why), OpenPaperOutcome::Opened(_)) => {
                    assert_eq!(why, reason);
                    "first"
                }
                outcomes => panic!("unexpected outcomes: {outcomes:?}"),
            };
            assert_eq!(
                candidate_state(&store, rejected).await?,
                (1, false, Some(reason.into()))
            );
            assert_eq!(store.summary().await?, (1, 1, Decimal::ZERO));
            store.close().await;
            other.close().await;
        }
        Ok(())
    }

    #[tokio::test]
    async fn open_handles_duplicate_tokens_and_realized_loss_cutoff() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let url = format!("sqlite://{}", dir.path().join("loss.db").display());
        let store = Store::connect(&url).await?;
        let mut cfg = AppConfig::test_config();
        cfg.max_daily_loss_usd = d("5.35");
        claimed(&store, "token").await?;
        assert!(matches!(
            open_paper_position(&store, &cfg, "token", "TKN", d("2")).await?,
            OpenPaperOutcome::Opened(_)
        ));
        // A legacy split write could leave an open position with an unfinished claim.
        sqlx::query("UPDATE token_events SET screened=2 WHERE address='token'")
            .execute(&store.pool)
            .await?;
        assert_eq!(
            open_paper_position(&store, &cfg, "token", "TKN", d("2")).await?,
            OpenPaperOutcome::Rejected("position already open")
        );

        let p = store.open_positions().await?.pop().unwrap();
        assert!(close_paper_position(&store, &cfg, &p, d("1"), "STOP_LOSS").await?);
        assert_eq!(store.summary().await?, (0, 1, d("-5.35")));
        claimed(&store, "next").await?;
        assert_eq!(
            open_paper_position(&store, &cfg, "next", "TKN", d("2")).await?,
            OpenPaperOutcome::Rejected("daily loss limit reached")
        );
        assert_eq!(store.summary().await?, (0, 1, d("-5.35")));
        store.close().await;
        Ok(())
    }

    #[tokio::test]
    async fn failed_writes_roll_back_accounting() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let url = format!("sqlite://{}", dir.path().join("rollback.db").display());
        let store = Store::connect(&url).await?;
        let cfg = AppConfig::test_config();
        claimed(&store, "token").await?;
        sqlx::query("CREATE TRIGGER reject_acceptance BEFORE UPDATE ON token_events WHEN NEW.accepted=1 BEGIN SELECT RAISE(ABORT,'test failure'); END")
            .execute(&store.pool).await?;
        assert!(
            open_paper_position(&store, &cfg, "token", "TKN", d("2"))
                .await
                .is_err()
        );
        assert_eq!(store.summary().await?, (0, 0, Decimal::ZERO));
        assert_eq!(candidate_state(&store, "token").await?, (2, false, None));
        sqlx::query("DROP TRIGGER reject_acceptance")
            .execute(&store.pool)
            .await?;
        assert!(matches!(
            open_paper_position(&store, &cfg, "token", "TKN", d("2")).await?,
            OpenPaperOutcome::Opened(_)
        ));

        let p = store.open_positions().await?.pop().unwrap();
        sqlx::query("CREATE TRIGGER reject_sell BEFORE INSERT ON trades WHEN NEW.side='SELL' BEGIN SELECT RAISE(ABORT,'test failure'); END")
            .execute(&store.pool).await?;
        assert!(
            close_paper_position(&store, &cfg, &p, d("4"), "TAKE_PROFIT")
                .await
                .is_err()
        );
        assert_eq!(store.summary().await?, (1, 1, Decimal::ZERO));
        store.close().await;
        Ok(())
    }
}
