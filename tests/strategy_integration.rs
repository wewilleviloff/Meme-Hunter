use meme_hunter::strategy::{CandidateMetrics, Decision, StrategyConfig, evaluate_candidate};
use rust_decimal::Decimal;
use std::str::FromStr;
fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}
#[test]
fn configured_liquidity_boundary_is_inclusive() {
    let c = StrategyConfig {
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
    };
    let m = CandidateMetrics {
        age_seconds: 60,
        graduated: true,
        liquidity_usd: Some(d("10000")),
        price_usd: Some(d("0.01")),
        price_confidence: Some("high".into()),
        thin_liquidity: Some(false),
        pools_count: Some(1),
        holders_count: Some(30),
        top10_pct: Some(d("55")),
        dev_pct: Some(d("5")),
        volume_5m_usd: Some(d("500")),
        security_score: Some(90),
        security_risk: Some("low".into()),
        is_honeypot: Some(false),
        selective_sell: Some(false),
        tax_source: Some("realized".into()),
        buy_tax_pct: Some(d("2")),
        sell_tax_pct: Some(d("2")),
        realized_sell_samples: Some(3),
        blacklist: Some(false),
        mintable: Some(false),
        pausable: Some(false),
        upgradeable: Some(false),
        can_change_tax: Some(false),
        can_change_limits: Some(false),
        is_proxy: Some(false),
        self_destruct: Some(false),
        hook_present: Some(false),
        hook_risk_level: None,
        hook_can_block_swaps: None,
        max_sell_tax_seen_pct: Some(d("2")),
    };
    assert_eq!(evaluate_candidate(&m, &c), Decision::Accept);
}
