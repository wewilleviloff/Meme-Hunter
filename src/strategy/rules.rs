use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct CandidateMetrics {
    pub age_seconds: i64,
    pub graduated: bool,
    pub liquidity_usd: Option<Decimal>,
    pub price_usd: Option<Decimal>,
    pub price_confidence: Option<String>,
    pub thin_liquidity: Option<bool>,
    pub pools_count: Option<u64>,
    pub holders_count: Option<u64>,
    pub top10_pct: Option<Decimal>,
    pub dev_pct: Option<Decimal>,
    pub volume_5m_usd: Option<Decimal>,
    pub security_score: Option<u8>,
    pub security_risk: Option<String>,
    pub is_honeypot: Option<bool>,
    pub selective_sell: Option<bool>,
    pub tax_source: Option<String>,
    pub buy_tax_pct: Option<Decimal>,
    pub sell_tax_pct: Option<Decimal>,
    pub realized_sell_samples: Option<u32>,
    pub blacklist: Option<bool>,
    pub mintable: Option<bool>,
    pub pausable: Option<bool>,
    pub upgradeable: Option<bool>,
    pub can_change_tax: Option<bool>,
    pub can_change_limits: Option<bool>,
    pub is_proxy: Option<bool>,
    pub self_destruct: Option<bool>,
    pub hook_present: Option<bool>,
    pub hook_risk_level: Option<String>,
    pub hook_can_block_swaps: Option<bool>,
    pub max_sell_tax_seen_pct: Option<Decimal>,
}

#[derive(Debug, Clone)]
pub struct StrategyConfig {
    pub min_age_seconds: i64,
    pub max_age_seconds: i64,
    pub min_liquidity_usd: Decimal,
    pub min_security_score: u8,
    pub min_holders: u64,
    pub max_top10_pct: Decimal,
    pub max_dev_pct: Decimal,
    pub min_volume_5m_usd: Decimal,
    pub max_buy_tax_pct: Decimal,
    pub max_sell_tax_pct: Decimal,
    pub min_realized_sell_samples: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    Accept,
    Reject(String),
}

