//! Shared `sqlx` row structs and small cross-route view models.

use std::collections::HashMap;

use serde::Serialize;
use sqlx::FromRow;

use crate::compute;

/// A full row of the `symbols` table.
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct SymbolRow {
    pub ticker: String,
    pub name: String,
    pub kind: String,
    pub exchange: Option<String>,
    pub currency: String,
    pub cik: Option<String>,
    /// SEC fund series id; set for an ETF, NULL otherwise (see migration 0005).
    pub series_id: Option<String>,
    pub sector: Option<String>,
    pub industry: Option<String>,
    pub is_seeded: i64,
    pub is_watched: i64,
    pub history_synced_at: Option<i64>,
    pub history_first_date: Option<String>,
    pub history_last_date: Option<String>,
    pub fundamentals_synced_at: Option<i64>,
    pub filings_synced_at: Option<i64>,
    /// When this ETF's fund profile was last refreshed from SEC.
    pub fund_synced_at: Option<i64>,
    /// When this stock's leadership roster was last refreshed from SEC.
    pub leadership_synced_at: Option<i64>,
    /// When this stock or ETF's dividend / distribution history was last
    /// refreshed from Yahoo (Phase 26 + Phase 28). NULL for indexes / futures
    /// and any not-yet-swept symbol.
    pub dividends_synced_at: Option<i64>,
    /// When this ETF's Yahoo `quoteSummary` snapshot was last refreshed
    /// (Phase 28). NULL on every non-ETF row and on ETFs not yet swept.
    pub fund_metadata_synced_at: Option<i64>,
    /// Curated benchmark index ticker for an ETF (e.g. `^SPX`), populated
    /// from `universe/starter.csv` (Phase 28). The symbol page's chart
    /// shows the relative-performance overlay only when this is set.
    pub benchmark: Option<String>,
    /// Next-expected earnings date from Yahoo's `quoteSummary.calendarEvents`
    /// (Phase 25), epoch-ms. NULL when Yahoo has no upcoming date for this
    /// stock or the symbol has not been swept yet; the symbol page then
    /// falls back to a cadence estimate from past 8-K item-2.02 filings.
    pub next_earnings_at: Option<i64>,
    /// When this stock's Yahoo earnings-calendar snapshot was last
    /// refreshed (Phase 25). NULL = never swept. Stocks only.
    pub earnings_synced_at: Option<i64>,
    /// When this stock's Yahoo `quoteSummary.assetProfile` snapshot was
    /// last refreshed (Phase 15). Backs the `sector` / `industry` columns.
    /// NULL = never swept. Stocks only — non-stock rows stay NULL forever.
    pub asset_profile_synced_at: Option<i64>,
    pub last_price: Option<f64>,
    pub prev_close: Option<f64>,
    pub last_quote_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A symbol's price row as selected for a card grid: ticker, name, kind, the
/// price to show, and the close it is changing against.
pub type SymbolCardRow = (String, String, String, Option<f64>, Option<f64>);

/// A compact symbol tile, rendered by the `ticker_card` macro. Shared by the
/// Markets dashboard and the Search page.
#[derive(Serialize)]
pub struct Card {
    pub ticker: String,
    pub name: String,
    pub kind: String,
    pub price: Option<f64>,
    pub change_abs: Option<f64>,
    pub change_pct: Option<f64>,
    /// The rolled-up strong / fair / weak verdict badge (Phase 20). Stocks
    /// only, and only once SEC fundamentals have synced; `None` otherwise.
    pub strength: Option<compute::Standing>,
}

/// Build a [`Card`] from a selected price row, computing the change off the
/// price and its prior close. The `strength` badge is left unset; a caller
/// with fundamentals on hand fills it in.
pub fn to_card((ticker, name, kind, last, prev): SymbolCardRow) -> Card {
    let (change_abs, change_pct) = match (last, prev) {
        (Some(l), Some(p)) => {
            let c = compute::change(l, p);
            (Some(c.abs), Some(c.pct))
        }
        _ => (None, None),
    };
    Card {
        ticker,
        name,
        kind,
        price: last,
        change_abs,
        change_pct,
        strength: None,
    }
}

/// One fundamentals fact as stored: a metric's value for one fiscal period.
#[derive(Debug, Clone, FromRow)]
pub struct FundFact {
    pub metric: String,
    pub period: String,
    pub fiscal_year: i64,
    pub fiscal_qtr: Option<i64>,
    pub value: f64,
    /// `YYYY-MM-DD` end-of-period date. Dates the fundamentals-anomaly feed
    /// (the FY in which a revenue / net-income move landed).
    pub period_end: String,
}

/// Assemble [`compute::RatioInputs`] for a company's most recent full fiscal
/// year from its stored facts plus a price. Annual rows only; the prior year's
/// figures (for the growth ratios) come from `latest_fy - 1`. `None` when the
/// company has no annual facts. Shared by the symbol page and the home quality
/// leaderboard so both grade a stock identically.
pub fn latest_annual_inputs(facts: &[FundFact], price: Option<f64>) -> Option<compute::RatioInputs> {
    latest_annual_inputs_filtered(facts, price, |_| true)
}

/// YoY change threshold (25%) on annual revenue or net-income above which a
/// fundamentals anomaly event is emitted.
const FUND_YOY_THRESHOLD: f64 = 0.25;

/// Walk a company's stored facts and emit one anomaly event per (metric,
/// fiscal year) whose YoY change exceeds ±25%. Only annual `revenue` and
/// `net_income` are surfaced — the two top-line figures whose moves are
/// readable without further context. The event's date is the fiscal year's
/// `period_end` (the year that ended), so the feed reads as "FY2024
/// revenue ‒32% YoY" on the day that fiscal year closed.
pub fn fundamentals_anomalies(facts: &[FundFact]) -> Vec<compute::AnomalyEvent> {
    let mut annual: HashMap<(&str, i64), (f64, &str)> = HashMap::new();
    for f in facts {
        if f.fiscal_qtr.is_none() && (f.metric == "revenue" || f.metric == "net_income") {
            annual.insert(
                (f.metric.as_str(), f.fiscal_year),
                (f.value, f.period_end.as_str()),
            );
        }
    }
    let mut out: Vec<compute::AnomalyEvent> = Vec::new();
    for ((metric, year), (val, period_end)) in &annual {
        let prev = match annual.get(&(metric, year - 1)) {
            Some((v, _)) => *v,
            None => continue,
        };
        if prev.abs() < 1e-9 {
            continue;
        }
        let change = (val - prev) / prev.abs();
        if change.abs() < FUND_YOY_THRESHOLD {
            continue;
        }
        let pct = change * 100.0;
        let label = match *metric {
            "revenue" => "revenue",
            "net_income" => "net income",
            _ => metric,
        };
        let (glyph, polarity, sign) = if pct >= 0.0 {
            ("fund-up", "good", "+")
        } else {
            ("fund-down", "bad", "\u{2212}")
        };
        out.push(compute::AnomalyEvent {
            date: period_end.to_string(),
            glyph,
            polarity,
            headline: format!("FY{year} {label} {sign}{:.0}% YoY", pct.abs()),
            url: None,
            severity: pct.abs(),
        });
    }
    out
}

fn latest_annual_inputs_filtered(
    facts: &[FundFact],
    price: Option<f64>,
    keep: impl Fn(&FundFact) -> bool,
) -> Option<compute::RatioInputs> {
    // (metric, fiscal_year) -> value, annual rows only.
    let mut annual: HashMap<(&str, i64), f64> = HashMap::new();
    let mut latest_fy: Option<i64> = None;
    for f in facts {
        if f.fiscal_qtr.is_none() && keep(f) {
            annual.insert((f.metric.as_str(), f.fiscal_year), f.value);
            // Only an income-statement metric may advance the "latest fiscal
            // year" the ratios key off. A stray balance-sheet or dividend figure
            // tagged a year ahead (common right around a filing) would otherwise
            // make `latest_fy` a year the core figures aren't in yet, blanking
            // every ratio instead of reading the most recent complete year.
            if matches!(f.metric.as_str(), "revenue" | "net_income" | "eps_diluted") {
                latest_fy = Some(latest_fy.map_or(f.fiscal_year, |y| y.max(f.fiscal_year)));
            }
        }
    }
    let fy = latest_fy?;
    let av = |m: &str, y: i64| annual.get(&(m, y)).copied();
    Some(compute::RatioInputs {
        price,
        eps_diluted: av("eps_diluted", fy),
        dividends_per_share: av("dividends_per_share", fy),
        revenue: av("revenue", fy),
        net_income: av("net_income", fy),
        assets: av("assets", fy),
        liabilities: av("liabilities", fy),
        equity: av("equity", fy),
        assets_current: av("assets_current", fy),
        liabilities_current: av("liabilities_current", fy),
        prev_revenue: av("revenue", fy - 1),
        prev_net_income: av("net_income", fy - 1),
    })
}