pub fn evaluate_candidate(m: &CandidateMetrics, c: &StrategyConfig) -> Decision {
    macro_rules! reject_if {
        ($condition:expr, $reason:expr) => {
            if $condition {
                return Decision::Reject($reason.to_string());
            }
        };
    }
    reject_if!(
        m.age_seconds < c.min_age_seconds || m.age_seconds > c.max_age_seconds,
        "token age outside configured window"
    );
    reject_if!(!m.graduated, "token has not graduated to a DEX pool");
    reject_if!(m.pools_count.unwrap_or(0) == 0, "no indexed DEX pool");
    reject_if!(
        m.liquidity_usd.is_none_or(|v| v < c.min_liquidity_usd),
        "liquidity missing or below minimum"
    );
    reject_if!(
        m.price_usd.is_none_or(|v| v <= Decimal::ZERO),
        "price missing or invalid"
    );
    reject_if!(
        !matches!(m.price_confidence.as_deref(), Some("high" | "medium")),
        "price confidence low or unknown"
    );
    reject_if!(
        m.thin_liquidity != Some(false),
        "thin-liquidity flag true or unknown"
    );
    reject_if!(
        m.holders_count.is_none_or(|v| v < c.min_holders),
        "holder count missing or below minimum"
    );
    reject_if!(
        m.top10_pct
            .is_none_or(|v| v < Decimal::ZERO || v > c.max_top10_pct),
        "top-10 concentration missing or too high"
    );
    reject_if!(
        m.dev_pct
            .is_none_or(|v| v < Decimal::ZERO || v > c.max_dev_pct),
        "developer concentration missing or too high"
    );
    reject_if!(
        m.volume_5m_usd.is_none_or(|v| v < c.min_volume_5m_usd),
        "five-minute volume missing or too low"
    );
    reject_if!(
        m.security_score
            .is_none_or(|v| v < c.min_security_score || v > 100),
        "security score missing or below minimum"
    );
    reject_if!(
        m.security_risk.as_deref() != Some("low"),
        "Crius risk is not low or unknown"
    );
    reject_if!(
        m.is_honeypot != Some(false),
        "honeypot check positive or unknown"
    );
    reject_if!(
        m.selective_sell != Some(false),
        "selective sell flag positive or unknown"
    );
    reject_if!(
        m.tax_source.as_deref() != Some("realized"),
        "realized tax data unavailable"
    );
    reject_if!(
        m.realized_sell_samples
            .is_none_or(|v| v < c.min_realized_sell_samples),
        "insufficient realized sell samples"
    );
    reject_if!(
        m.buy_tax_pct
            .is_none_or(|v| v < Decimal::ZERO || v > c.max_buy_tax_pct),
        "buy tax missing or too high"
    );
    reject_if!(
        m.sell_tax_pct
            .is_none_or(|v| v < Decimal::ZERO || v > c.max_sell_tax_pct),
        "sell tax missing or too high"
    );
    reject_if!(
        m.blacklist != Some(false),
        "blacklist capability present or unknown"
    );
    reject_if!(
        m.mintable != Some(false),
        "mint capability present or unknown"
    );
    reject_if!(
        m.pausable != Some(false),
        "pause capability present or unknown"
    );
    reject_if!(
        m.upgradeable != Some(false),
        "upgradeability present or unknown"
    );
    reject_if!(
        m.can_change_tax != Some(false),
        "mutable tax capability present or unknown"
    );
    reject_if!(
        m.can_change_limits != Some(false),
        "mutable limits capability present or unknown"
    );
    reject_if!(
        m.is_proxy != Some(false),
        "proxy contract present or unknown"
    );
    reject_if!(
        m.self_destruct != Some(false),
        "self-destruct capability present or unknown"
    );
    reject_if!(
        m.max_sell_tax_seen_pct
            .is_none_or(|v| v < Decimal::ZERO || v > c.max_sell_tax_pct),
        "historical maximum sell tax missing or above limit"
    );
    reject_if!(m.hook_present.is_none(), "v4 hook presence unknown");
    if m.hook_present == Some(true) {
        reject_if!(
            m.hook_risk_level.as_deref() != Some("low"),
            "v4 hook risk is not low or unknown"
        );
        reject_if!(
            m.hook_can_block_swaps != Some(false),
            "v4 hook can block swaps or capability is unknown"
        );
    }
    Decision::Accept
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitSignal {
    TakeProfit,
    StopLoss,
}

pub fn exit_thresholds(
    entry_price: Decimal,
    take_profit_multiple: Decimal,
    stop_loss_pct: Decimal,
) -> (Decimal, Decimal) {
    (
        entry_price * take_profit_multiple,
        entry_price * (Decimal::ONE - stop_loss_pct / Decimal::from(100)),
    )
}

pub fn evaluate_exit(
    entry_price: Decimal,
    current_price: Decimal,
    take_profit_multiple: Decimal,
    stop_loss_pct: Decimal,
) -> Option<ExitSignal> {
    if entry_price <= Decimal::ZERO || current_price <= Decimal::ZERO {
        return None;
    }
    let (take_profit_price, stop_loss_price) =
        exit_thresholds(entry_price, take_profit_multiple, stop_loss_pct);
    if current_price >= take_profit_price {
        return Some(ExitSignal::TakeProfit);
    }
    if current_price <= stop_loss_price {
        return Some(ExitSignal::StopLoss);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    fn d(v: &str) -> Decimal {
        Decimal::from_str(v).unwrap()
    }
    fn passing() -> CandidateMetrics {
        CandidateMetrics {
            age_seconds: 120,
            graduated: true,
            liquidity_usd: Some(d("12000")),
            price_usd: Some(d("0.01")),
            price_confidence: Some("high".into()),
            thin_liquidity: Some(false),
            pools_count: Some(1),
            holders_count: Some(60),
            top10_pct: Some(d("35")),
            dev_pct: Some(d("2")),
            volume_5m_usd: Some(d("900")),
            security_score: Some(95),
            security_risk: Some("low".into()),
            is_honeypot: Some(false),
            selective_sell: Some(false),
            tax_source: Some("realized".into()),
            buy_tax_pct: Some(d("0")),
            sell_tax_pct: Some(d("0")),
            realized_sell_samples: Some(5),
            blacklist: Some(false),
            mintable: Some(false),
            pausable: Some(false),
            upgradeable: Some(false),
            can_change_tax: Some(false),
            can_change_limits: Some(false),
            is_proxy: Some(false),
            self_destruct: Some(false),
            hook_present: Some(true),
            hook_risk_level: Some("low".into()),
            hook_can_block_swaps: Some(false),
            max_sell_tax_seen_pct: Some(d("0")),
        }
    }
    fn cfg() -> StrategyConfig {
        StrategyConfig {
            min_age_seconds: 60,
            max_age_seconds: 1800,
            min_liquidity_usd: d("10000"),
            min_security_score: 90,
            min_holders: 30,
            max_top10_pct: d("55"),
            max_dev_pct: d("5"),
            min_volume_5m_usd: d("500"),
            max_buy_tax_pct: d("2"),
            max_sell_tax_pct: d("2"),
            min_realized_sell_samples: 3,
        }
    }
    #[test]
    fn passing_candidate_is_accepted() {
        assert_eq!(evaluate_candidate(&passing(), &cfg()), Decision::Accept);
    }
    #[test]
    fn unknown_confidence_or_hook_and_invalid_tax_fail_closed() {
        let mut m = passing();
        m.price_confidence = Some("unknown".into());
        assert!(matches!(
            evaluate_candidate(&m, &cfg()),
            Decision::Reject(_)
        ));
        m.price_confidence = Some("medium".into());
        m.hook_present = None;
        assert!(matches!(
            evaluate_candidate(&m, &cfg()),
            Decision::Reject(_)
        ));
        m.hook_present = Some(false);
        m.sell_tax_pct = Some(d("-1"));
        assert!(matches!(
            evaluate_candidate(&m, &cfg()),
            Decision::Reject(_)
        ));
    }
    #[test]
    fn missing_security_data_fails_closed() {
        let mut m = passing();
        m.is_honeypot = None;
        assert!(matches!(
            evaluate_candidate(&m, &cfg()),
            Decision::Reject(_)
        ));
    }
    #[test]
    fn low_liquidity_rejected() {
        let mut m = passing();
        m.liquidity_usd = Some(d("9999"));
        assert!(matches!(
            evaluate_candidate(&m, &cfg()),
            Decision::Reject(_)
        ));
    }
    #[test]
    fn tp_and_sl_levels_are_exact() {
        assert_eq!(
            evaluate_exit(d("1"), d("2"), d("2"), d("35")),
            Some(ExitSignal::TakeProfit)
        );
        assert_eq!(
            evaluate_exit(d("1"), d("0.65"), d("2"), d("35")),
            Some(ExitSignal::StopLoss)
        );
    }
}
