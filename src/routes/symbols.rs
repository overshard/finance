use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::Sse,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::Datelike as _;
use serde::{Deserialize, Serialize};

use crate::compute;
use crate::db::now_ms;
use crate::guard::{EndpointGuard, Permit};
use crate::market;
use crate::models::{self, SymbolRow};
use crate::providers::http;
use crate::providers::yahoo::{SymbolLookup, YahooProvider};
use crate::render::{not_found, render};
use crate::{scheduler, AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/s/{ticker}", get(symbol_page))
        .route("/api/symbols", post(add_symbol))
        .route("/api/symbols/{ticker}/history", get(history_api))
        .route("/api/symbols/{ticker}/growth", get(growth_api))
        .route("/api/symbols/{ticker}/refresh", get(refresh_stream))
}

/// Stats for the symbol page header and key-stats visualizations. Until
/// intraday data exists (Phase 5), "day" figures come from the most recent
/// daily bar. The `*_pos` fields are 0..100 marker positions for the Paper
/// Ledger range bars; they are derived here so the template stays declarative.
#[derive(Debug, Serialize)]
struct Stats {
    date: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: i64,
    prev_close: Option<f64>,
    change_abs: Option<f64>,
    change_pct: Option<f64>,
    /// Open relative to the prior close (the overnight gap).
    open_change_pct: Option<f64>,
    high_52w: f64,
    low_52w: f64,
    /// Mean daily volume over the recent ~3-month window.
    avg_volume: i64,
    /// Marker positions on the day's low..high range bar.
    day_open_pos: f64,
    day_close_pos: f64,
    /// Marker positions on the 52-week low..high range bar.
    yr_close_pos: f64,
    yr_prev_pos: Option<f64>,
    /// Today's volume on a 0..2x-average scale (the average sits at 50).
    vol_fill_pct: f64,
    /// Today's volume as a multiple of the average, e.g. 1.3 for 1.3x.
    vol_ratio: Option<f64>,
}

/// The chart's indicators distilled into a colour-coded read: an overall
/// trend verdict, an RSI momentum gauge, and one signal tile per moving average
/// (where the price sits vs it) plus the 50/200 cross. Built from the daily
/// closes; `None` until a symbol has enough history.
#[derive(Serialize)]
struct IndicatorRead {
    /// Overall trend verdict ("Bullish" / "Mixed" / "Bearish") + its tone and a
    /// one-line tally ("3 of 4 trend signals bullish").
    verdict: String,
    verdict_tone: String,
    verdict_note: String,
    /// RSI(14): the value, its 0–100 position (for the gauge), bucket label,
    /// tone, and a plain-language verdict.
    rsi: f64,
    rsi_pos: f64,
    rsi_label: String,
    rsi_tone: String,
    rsi_note: String,
    /// One colour-coded tile per moving-average signal.
    signals: Vec<IndicatorSignal>,
}

/// One signal tile: the indicator label, its current value, a short status word
/// (Above / Below / Golden cross / …), the tone colour, and a plain meaning.
#[derive(Serialize)]
struct IndicatorSignal {
    label: String,
    value: String,
    status: String,
    tone: String,
    note: String,
}

/// Build the colour-coded indicator read from the daily closes (oldest first),
/// the current price, and whether the symbol is dollar-priced. Needs enough
/// history for RSI(14)/EMA(21); the 50/200 averages join once they exist.
fn build_indicator_read(
    highs: &[f64],
    lows: &[f64],
    closes: &[f64],
    price: f64,
    dollar: bool,
) -> Option<IndicatorRead> {
    if closes.len() < 30 || price <= 0.0 {
        return None;
    }
    let rsi = compute::rsi(closes, 14).last().copied().flatten()?;
    let fmt = |v: f64| {
        let n = format!("{v:.2}");
        if dollar {
            format!("${n}")
        } else {
            n
        }
    };

    // RSI verdict: 70+/30- are the textbook overbought/oversold extremes; the
    // 45–55 middle is balanced, with a "leaning" read on either side.
    let (rsi_label, rsi_tone, rsi_note) = if rsi >= 70.0 {
        ("Overbought", "down",
         format!("RSI {rsi:.0} — overbought: momentum is stretched and may be due for a pullback."))
    } else if rsi <= 30.0 {
        ("Oversold", "up",
         format!("RSI {rsi:.0} — oversold: selling looks stretched and may be due for a bounce."))
    } else if rsi >= 55.0 {
        ("Leaning bullish", "up",
         format!("RSI {rsi:.0} — firm momentum, not yet overbought."))
    } else if rsi <= 45.0 {
        ("Leaning bearish", "down",
         format!("RSI {rsi:.0} — soft momentum, not yet oversold."))
    } else {
        ("Neutral", "steady",
         format!("RSI {rsi:.0} — momentum is balanced between buyers and sellers."))
    };

    let ema21 = compute::ema(closes, 21).last().copied().flatten();
    let sma50 = compute::sma(closes, 50).last().copied().flatten();
    let sma200 = compute::sma(closes, 200).last().copied().flatten();

    // One tile per average that exists, each relating the current price to it.
    let mut signals = Vec::new();
    let mut bull = 0u32;
    let mut total = 0u32;
    let mut ma_tile = |label: &str, val: f64, span: &str| {
        let above = price >= val;
        if above {
            bull += 1;
        }
        total += 1;
        signals.push(IndicatorSignal {
            label: label.to_string(),
            value: fmt(val),
            status: if above { "Above" } else { "Below" }.to_string(),
            tone: if above { "up" } else { "down" }.to_string(),
            note: format!("{span} trend is {}", if above { "up" } else { "down" }),
        });
    };
    if let Some(v) = ema21 {
        ma_tile("EMA 21", v, "Near-term");
    }
    if let Some(v) = sma50 {
        ma_tile("SMA 50", v, "Medium-term");
    }
    if let Some(v) = sma200 {
        ma_tile("SMA 200", v, "Long-term");
    }

    // The 50-vs-200 posture (the golden/death-cross regime).
    if let (Some(f), Some(s)) = (sma50, sma200) {
        let golden = f >= s;
        if golden {
            bull += 1;
        }
        total += 1;
        signals.push(IndicatorSignal {
            label: "50 / 200-day".to_string(),
            value: String::new(),
            status: if golden { "Golden cross" } else { "Death cross" }.to_string(),
            tone: if golden { "up" } else { "down" }.to_string(),
            note: if golden {
                "50-day above the 200-day — bullish".to_string()
            } else {
                "50-day below the 200-day — bearish".to_string()
            },
        });
    }

    // Supertrend posture: which side of the ATR band price closed on. Folds
    // into the same bullish/bearish tally as the moving-average signals.
    let st = compute::supertrend(highs, lows, closes, compute::SUPERTREND_PERIOD, compute::SUPERTREND_MULT)
        .last()
        .copied()
        .flatten();
    if let Some(p) = st {
        if p.up {
            bull += 1;
        }
        total += 1;
        signals.push(IndicatorSignal {
            label: "Supertrend".to_string(),
            value: fmt(p.value),
            status: if p.up { "Uptrend" } else { "Downtrend" }.to_string(),
            tone: if p.up { "up" } else { "down" }.to_string(),
            note: if p.up {
                "Price is holding above the Supertrend band — bullish".to_string()
            } else {
                "Price is below the Supertrend band — bearish".to_string()
            },
        });
    }

    // Overall verdict from the trend tally.
    let (verdict, verdict_tone) = if total == 0 {
        ("No signal", "steady")
    } else if bull == total {
        ("Bullish", "up")
    } else if bull == 0 {
        ("Bearish", "down")
    } else {
        ("Mixed", "warn")
    };
    let verdict_note = format!("{bull} of {total} trend signals bullish");

    Some(IndicatorRead {
        verdict: verdict.to_string(),
        verdict_tone: verdict_tone.to_string(),
        verdict_note,
        rsi,
        rsi_pos: rsi.clamp(0.0, 100.0),
        rsi_label: rsi_label.to_string(),
        rsi_tone: rsi_tone.to_string(),
        rsi_note,
        signals,
    })
}

/// The live quote shown in the symbol header, when one exists. The header
/// carries `data-field` hooks, so the stream client patches these in place as
/// fresh quotes arrive; this is just the server-rendered starting point.
#[derive(Debug, Serialize)]
struct HeaderQuote {
    price: f64,
    change_abs: Option<f64>,
    change_pct: Option<f64>,
    /// A short human label for the quote's freshness, e.g. "Live", "At close".
    state_label: String,
}

#[derive(sqlx::FromRow)]
struct QuoteRow {
    price: f64,
    prev_close: Option<f64>,
}

/// A short freshness label for the symbol header. Yahoo's chart endpoint does
/// not carry a market-state field, so this comes from our own session clock
/// (`market.rs`) rather than the quote.
fn quote_state_label() -> &'static str {
    match market::session_at(chrono::Utc::now()) {
        market::Session::Pre => "Pre-market",
        market::Session::Regular => "Live",
        market::Session::Post => "After hours",
        market::Session::Closed => "At close",
    }
}

// ── fundamentals + filings (Phase 7) ──────────────────────────────────────

/// Placeholder glyph for a value the company did not report — an em dash, an
/// unambiguous "no data" mark (a middle dot read as a stray decimal point).
const DASH: &str = "\u{2014}";

/// Whether a period-over-period rise in a metric is good news, for the
/// financials-table growth cue (PLAN.md Phase 24).
#[derive(Clone, Copy)]
enum Trend {
    /// A rise reads as good: revenue, earnings, dividends.
    RiseGood,
    /// A rise reads as bad: liabilities.
    RiseBad,
    /// No good/bad reading — total assets and equity, where a rise can be
    /// debt-funded or a fall can be a shareholder-friendly buyback. The cue
    /// still shows the direction, just without a colour.
    Neutral,
}

impl Trend {
    /// How a rise vs the prior period reads: `good`, `bad`, or `` (no colour).
    fn rise(self) -> &'static str {
        match self {
            Trend::RiseGood => "good",
            Trend::RiseBad => "bad",
            Trend::Neutral => "",
        }
    }
    /// How a fall vs the prior period reads.
    fn fall(self) -> &'static str {
        match self {
            Trend::RiseGood => "bad",
            Trend::RiseBad => "good",
            Trend::Neutral => "",
        }
    }
}

/// One cell of a financials table: a formatted figure and its period-over-
/// period growth cue (PLAN.md Phase 24).
#[derive(Serialize)]
struct FundCell {
    /// The formatted figure, or [`DASH`] where nothing was reported.
    display: String,
    /// Direction vs the column to its left: `up`, `down`, or `` (the first
    /// column, a flat figure, or a missing value on either side).
    dir: &'static str,
    /// How that move reads for this metric: `good`, `bad`, or `` (no colour).
    sense: &'static str,
}

/// One row of a financials table: a metric label and one cell per period.
#[derive(Serialize)]
struct FundRow {
    label: String,
    cells: Vec<FundCell>,
}

/// A financials table (annual or quarterly) as period columns and metric rows.
#[derive(Serialize)]
struct FundTable {
    /// Column headers, oldest period first.
    periods: Vec<String>,
    rows: Vec<FundRow>,
}

/// Everything the symbol page's fundamentals + financials sections need.
#[derive(Serialize)]
struct FundamentalsView {
    /// Fiscal period the ratios are based on, e.g. `FY2024`.
    basis: Option<String>,
    ratios: Vec<compute::Ratio>,
    annual: FundTable,
    quarterly: FundTable,
    has_annual: bool,
    has_quarterly: bool,
}

/// One SEC filing shaped for the page.
#[derive(Serialize)]
struct FilingView {
    /// The raw form type, e.g. `10-K`, shown as a badge.
    form: String,
    /// A plain-English title derived from the form.
    title: String,
    filed_at: String,
    period_of_report: Option<String>,
    url: String,
}

#[derive(sqlx::FromRow)]
struct FilingRow {
    form: String,
    filed_at: String,
    period_of_report: Option<String>,
    url: String,
}

/// The metrics shown as rows of the financials table, in order. `is_money` is
/// `true` for a whole-dollar figure (shown compact, e.g. `$391.0B`) and
/// `false` for a per-share figure (shown as plain dollars, e.g. `$6.08`).
/// `in_quarterly` is `false` for the balance-sheet rows: only the fiscal
/// year-end balance is collected, so those rows appear in the annual table
/// only (see `providers::sec::classify`). `trend` sets the period-over-period
/// growth cue's good/bad reading (PLAN.md Phase 24).
struct TableMetric {
    metric: &'static str,
    label: &'static str,
    is_money: bool,
    in_quarterly: bool,
    trend: Trend,
}

const FUND_TABLE_METRICS: &[TableMetric] = &[
    TableMetric { metric: "revenue", label: "Revenue", is_money: true, in_quarterly: true, trend: Trend::RiseGood },
    TableMetric { metric: "net_income", label: "Net income", is_money: true, in_quarterly: true, trend: Trend::RiseGood },
    TableMetric { metric: "eps_diluted", label: "Diluted EPS", is_money: false, in_quarterly: true, trend: Trend::RiseGood },
    TableMetric { metric: "dividends_per_share", label: "Dividend / share", is_money: false, in_quarterly: true, trend: Trend::RiseGood },
    TableMetric { metric: "assets", label: "Total assets", is_money: true, in_quarterly: false, trend: Trend::Neutral },
    TableMetric { metric: "liabilities", label: "Total liabilities", is_money: true, in_quarterly: false, trend: Trend::RiseBad },
    TableMetric { metric: "equity", label: "Shareholder equity", is_money: true, in_quarterly: false, trend: Trend::Neutral },
];

/// Format a whole-dollar figure compactly: `391035000000.0` -> `$391.0B`.
fn fmt_usd_compact(v: f64) -> String {
    let sign = if v < 0.0 { "-" } else { "" };
    let a = v.abs();
    let (n, suffix) = if a >= 1e12 {
        (a / 1e12, "T")
    } else if a >= 1e9 {
        (a / 1e9, "B")
    } else if a >= 1e6 {
        (a / 1e6, "M")
    } else if a >= 1e3 {
        (a / 1e3, "K")
    } else {
        (a, "")
    };
    if suffix.is_empty() {
        format!("{sign}${n:.0}")
    } else {
        format!("{sign}${n:.1}{suffix}")
    }
}

/// Format a per-share figure: `6.08` -> `$6.08`.
fn fmt_per_share(v: f64) -> String {
    format!("${v:.2}")
}

/// A plain-English title for a filing, derived from its form type.
fn filing_title(form: &str) -> String {
    let base = if form.starts_with("10-K") {
        "Annual report"
    } else if form.starts_with("10-Q") {
        "Quarterly report"
    } else if form.starts_with("8-K") {
        "Current report"
    } else if form.starts_with("DEF 14A") {
        "Proxy statement"
    } else if form.starts_with("20-F") || form.starts_with("40-F") {
        "Annual report"
    } else if form.starts_with("6-K") {
        "Interim report"
    } else if form.starts_with("NPORT") {
        "Portfolio holdings report"
    } else if form.starts_with("N-CEN") {
        "Annual fund census"
    } else if form.starts_with("N-CSR") {
        "Shareholder report"
    } else if form.starts_with("485") {
        "Prospectus"
    } else {
        "Filing"
    };
    if form.ends_with("/A") {
        format!("{base} (amended)")
    } else {
        base.to_string()
    }
}

/// Build one financials table for the given periods (each `(fiscal_year,
/// period_label)`, oldest first), pulling formatted cells from `lookup`. The
/// quarterly table omits the balance-sheet rows, which are only collected per
/// fiscal year. Each cell also carries a period-over-period growth cue
/// (PLAN.md Phase 24), computed against the column to its left.
fn fund_table(
    periods: &[(i64, String)],
    lookup: &HashMap<(String, String), f64>,
    quarterly: bool,
) -> FundTable {
    let rows = FUND_TABLE_METRICS
        .iter()
        .filter(|m| !quarterly || m.in_quarterly)
        .map(|m| {
            // Walk periods oldest-first, carrying the prior period's value so
            // each cell can be marked up / down against the one before it.
            let mut prev: Option<f64> = None;
            let cells = periods
                .iter()
                .map(|(_, period)| {
                    let value = lookup
                        .get(&(m.metric.to_string(), period.clone()))
                        .copied();
                    let display = match value {
                        Some(v) if m.is_money => fmt_usd_compact(v),
                        Some(v) => fmt_per_share(v),
                        None => DASH.to_string(),
                    };
                    let (dir, sense) = match (prev, value) {
                        (Some(p), Some(v)) if v > p => ("up", m.trend.rise()),
                        (Some(p), Some(v)) if v < p => ("down", m.trend.fall()),
                        _ => ("", ""),
                    };
                    prev = value;
                    FundCell { display, dir, sense }
                })
                .collect();
            FundRow {
                label: m.label.to_string(),
                cells,
            }
        })
        .collect();
    FundTable {
        periods: periods.iter().map(|(_, p)| p.clone()).collect(),
        rows,
    }
}

/// The flow / per-share metrics whose Q4 can be derived as the full fiscal
/// year minus its first three quarters. The balance-sheet metrics are excluded
/// — a year-end balance is a snapshot, not a sum of quarters.
const Q4_DERIVABLE: &[&str] = &["revenue", "net_income", "eps_diluted", "dividends_per_share"];

/// Derive the missing Q4 facts. SEC XBRL carries no discrete fourth quarter:
/// there is no Q4 10-Q, so Q4 lives only inside the 10-K's full-year figure.
/// For every fiscal year with the full year and all three prior quarters
/// present, Q4 is `FY - (Q1 + Q2 + Q3)` (PLAN.md Phase 23). Diluted EPS does
/// not decompose perfectly — the diluted share count drifts quarter to quarter
/// — but the residual is small and the plan calls for showing it.
fn derive_q4(facts: &[models::FundFact]) -> Vec<models::FundFact> {
    // (metric, fiscal_year) -> fiscal_qtr (None = full year) -> (value, period_end).
    let mut by: HashMap<(&str, i64), HashMap<Option<i64>, (f64, String)>> = HashMap::new();
    for f in facts {
        by.entry((f.metric.as_str(), f.fiscal_year))
            .or_default()
            .insert(f.fiscal_qtr, (f.value, f.period_end.clone()));
    }
    let mut derived = Vec::new();
    for ((metric, year), vals) in by {
        // A genuine Q4 row (rare, but XBRL does carry a few) always wins.
        if !Q4_DERIVABLE.contains(&metric) || vals.contains_key(&Some(4)) {
            continue;
        }
        let (Some(fy), Some(q1), Some(q2), Some(q3)) = (
            vals.get(&None),
            vals.get(&Some(1)),
            vals.get(&Some(2)),
            vals.get(&Some(3)),
        ) else {
            continue;
        };
        derived.push(models::FundFact {
            metric: metric.to_string(),
            period: format!("Q4-{year}"),
            fiscal_year: year,
            fiscal_qtr: Some(4),
            value: fy.0 - q1.0 - q2.0 - q3.0,
            // The synthetic Q4 ends on the FY's period end (Q4 closes the fiscal year).
            period_end: fy.1.clone(),
        });
    }
    derived
}

/// Assemble the fundamentals view from a company's stored facts plus the
/// latest price. `None` when the company has no fundamentals stored yet.
fn build_fundamentals(facts: &[models::FundFact], price: Option<f64>) -> Option<FundamentalsView> {
    if facts.is_empty() {
        return None;
    }

    // SEC XBRL has no discrete Q4 (PLAN.md Phase 23); derive it and fold the
    // derived rows in, so the quarterly periods and the cell lookup below pick
    // them up exactly like a stored fact.
    let derived = derive_q4(facts);
    let facts: Vec<models::FundFact> = facts.iter().cloned().chain(derived).collect();
    let facts: &[models::FundFact] = &facts;

    // (metric, period) -> value, for table-cell lookup.
    let mut lookup: HashMap<(String, String), f64> = HashMap::new();
    for f in facts {
        lookup.insert((f.metric.clone(), f.period.clone()), f.value);
    }

    // Distinct annual periods, oldest first, most recent 5 kept.
    let mut annual: Vec<(i64, String)> = facts
        .iter()
        .filter(|f| f.fiscal_qtr.is_none())
        .map(|f| (f.fiscal_year, f.period.clone()))
        .collect();
    annual.sort();
    annual.dedup();
    let annual: Vec<(i64, String)> = annual.into_iter().rev().take(5).rev().collect();

    // Distinct quarterly periods, oldest first, most recent 8 kept.
    let mut quarterly: Vec<(i64, i64, String)> = facts
        .iter()
        .filter_map(|f| f.fiscal_qtr.map(|q| (f.fiscal_year, q, f.period.clone())))
        .collect();
    quarterly.sort();
    quarterly.dedup();
    let quarterly: Vec<(i64, String)> = quarterly
        .into_iter()
        .rev()
        .take(8)
        .rev()
        .map(|(y, _, p)| (y, p))
        .collect();

    // Ratios run off the most recent full fiscal year; the shared helper in
    // `models` assembles the inputs so the home ranking grades stocks the
    // same way this page does.
    let latest_fy = annual.last().map(|(y, _)| *y);
    let ratios = models::latest_annual_inputs(facts, price)
        .map(|inputs| compute::compute_ratios(&inputs))
        .unwrap_or_default();

    Some(FundamentalsView {
        basis: latest_fy.map(|y| format!("FY{y}")),
        ratios,
        has_annual: !annual.is_empty(),
        has_quarterly: !quarterly.is_empty(),
        annual: fund_table(&annual, &lookup, false),
        quarterly: fund_table(&quarterly, &lookup, true),
    })
}

// ── ETF fund profile (Phase 18) ────────────────────────────────────────────

/// A `fund_profiles` row as stored.
#[derive(sqlx::FromRow)]
struct FundProfileRow {
    /// `portfolio` or `commodity_trust`.
    kind: String,
    net_assets: Option<f64>,
    holdings_count: Option<i64>,
    report_date: Option<String>,
    /// JSON `[[bucket, percent], ...]`.
    asset_mix: Option<String>,
    /// JSON `[[label, percent], ...]`, from each holding's N-PORT
    /// `<issuerCat>` (Phase 28). Often degenerate on an equity ETF.
    sector_mix: Option<String>,
    /// JSON `[[label, percent], ...]`, from each holding's N-PORT
    /// `<invCountry>` (Phase 28). Often US-dominant.
    geography_mix: Option<String>,
}

/// A `fund_holdings` row as stored.
#[derive(sqlx::FromRow)]
struct HoldingRow {
    rank: i64,
    name: String,
    pct: Option<f64>,
    value_usd: Option<f64>,
}

/// One asset-class slice of an ETF's portfolio mix.
#[derive(Serialize)]
struct AssetSlice {
    label: String,
    /// Percent string, e.g. `99.8%`.
    pct: String,
    /// Segment width 0..100 for the mix bar.
    width: f64,
}

/// One holding row shaped for the page.
#[derive(Serialize)]
struct HoldingView {
    rank: i64,
    name: String,
    /// Weight as a percent string, e.g. `8.42%`.
    weight: String,
    /// Bar width 0..100, scaled so the largest holding shown fills the rail.
    bar_pct: f64,
    /// Position value, compact USD, e.g. `$3.3B`.
    value: String,
}

/// Everything the symbol page's ETF fund-profile section needs.
#[derive(Serialize)]
struct FundView {
    /// A physical-commodity grantor trust (GLD, SLV): holds bullion, not a
    /// securities portfolio, so it has no holdings and no asset mix.
    is_commodity: bool,
    /// Net assets / AUM, compact USD. `None` when the fund reported none.
    net_assets: Option<String>,
    holdings_count: Option<i64>,
    /// The N-PORT "as of" date, `YYYY-MM-DD`.
    report_date: Option<String>,
    asset_mix: Vec<AssetSlice>,
    /// N-PORT issuer-category mix (Phase 28). Empty / single-bucket on an
    /// equity ETF where everything rolls up to one bucket — the template
    /// hides the panel in that case rather than rendering a flat bar.
    sector_mix: Vec<AssetSlice>,
    /// N-PORT issuer-country mix (Phase 28). Empty / US-only on a domestic
    /// ETF; hidden by the template the same way.
    geography_mix: Vec<AssetSlice>,
    holdings: Vec<HoldingView>,
}

/// Parse a stored mix JSON column into the page's `AssetSlice` row shape.
/// Phase 28 calls this for asset / sector / geography mixes alike.
fn parse_mix(json: Option<&str>) -> Vec<AssetSlice> {
    json.and_then(|j| serde_json::from_str::<Vec<(String, f64)>>(j).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|(label, pct)| AssetSlice {
            pct: format!("{pct:.1}%"),
            width: pct.clamp(0.0, 100.0),
            label,
        })
        .collect()
}

/// Assemble the fund view from a stored profile row and its holdings.
fn build_fund(profile: FundProfileRow, holdings: Vec<HoldingRow>) -> FundView {
    let asset_mix = parse_mix(profile.asset_mix.as_deref());
    let sector_mix = parse_mix(profile.sector_mix.as_deref());
    let geography_mix = parse_mix(profile.geography_mix.as_deref());

    // Holdings: each weight bar is scaled to the largest holding shown, so the
    // top position fills the rail and the rest read against it.
    let max_pct = holdings.iter().filter_map(|h| h.pct).fold(0.0_f64, f64::max);
    let holdings = holdings
        .into_iter()
        .map(|h| HoldingView {
            rank: h.rank,
            name: h.name,
            weight: h.pct.map_or_else(|| DASH.to_string(), |p| format!("{p:.2}%")),
            bar_pct: match h.pct {
                Some(p) if max_pct > 0.0 => (p / max_pct * 100.0).clamp(0.0, 100.0),
                _ => 0.0,
            },
            value: h.value_usd.map_or_else(|| DASH.to_string(), fmt_usd_compact),
        })
        .collect();

    FundView {
        is_commodity: profile.kind == "commodity_trust",
        net_assets: profile.net_assets.map(fmt_usd_compact),
        holdings_count: profile.holdings_count,
        report_date: profile.report_date,
        asset_mix,
        sector_mix,
        geography_mix,
        holdings,
    }
}

// ── ETF fund metadata + trailing returns (Phase 28) ────────────────────────

/// A `fund_metadata` row as stored.
#[derive(sqlx::FromRow)]
struct FundMetadataRow {
    expense_ratio: Option<f64>,
    yield_pct: Option<f64>,
    trailing_yield_pct: Option<f64>,
    nav_price: Option<f64>,
    inception_date: Option<String>,
    category: Option<String>,
    fund_family: Option<String>,
    strategy_summary: Option<String>,
    /// When the daily `fund_nav` job last refreshed `nav_price` (Phase 4). The
    /// quality read's tracking factor is only graded against a fresh NAV; a
    /// stale one drops the factor rather than asserting a bogus premium.
    nav_synced_at: Option<i64>,
}

/// The "About this fund" section of the ETF symbol page. Every field is
/// pre-formatted, so the template stays declarative; an unpopulated field
/// becomes `—` rather than a hole in the layout.
#[derive(Serialize)]
struct FundMetaView {
    expense_ratio: String,
    yield_pct: String,
    nav_price: Option<f64>,
    /// Pre-formatted premium / discount, e.g. `+0.12%`, with a good/ok/bad
    /// `Grade` so the template can colour-band it. `None` when no NAV.
    premium: Option<PremiumView>,
    inception_date: Option<String>,
    category: Option<String>,
    fund_family: Option<String>,
    strategy_summary: Option<String>,
}

#[derive(Serialize)]
struct PremiumView {
    /// Signed pre-formatted percent, e.g. `+0.12%` / `-0.45%`.
    text: String,
    /// Grade for the semantic colour band: Good (tight), Ok, Bad (wide).
    grade: compute::Grade,
}

fn build_fund_meta(row: FundMetadataRow, price: Option<f64>) -> FundMetaView {
    let pct = |v: Option<f64>, dp: usize| -> String {
        v.map_or_else(|| DASH.to_string(), |x| format!("{:.*}%", dp, x * 100.0))
    };
    // Premium / discount: live price against the latest NAV. Live price falls
    // back to the daily close when no quote yet, just as the ratio cards do.
    let premium = price
        .and_then(|p| compute::premium_discount_pct(p, row.nav_price).map(|pct| (p, pct)))
        .map(|(_, pct)| PremiumView {
            text: format!("{:+.2}%", pct),
            grade: compute::premium_grade(pct),
        });
    FundMetaView {
        expense_ratio: pct(row.expense_ratio, 2),
        yield_pct: pct(row.yield_pct.or(row.trailing_yield_pct), 2),
        nav_price: row.nav_price,
        premium,
        inception_date: row.inception_date,
        category: row.category,
        fund_family: row.fund_family,
        strategy_summary: row.strategy_summary,
    }
}

/// One row of the trailing-returns table, pre-formatted.
#[derive(Serialize)]
struct ReturnRow {
    label: &'static str,
    /// Cumulative percent move, e.g. `+18.27%`. `—` when missing.
    pct: String,
    /// Annualised percent for periods over 1 year, blank `""` for the YTD /
    /// 1m / 3m rows (where annualising is misleading) and `—` when missing.
    annualised: String,
    /// Whether `pct` is positive (green), negative (red), or unknown (none).
    dir: i8,
}

fn fmt_pct(v: Option<f64>) -> (String, i8) {
    match v {
        Some(v) => {
            let dir = if v > 0.0 { 1 } else if v < 0.0 { -1 } else { 0 };
            (format!("{:+.2}%", v), dir)
        }
        None => (DASH.to_string(), 0),
    }
}

fn build_returns(r: &compute::TrailingReturns) -> Vec<ReturnRow> {
    let row = |label: &'static str, tr: Option<compute::TrailingReturn>, annualised: bool| {
        let (pct, dir) = fmt_pct(tr.map(|t| t.pct));
        let annualised = if annualised {
            match tr {
                Some(t) => format!("{:+.2}%", t.annualised_pct),
                None => DASH.to_string(),
            }
        } else {
            String::new()
        };
        ReturnRow {
            label,
            pct,
            annualised,
            dir,
        }
    };
    vec![
        row("1 month", r.m1, false),
        row("3 months", r.m3, false),
        row("Year to date", r.ytd, false),
        row("1 year", r.y1, false),
        row("3 years", r.y3, true),
        row("5 years", r.y5, true),
        row("10 years", r.y10, true),
        row("Since inception", r.since_inception, true),
    ]
}

// ── dividend payouts (Phase 26) ────────────────────────────────────────────

/// One dividend payment, shaped for the page.
#[derive(Serialize)]
struct DividendRow {
    /// Ex-dividend date, `YYYY-MM-DD` (the template's `shortdate` filter
    /// formats it for display).
    ex_date: String,
    /// Per-share amount, formatted as plain dollars, e.g. `$0.24`.
    amount: String,
}

/// Everything the symbol page's Dividends section needs.
#[derive(Serialize)]
struct DividendsView {
    /// Whether the Yahoo dividend sweep has reached this stock yet — picks the
    /// "not synced yet" pending note apart from a genuine no-dividends history.
    synced: bool,
    /// The inferred pace read: cadence, prior-year and YTD totals, projection,
    /// and the on-track grade.
    pace: compute::DividendPace,
    /// Prior-year total per share, formatted, e.g. `$0.92`. Empty string when
    /// there were no payouts in the prior calendar year.
    prior_year_display: String,
    /// YTD total per share, formatted.
    ytd_display: String,
    /// Calendar year YTD belongs to (e.g. `2026`).
    current_year: i32,
    /// Projected current-year total, formatted; `None` when the projection is.
    projection_display: Option<String>,
    /// Signed percent change vs prior year, e.g. `+4.3%`; `None` when the
    /// projection is.
    pct_change_display: Option<String>,
    /// All payouts on file, newest first.
    history: Vec<DividendRow>,
}

/// Load the Dividends section for a stock (PLAN.md Phase 26). Returns `None`
/// when there is nothing to show *and* the sweep has already run: a stock that
/// pays no dividend gets no section. A pending stock (sweep has not reached it
/// yet) still returns a `DividendsView` so the template can render the "not
/// synced yet" note in place.
async fn build_dividends(
    pool: &sqlx::SqlitePool,
    ticker: &str,
    synced: bool,
) -> Option<DividendsView> {
    // Newest first for the per-event history; the pace math wants oldest first.
    let rows: Vec<(String, f64)> = sqlx::query_as(
        "SELECT ex_date, amount FROM dividends WHERE ticker = ? ORDER BY ex_date DESC",
    )
    .bind(ticker)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    // Pending sweep on a stock with no payouts yet — show the pending note.
    if rows.is_empty() && !synced {
        let pace = compute::dividend_pace(&[], chrono::Utc::now().date_naive());
        return Some(DividendsView {
            synced: false,
            pace,
            prior_year_display: String::new(),
            ytd_display: String::new(),
            current_year: chrono::Utc::now().date_naive().year(),
            projection_display: None,
            pct_change_display: None,
            history: Vec::new(),
        });
    }
    // A swept stock with no payouts pays no dividend — hide the section
    // entirely rather than render a heading over an empty table.
    if rows.is_empty() {
        return None;
    }

    let oldest_first: Vec<(String, f64)> = rows.iter().rev().cloned().collect();
    let pace = compute::dividend_pace(&oldest_first, chrono::Utc::now().date_naive());
    // Per-share dividends are usually quoted to the cent; monthly REITs sometimes
    // pay sub-cent amounts (e.g. `$0.0625`), so a sub-cent figure widens to 4dp.
    let fmt_div = |v: f64| if v < 0.01 { format!("${v:.4}") } else { format!("${v:.2}") };
    let history: Vec<DividendRow> = rows
        .iter()
        .map(|(d, a)| DividendRow {
            ex_date: d.clone(),
            amount: fmt_div(*a),
        })
        .collect();
    // Totals and the projection are annual sums of those per-share amounts;
    // keep the same precision rule so a small payout's effect is not rounded off.
    let fmt_money = fmt_div;
    Some(DividendsView {
        synced,
        prior_year_display: fmt_money(pace.prior_year_total),
        ytd_display: fmt_money(pace.ytd_total),
        projection_display: pace.projection.map(fmt_money),
        pct_change_display: pace.pct_change.map(|p| format!("{p:+.1}%")),
        current_year: chrono::Utc::now().date_naive().year(),
        history,
        pace,
    })
}

// ── company leadership (Phase 14) ──────────────────────────────────────────

/// A `leadership` row as stored.
#[derive(sqlx::FromRow)]
struct LeadershipRow {
    name: String,
    is_director: i64,
    is_officer: i64,
    officer_title: Option<String>,
}

/// One person on the leadership roster, shaped for the page.
#[derive(Serialize)]
struct LeaderView {
    /// Display name, title-cased from the as-filed upper-case form.
    name: String,
    /// Role line, e.g. `Chief Executive Officer · Director`.
    role: String,
    /// Sort key only: officers ahead of directors, chiefs first. `serde(skip)`
    /// keeps it out of the template context.
    #[serde(skip)]
    rank: u8,
}

/// One 8-K item-5.02 leadership-change event, shaped for the page.
#[derive(Serialize)]
struct ChangeView {
    filed_at: String,
    url: String,
}

/// Everything the symbol page's Leadership section needs.
#[derive(Serialize)]
struct LeadershipView {
    /// Whether the SEC leadership sweep has reached this stock yet — picks the
    /// "not synced yet" pending note apart from a genuine empty roster.
    synced: bool,
    roster: Vec<LeaderView>,
    /// Recent officer/director changes, newest first.
    changes: Vec<ChangeView>,
}

/// Title-case a name filed in SEC's upper-case form: `COOK TIMOTHY D` ->
/// `Cook Timothy D`. The first letter of each word, and of each part after an
/// apostrophe or hyphen, is capitalized — so `O'BRIEN` reads `O'Brien`. The
/// order is left as filed (last name first); reordering it is unreliable for
/// compound surnames and generational suffixes.
fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut cap_next = true;
    for ch in s.chars() {
        if ch.is_whitespace() || ch == '\'' || ch == '-' {
            out.push(ch);
            cap_next = true;
        } else if cap_next {
            out.extend(ch.to_uppercase());
            cap_next = false;
        } else {
            out.extend(ch.to_lowercase());
        }
    }
    out
}

/// Sort rank for the roster: officers ahead of directors, with the chief
/// executive / financial / operating officers ahead of the other officers.
/// Both the spelled-out titles and the abbreviations are matched, since filers
/// use either (`Chief Executive Officer` or `CEO and Chairman`).
fn role_rank(is_director: bool, is_officer: bool, title: Option<&str>) -> u8 {
    let t = title.unwrap_or("").to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| t.contains(n));
    if has(&["chief executive", "ceo"]) {
        0
    } else if has(&["chief financial", "cfo"]) {
        1
    } else if has(&["chief operating", "coo"]) {
        2
    } else if is_officer {
        3
    } else if is_director {
        4
    } else {
        5
    }
}

/// The role line for a roster row: the officer title (when the filer gave one)
/// and `Director`, joined — e.g. `Chief Financial Officer · Director`.
fn role_text(is_director: bool, is_officer: bool, title: Option<&str>) -> String {
    let mut parts: Vec<String> = Vec::new();
    match title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => parts.push(t.to_string()),
        None if is_officer => parts.push("Officer".to_string()),
        None => {}
    }
    if is_director {
        parts.push("Director".to_string());
    }
    if parts.is_empty() {
        parts.push("Insider".to_string());
    }
    parts.join(" \u{00b7} ")
}

/// Load the Leadership section for a stock: the current officer/board roster
/// and the recent 8-K item-5.02 change events. The roster is filtered to
/// insiders seen filing within the recency window, so people who left long ago
/// drop off (ownership filings carry no explicit departure signal).
async fn build_leadership(pool: &sqlx::SqlitePool, ticker: &str, synced: bool) -> LeadershipView {
    // ~18 months: long enough that an annually-filing director still shows,
    // short enough that a departed insider ages out.
    let cutoff = (chrono::Utc::now().date_naive() - chrono::Duration::days(550)).to_string();
    let rows: Vec<LeadershipRow> = sqlx::query_as(
        "SELECT name, is_director, is_officer, officer_title FROM leadership \
         WHERE ticker = ? AND last_seen >= ?",
    )
    .bind(ticker)
    .bind(&cutoff)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let mut roster: Vec<LeaderView> = rows
        .into_iter()
        .map(|r| {
            let (is_dir, is_off) = (r.is_director != 0, r.is_officer != 0);
            LeaderView {
                rank: role_rank(is_dir, is_off, r.officer_title.as_deref()),
                role: role_text(is_dir, is_off, r.officer_title.as_deref()),
                name: title_case(&r.name),
            }
        })
        .collect();
    roster.sort_by(|a, b| a.rank.cmp(&b.rank).then_with(|| a.name.cmp(&b.name)));

    let changes: Vec<ChangeView> = sqlx::query_as::<_, (String, String)>(
        "SELECT filed_at, url FROM filings \
         WHERE ticker = ? AND form LIKE '8-K%' AND items LIKE '%5.02%' \
         ORDER BY filed_at DESC, accession DESC LIMIT 8",
    )
    .bind(ticker)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(filed_at, url)| ChangeView { filed_at, url })
    .collect();

    LeadershipView {
        synced,
        roster,
        changes,
    }
}

// ── earnings dates (Phase 25) ─────────────────────────────────────────────

/// One past earnings date shaped for the page.
#[derive(Serialize)]
struct PastEarningsRow {
    /// `YYYY-MM-DD`; the template's `shortdate` filter formats it.
    date: String,
    /// Days from today; positive for past dates.
    days_ago: i64,
}

/// Everything the symbol-page Earnings section needs. Stocks only — every
/// caller gates the build on `kind == "stock"`.
#[derive(Serialize)]
struct EarningsView {
    /// Most recent past earnings date (`YYYY-MM-DD`), with a days-ago figure.
    most_recent: Option<PastEarningsRow>,
    /// Next-expected earnings date (`YYYY-MM-DD`) and days-from-today.
    next_date: Option<String>,
    next_days: Option<i64>,
    /// Where the next date came from: `yahoo` (authoritative), `estimate`
    /// (cadence projection), or `unknown` (Yahoo has no date and we cannot
    /// estimate one — too few priors).
    next_source: &'static str,
    /// The last few past earnings dates, newest first. Capped to 4 (one
    /// trailing year of a quarterly cadence) per the design pass.
    past: Vec<PastEarningsRow>,
    /// All past earnings dates surfaced to the chart as ink pips above
    /// each matching candle. Kept here so the route's history API can
    /// echo them into the chart payload.
    chart_dates: Vec<String>,
    /// When this stock's earnings-calendar sync last ran, for the section
    /// caption. NULL when Yahoo has never been hit for this stock; the page
    /// then shows the past dates and the cadence estimate without the "as of"
    /// line so it does not lie about a sync that did not happen.
    earnings_synced_at: Option<i64>,
}

/// How many past earnings dates to list on the page. Four covers one trailing
/// year of a quarterly cadence; the chart pips show all of them up to the
/// chart's visible range.
const EARNINGS_PAST_LIMIT: usize = 4;

/// Load past earnings dates from `filings.items LIKE '%2.02%'` (Phase 14
/// stored 8-K item codes). Newest first; capped to a generous window so a
/// company that moved its reporting day still produces a clean median.
async fn load_past_earnings(pool: &sqlx::SqlitePool, ticker: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT filed_at FROM filings \
         WHERE ticker = ? AND form LIKE '8-K%' AND items LIKE '%2.02%' \
         ORDER BY filed_at DESC, accession DESC LIMIT 16",
    )
    .bind(ticker)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// Build the Earnings section for a stock. Returns `None` when SEC has not
/// synced yet (no past dates to anchor the section) and Yahoo also carries
/// no next date — the section is hidden cleanly in that case.
async fn build_earnings(
    pool: &sqlx::SqlitePool,
    ticker: &str,
    next_earnings_at: Option<i64>,
    earnings_synced_at: Option<i64>,
) -> Option<EarningsView> {
    let past_dates = load_past_earnings(pool, ticker).await;
    if past_dates.is_empty() && next_earnings_at.is_none() {
        return None;
    }
    let today = chrono::Utc::now().date_naive();
    let days_between = |d: &str| -> Option<i64> {
        chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .ok()
            .map(|nd| (nd - today).num_days())
    };

    let most_recent = past_dates.first().and_then(|d| {
        days_between(d).map(|gap| PastEarningsRow {
            date: d.clone(),
            days_ago: -gap, // gap is negative for past dates; flip to days-ago.
        })
    });

    // Resolve the next date: Yahoo primary, cadence-estimate fallback.
    let (next_date, next_source) = match next_earnings_at {
        Some(ts) => {
            let date = chrono::DateTime::from_timestamp_millis(ts)
                .map(|dt| dt.naive_utc().date().format("%Y-%m-%d").to_string());
            (date, "yahoo")
        }
        None => {
            let date_refs: Vec<&str> = past_dates.iter().map(String::as_str).collect();
            match compute::next_earnings_estimate(&date_refs) {
                Some(d) => (Some(d), "estimate"),
                None => (None, "unknown"),
            }
        }
    };
    let next_days = next_date.as_deref().and_then(days_between);

    let past: Vec<PastEarningsRow> = past_dates
        .iter()
        .take(EARNINGS_PAST_LIMIT)
        .filter_map(|d| {
            days_between(d).map(|gap| PastEarningsRow {
                date: d.clone(),
                days_ago: -gap,
            })
        })
        .collect();

    Some(EarningsView {
        most_recent,
        next_date,
        next_days,
        next_source,
        past,
        chart_dates: past_dates,
        earnings_synced_at,
    })
}

// ── per-ticker anomaly feed (Phase 16) ────────────────────────────────────

/// One row in the anomaly feed, as shaped for the template. Wraps
/// `compute::AnomalyEvent` with no extra fields — re-exposed so the template
/// can iterate a single concrete type regardless of which compute helper
/// (or the leadership-filings SELECT below) produced the row.
type AnomalyRow = compute::AnomalyEvent;

#[derive(Serialize)]
struct AnomalyView {
    events: Vec<AnomalyRow>,
}

/// Display cap on the merged feed. Severity-rank-then-newest the four
/// streams together, then trim to this many before rendering.
const ANOMALY_MAX_EVENTS: usize = 20;
/// How far back the feed reaches.
const ANOMALY_WINDOW_DAYS: i64 = 365;

/// Build the symbol-page anomaly feed: large price moves and new 6-month
/// lows for every symbol with a daily history; YoY fundamentals jumps and
/// 8-K item-5.02 leadership changes additionally for stocks. The feed is
/// trimmed to the past year and capped at [`ANOMALY_MAX_EVENTS`]. Returns
/// `None` when no events qualify, so the template hides the section.
async fn build_anomalies(
    pool: &sqlx::SqlitePool,
    ticker: &str,
    kind: &str,
    bars_newest_first: &[(String, f64, f64, f64, f64, i64)],
    facts: &[models::FundFact],
) -> Option<AnomalyView> {
    let today = chrono::Utc::now().date_naive();
    let cutoff_date = today - chrono::Duration::days(ANOMALY_WINDOW_DAYS);
    let cutoff = cutoff_date.format("%Y-%m-%d").to_string();

    // Price + drawdown events want oldest-first closes paired with dates.
    let oldest_first: Vec<(String, f64)> = bars_newest_first
        .iter()
        .rev()
        .map(|(d, _, _, _, c, _)| (d.clone(), *c))
        .collect();
    let closes: Vec<f64> = oldest_first.iter().map(|(_, c)| *c).collect();
    let dates_refs: Vec<&str> = oldest_first.iter().map(|(d, _)| d.as_str()).collect();

    let mut events: Vec<AnomalyRow> = Vec::new();
    events.extend(compute::price_anomalies(&closes, &dates_refs));
    events.extend(compute::drawdown_anomalies(&closes, &dates_refs));

    // Fundamentals events and leadership events are stocks-only.
    if kind == "stock" {
        events.extend(models::fundamentals_anomalies(facts));
        let lead_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT filed_at, url FROM filings \
             WHERE ticker = ? AND form LIKE '8-K%' AND items LIKE '%5.02%' \
               AND filed_at >= ? \
             ORDER BY filed_at DESC, accession DESC",
        )
        .bind(ticker)
        .bind(&cutoff)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        for (filed_at, url) in lead_rows {
            events.push(AnomalyRow {
                date: filed_at,
                glyph: "leader",
                polarity: "neutral",
                headline: "Officer or director change reported in an 8-K".to_string(),
                url: Some(url),
                // Hand-picked: above a typical 5-8% one-day move so a leadership
                // change is not crowded off the list, below a major drawdown.
                severity: 7.5,
            });
        }
    }

    // Trim to the past year window.
    events.retain(|e| e.date.as_str() >= cutoff.as_str());
    if events.is_empty() {
        return None;
    }
    // Newest first; ties broken by severity so the bigger event of the same
    // day reads first. Then cap.
    events.sort_by(|a, b| {
        b.date
            .cmp(&a.date)
            .then_with(|| b.severity.partial_cmp(&a.severity).unwrap_or(std::cmp::Ordering::Equal))
    });
    events.truncate(ANOMALY_MAX_EVENTS);

    Some(AnomalyView { events })
}

async fn symbol_page(Path(ticker): Path<String>, State(state): State<AppState>) -> Response {
    let ticker = ticker.to_uppercase();

    let symbol = sqlx::query_as::<_, SymbolRow>("SELECT * FROM symbols WHERE ticker = ?")
        .bind(&ticker)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();
    let Some(symbol) = symbol else {
        return not_found(&state);
    };

    // The latest stored live quote, if the symbol has ever been quoted. The
    // header prefers it over the last daily close.
    let quote = sqlx::query_as::<_, QuoteRow>("SELECT price, prev_close FROM quotes WHERE ticker = ?")
        .bind(&ticker)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten()
        .map(|q| {
            let change = q.prev_close.map(|p| compute::change(q.price, p));
            HeaderQuote {
                price: q.price,
                change_abs: change.map(|c| c.abs),
                change_pct: change.map(|c| c.pct),
                state_label: quote_state_label().to_string(),
            }
        });

    // Most recent ~1.5 years of daily bars, newest first.
    let bars: Vec<(String, f64, f64, f64, f64, i64)> = sqlx::query_as(
        "SELECT d, open, high, low, close, volume FROM daily_prices \
         WHERE ticker = ? ORDER BY d DESC LIMIT 400",
    )
    .bind(&ticker)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let stats = bars.first().map(|latest| {
        let (date, open, high, low, close, volume) = latest.clone();
        let prev_close = bars.get(1).map(|b| b.4);
        let change = prev_close.map(|p| compute::change(close, p));
        // 52-week range from the most recent ~252 trading days.
        let window = &bars[..bars.len().min(252)];
        let high_52w = window.iter().map(|b| b.2).fold(f64::NEG_INFINITY, f64::max);
        let low_52w = window.iter().map(|b| b.3).fold(f64::INFINITY, f64::min);
        // Average daily volume over the recent ~3-month window (65 sessions).
        let vol_window = &bars[..bars.len().min(65)];
        let avg_volume =
            vol_window.iter().map(|b| b.5).sum::<i64>() / vol_window.len().max(1) as i64;
        let vol_ratio = (avg_volume > 0).then(|| volume as f64 / avg_volume as f64);
        Stats {
            date,
            open,
            high,
            low,
            close,
            volume,
            prev_close,
            change_abs: change.map(|c| c.abs),
            change_pct: change.map(|c| c.pct),
            open_change_pct: prev_close.map(|p| compute::change(open, p).pct),
            high_52w,
            low_52w,
            avg_volume,
            day_open_pos: compute::pos(open, low, high),
            day_close_pos: compute::pos(close, low, high),
            yr_close_pos: compute::pos(close, low_52w, high_52w),
            yr_prev_pos: prev_close.map(|p| compute::pos(p, low_52w, high_52w)),
            // Cap the bar at 2x the average so an outlier session stays on-rail.
            vol_fill_pct: vol_ratio.map_or(0.0, |r| (r / 2.0 * 100.0).clamp(0.0, 100.0)),
            vol_ratio,
        }
    });

    // Fundamentals are stocks-only; an ETF gets a fund profile instead; an
    // index gets neither. Filings cover both stocks and ETFs.
    let is_stock = symbol.kind == "stock";
    let is_etf = symbol.kind == "etf";
    // Ratios price off the live quote, falling back to the last daily close.
    let price = quote
        .as_ref()
        .map(|q| q.price)
        .or_else(|| stats.as_ref().map(|s| s.close));

    // Plain-language read of the chart's indicators (RSI verdict + price vs each
    // moving average), shown beneath the chart. Built from the daily closes
    // (oldest first) against the current price; `None` without enough history.
    let indicators = price.and_then(|p| {
        // `bars` is newest-first; the indicator maths want oldest-first.
        let highs: Vec<f64> = bars.iter().rev().map(|b| b.2).collect();
        let lows: Vec<f64> = bars.iter().rev().map(|b| b.3).collect();
        let closes: Vec<f64> = bars.iter().rev().map(|b| b.4).collect();
        build_indicator_read(&highs, &lows, &closes, p, symbol.kind != "index")
    });

    // Stock fundamentals are loaded once and shared by the ratio cards
    // (`build_fundamentals`) and the anomaly feed's YoY detector
    // (`build_anomalies` via `models::fundamentals_anomalies`).
    let facts: Vec<models::FundFact> = if is_stock {
        sqlx::query_as(
            "SELECT metric, period, fiscal_year, fiscal_qtr, value, period_end \
             FROM fundamentals WHERE ticker = ?",
        )
        .bind(&ticker)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default()
    } else {
        Vec::new()
    };
    let fundamentals = if is_stock {
        build_fundamentals(&facts, price)
    } else {
        None
    };

    // The overall strong / fair / weak standing (Phase 20): the ratios above
    // rolled up, with the daily-close trajectory folded into its score. Shown
    // as a single badge over the ratio cards. `bars` is newest-first, so it is
    // reversed into an oldest-first close series.
    let closes_oldest_first: Vec<f64> = if is_stock {
        bars.iter().rev().map(|b| b.4).collect()
    } else {
        Vec::new()
    };
    let standing = fundamentals
        .as_ref()
        .and_then(|f| compute::standing(&f.ratios, &closes_oldest_first));

    // The stock health read (Phase 17): the ratios + trajectory of the
    // standing above, plus a leadership-stability signal read off the recent
    // 8-K item-5.02 change count from Phase 14. `None` until the leadership
    // sweep has reached this stock; the composite then drops that component
    // cleanly instead of penalising an unsynced stock. Stocks only.
    let leadership_changes_recent: Option<usize> = if is_stock
        && symbol.leadership_synced_at.is_some()
    {
        let cutoff = (chrono::Utc::now().date_naive()
            - chrono::Duration::days(compute::LEADERSHIP_STABILITY_DAYS))
        .to_string();
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM filings \
             WHERE ticker = ? AND form LIKE '8-K%' AND items LIKE '%5.02%' \
               AND filed_at >= ?",
        )
        .bind(&ticker)
        .bind(&cutoff)
        .fetch_one(&state.pool)
        .await
        .ok()
        .map(|n| n.max(0) as usize)
    } else {
        None
    };
    let health = fundamentals.as_ref().and_then(|f| {
        compute::health_read(&f.ratios, &closes_oldest_first, leadership_changes_recent)
    });

    let filings: Vec<FilingView> = if is_stock || is_etf {
        sqlx::query_as::<_, FilingRow>(
            "SELECT form, filed_at, period_of_report, url FROM filings \
             WHERE ticker = ? ORDER BY filed_at DESC, accession DESC LIMIT 18",
        )
        .bind(&ticker)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|r| FilingView {
            title: filing_title(&r.form),
            form: r.form,
            filed_at: r.filed_at,
            period_of_report: r.period_of_report,
            url: r.url,
        })
        .collect()
    } else {
        Vec::new()
    };

    // Raw ETF figures captured as the fund / metadata blocks build them, then
    // rolled into the Phase 4 quality read below. Kept as scalars so the read
    // can be computed once both SEC (profile/holdings) and Yahoo (metadata)
    // sources are loaded, without re-querying.
    let mut etf_net_assets: Option<f64> = None;
    let mut etf_top10_pct: Option<f64> = None;
    let mut etf_expense_ratio: Option<f64> = None;
    let mut etf_nav: Option<f64> = None;
    let mut etf_nav_synced_at: Option<i64> = None;

    // The ETF fund profile, when the SEC sweep has reached this symbol.
    let fund = if is_etf {
        let profile = sqlx::query_as::<_, FundProfileRow>(
            "SELECT kind, net_assets, holdings_count, report_date, \
                    asset_mix, sector_mix, geography_mix \
             FROM fund_profiles WHERE ticker = ?",
        )
        .bind(&ticker)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();
        match profile {
            Some(profile) => {
                let holdings = sqlx::query_as::<_, HoldingRow>(
                    "SELECT rank, name, pct, value_usd FROM fund_holdings \
                     WHERE ticker = ? ORDER BY rank",
                )
                .bind(&ticker)
                .fetch_all(&state.pool)
                .await
                .unwrap_or_default();
                etf_net_assets = profile.net_assets;
                // Top-10 concentration for the diversification factor: the
                // summed weight of the ten largest holdings (rows are rank-
                // ordered, `pct` already in percent units). `None` when the
                // fund reported no holdings (a commodity trust), so that factor
                // drops out of the blend rather than reading as zero.
                let top10: f64 = holdings.iter().take(10).filter_map(|h| h.pct).sum();
                etf_top10_pct = (top10 > 0.0).then_some(top10);
                Some(build_fund(profile, holdings))
            }
            None => None,
        }
    } else {
        None
    };

    // ETF fund metadata + trailing returns (Phase 28). Both keyed by the
    // same `is_etf` gate; the fund_metadata row exists once the new Yahoo
    // job has swept this symbol. An unswept ETF shows the section's
    // "pending" note in the template.
    let fund_meta = if is_etf {
        let row = sqlx::query_as::<_, FundMetadataRow>(
            "SELECT expense_ratio, yield_pct, trailing_yield_pct, nav_price, \
                    inception_date, category, fund_family, strategy_summary, \
                    nav_synced_at \
             FROM fund_metadata WHERE ticker = ?",
        )
        .bind(&ticker)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();
        match row {
            Some(row) => {
                etf_expense_ratio = row.expense_ratio;
                etf_nav = row.nav_price;
                etf_nav_synced_at = row.nav_synced_at;
                Some(build_fund_meta(row, price))
            }
            None => None,
        }
    } else {
        None
    };

    // Phase 4 — ETF quality read: cost-weighted blend of cost, tracking (price
    // vs NAV premium), diversification (top-10 concentration), and size (AUM).
    // Mirrors the stock health donut. `None` until ≥2 factors grade, so a fund
    // the sweeps have barely reached gets no badge rather than a hollow one.
    let etf_quality = if is_etf {
        // Only read a price-vs-NAV premium (the tracking factor) against a
        // *fresh* NAV: NAV is struck daily, so a stale one makes the premium
        // meaningless. We let the factor drop out rather than assert a bogus
        // tracking verdict. The daily `fund_nav` job keeps NAV current; when it
        // is behind (fresh deploy, guard tripped), tracking simply reads "—".
        const NAV_FRESH_MS: i64 = 3 * 24 * 3600 * 1000;
        let nav_fresh =
            etf_nav_synced_at.is_some_and(|t| crate::db::now_ms() - t <= NAV_FRESH_MS);
        let premium_pct = if nav_fresh {
            price.and_then(|p| compute::premium_discount_pct(p, etf_nav))
        } else {
            None
        };
        compute::etf_quality(etf_expense_ratio, premium_pct, etf_top10_pct, etf_net_assets)
    } else {
        None
    };
    // Trailing returns reach back as far as the fund's daily history goes
    // (since inception, ten years, ...), so they pull the *full* series for
    // this symbol rather than the 400-bar window the chart's key stats use.
    // ETFs only.
    let returns = if is_etf {
        let full: Vec<(String, f64)> = sqlx::query_as(
            "SELECT d, close FROM daily_prices WHERE ticker = ? ORDER BY d ASC",
        )
        .bind(&ticker)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();
        if full.len() >= 2 {
            let dated: Vec<compute::DatedClose<'_>> = full
                .iter()
                .map(|(d, c)| compute::DatedClose {
                    date: d,
                    close: *c,
                })
                .collect();
            let today = chrono::Utc::now().date_naive().format("%Y-%m-%d").to_string();
            Some(build_returns(&compute::trailing_returns(&dated, &today)))
        } else {
            None
        }
    } else {
        None
    };

    // The leadership roster + change feed (Phase 14): stocks only, like the
    // fundamentals above.
    let leadership = if is_stock {
        Some(build_leadership(&state.pool, &ticker, symbol.leadership_synced_at.is_some()).await)
    } else {
        None
    };

    // Dividend / distribution payouts (Phase 26 + Phase 28): now covers
    // stocks AND ETFs. Indexes and futures have no concept and get no
    // section; an unswept symbol shows a pending note in place; a swept one
    // with no payouts in the past five years hides the section.
    let dividends = if is_stock || is_etf {
        build_dividends(&state.pool, &ticker, symbol.dividends_synced_at.is_some()).await
    } else {
        None
    };

    // Per-ticker anomaly feed (Phase 16). All instruments get price-based
    // events (large daily moves, new 6-month lows); stocks additionally get
    // YoY fundamentals jumps and 8-K item-5.02 leadership changes. Returns
    // `None` when the symbol has no qualifying events in the past year so
    // the template hides the section.
    let anomalies = build_anomalies(&state.pool, &ticker, &symbol.kind, &bars, &facts).await;

    // Earnings dates (Phase 25). Stocks only; the past dates ride for free
    // off the existing 8-K item-2.02 filings (Phase 14 stored the `items`
    // column), the next date is either Yahoo's `calendarEvents` or a cadence
    // estimate from those past dates. The chart pips also read off the past
    // dates carried in `earnings.chart_dates`.
    let earnings = if is_stock {
        build_earnings(
            &state.pool,
            &ticker,
            symbol.next_earnings_at,
            symbol.earnings_synced_at,
        )
        .await
    } else {
        None
    };

    let extra = minijinja::context! {
        title => ticker,
        symbol => symbol,
        stats => stats,
        indicators => indicators,
        quote => quote,
        fundamentals => fundamentals,
        standing => standing,
        health => health,
        fund => fund,
        fund_meta => fund_meta,
        etf_quality => etf_quality,
        returns => returns,
        leadership => leadership,
        dividends => dividends,
        anomalies => anomalies,
        earnings => earnings,
        filings => filings,
    };
    render(&state, "pages/symbol.html", &format!("/s/{ticker}"), extra)
}

#[derive(Deserialize)]
struct HistoryQuery {
    range: Option<String>,
}

/// A bar's time on the chart axis. Daily bars are calendar dates
/// (`YYYY-MM-DD`); intraday bars (the 1D / 1W ranges) are UNIX seconds, which
/// is the other form lightweight-charts accepts for an intraday time scale.
/// `#[serde(untagged)]` so each variant serialises as a bare string or number
/// — no tag wrapper the chart would have to unpick.
#[derive(Serialize)]
#[serde(untagged)]
enum BarTime {
    Date(String),
    Unix(i64),
}

/// One OHLCV point shaped for lightweight-charts.
#[derive(Serialize)]
struct Candle {
    time: BarTime,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: i64,
}

/// One point of a derived overlay/indicator series, shaped for
/// lightweight-charts (`time` is `YYYY-MM-DD`). Sparse: bars with no value
/// yet (an average's warm-up period) are simply omitted.
#[derive(Serialize)]
struct LinePoint {
    time: String,
    value: f64,
}

/// One earnings-date marker for the chart. `YYYY-MM-DD` (matches the
/// candle `time` field), so the client can index it against the candle
/// series directly.
#[derive(Serialize)]
struct EarningsMarker {
    time: String,
}

/// One Supertrend point for the chart: the band value plus whether the trend is
/// up, so the client can split the single line into a green (uptrend) and a red
/// (downtrend) series with a clean break at each flip.
#[derive(Serialize)]
struct SuperTrendPoint {
    time: String,
    value: f64,
    up: bool,
}

/// The symbol chart payload (Phase 8 + Phase 28): the candles for the
/// selected range plus the indicator overlays, each already trimmed to the
/// visible window. Phase 28 adds an optional benchmark series — the
/// curated index a fund tracks, normalised to the same start point as the
/// fund's first visible close — rendered as a relative-performance line
/// only when the symbol has a `symbols.benchmark` configured.
#[derive(Serialize)]
struct HistoryResponse {
    candles: Vec<Candle>,
    sma50: Vec<LinePoint>,
    sma200: Vec<LinePoint>,
    ema21: Vec<LinePoint>,
    rsi14: Vec<LinePoint>,
    /// Supertrend band (ATR 10 / 3×). Each point carries its trend side so the
    /// client draws it green below price in an uptrend, red above in a
    /// downtrend. Empty on the intraday ranges (a daily-only overlay).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    supertrend: Vec<SuperTrendPoint>,
    /// Benchmark closes scaled to the same starting price as the visible
    /// candles, so the two lines start together and drift apart on relative
    /// performance. Empty when no benchmark is configured or no benchmark
    /// history overlaps the range.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    benchmark: Vec<LinePoint>,
    /// Curated benchmark ticker label for the chart legend (e.g. `^SPX`).
    /// Absent when no benchmark is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    benchmark_ticker: Option<String>,
    /// Past earnings-date markers for the chart (Phase 25). Each is a
    /// `YYYY-MM-DD` matching one of the visible candles; the client draws
    /// a small ink dot above each matching bar. Stocks only; empty otherwise.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    earnings: Vec<EarningsMarker>,
    /// The prior daily close (Phase 6). Carried only for the intraday ranges,
    /// where the chart draws it as a dashed reference line so the day's move is
    /// legible against where the symbol opened the session.
    #[serde(skip_serializing_if = "Option::is_none")]
    prev_close: Option<f64>,
    /// True when `candles` carry intraday (UNIX-seconds) times rather than
    /// daily dates (Phase 6). The chart switches its axis/labels accordingly
    /// and suppresses the daily-only overlays.
    intraday: bool,
}

/// Earliest `YYYY-MM-DD` to *show* for a range button. `None` means no limit.
fn range_cutoff(range: &str) -> Option<String> {
    let today = chrono::Utc::now().date_naive();
    match range {
        "MAX" => None,
        "YTD" => Some(format!("{:04}-01-01", today.year())),
        "1M" => Some((today - chrono::Duration::days(31)).to_string()),
        "3M" => Some((today - chrono::Duration::days(93)).to_string()),
        "6M" => Some((today - chrono::Duration::days(186)).to_string()),
        "3Y" => Some((today - chrono::Duration::days(1098)).to_string()),
        "5Y" => Some((today - chrono::Duration::days(1830)).to_string()),
        // 1Y is the default for any unrecognised value.
        _ => Some((today - chrono::Duration::days(366)).to_string()),
    }
}

/// Trading days of history the longest indicator (the 200-day average) needs
/// before the visible window, expressed as calendar days with comfortable
/// slack for weekends and holidays.
const INDICATOR_LOOKBACK_DAYS: i64 = 320;

async fn history_api(
    Path(ticker): Path<String>,
    Query(q): Query<HistoryQuery>,
    State(state): State<AppState>,
) -> Response {
    let ticker = ticker.to_uppercase();
    let range = q.range.unwrap_or_else(|| "1Y".to_string());

    // The 1D / 1W ranges (Phase 6) draw today's real-time 15-minute bars on an
    // intraday axis, live-ticked by the quote stream — a different data source
    // (`intraday_bars`) and time format from the daily candles, so they take
    // their own path and the daily indicator machinery never runs for them.
    if range == "1D" || range == "1W" {
        return intraday_history(&state.pool, &ticker, &range).await;
    }

    let display_cutoff = range_cutoff(&range);

    // Indicators need history *before* the visible window or their first
    // values would be blank — a 200-day average needs 200 prior bars. Fetch a
    // fixed lookback before the range cutoff, compute over the whole set, then
    // trim every series back to the visible window.
    let fetch_cutoff = display_cutoff.as_deref().map(|c| {
        chrono::NaiveDate::parse_from_str(c, "%Y-%m-%d")
            .map(|d| (d - chrono::Duration::days(INDICATOR_LOOKBACK_DAYS)).to_string())
            .unwrap_or_else(|_| c.to_string())
    });

    // Daily rows as tuples (date, OHLCV). The indicator maths and the benchmark
    // overlay key off the date strings, so they are kept in a parallel `dates`
    // vec and the `Candle`s (whose `time` is now the `BarTime` enum) are built
    // from the same rows at the end.
    let rows: Vec<(String, f64, f64, f64, f64, i64)> = match &fetch_cutoff {
        Some(cutoff) => {
            sqlx::query_as(
                "SELECT d, open, high, low, close, volume FROM daily_prices \
                 WHERE ticker = ? AND d >= ? ORDER BY d ASC",
            )
            .bind(&ticker)
            .bind(cutoff)
            .fetch_all(&state.pool)
            .await
        }
        None => {
            sqlx::query_as(
                "SELECT d, open, high, low, close, volume FROM daily_prices \
                 WHERE ticker = ? ORDER BY d ASC",
            )
            .bind(&ticker)
            .fetch_all(&state.pool)
            .await
        }
    }
    .unwrap_or_default();

    let dates: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
    let highs: Vec<f64> = rows.iter().map(|r| r.2).collect();
    let lows: Vec<f64> = rows.iter().map(|r| r.3).collect();
    let closes: Vec<f64> = rows.iter().map(|r| r.4).collect();

    // First bar inside the visible window; everything before it is lookback,
    // fetched only so the indicators are correct from the very first shown bar.
    let start = match &display_cutoff {
        Some(c) => dates
            .iter()
            .position(|d| d.as_str() >= c.as_str())
            .unwrap_or(dates.len()),
        None => 0,
    };

    // Zip a raw indicator series to its bar dates, dropping warm-up `None`s
    // and the lookback bars, leaving only points inside the visible window.
    let line = |series: Vec<Option<f64>>| -> Vec<LinePoint> {
        series
            .into_iter()
            .enumerate()
            .skip(start)
            .filter_map(|(i, v)| {
                v.map(|value| LinePoint {
                    time: dates[i].clone(),
                    value,
                })
            })
            .collect()
    };

    // Benchmark overlay (Phase 28). Loaded only when the symbol has a
    // curated `symbols.benchmark` set, and only when the visible range
    // actually has anchor bars to scale against. The benchmark series is
    // pinned to the fund's first visible close so the two lines start
    // together and drift apart on relative performance.
    let benchmark_ticker: Option<String> = sqlx::query_scalar(
        "SELECT benchmark FROM symbols WHERE ticker = ?",
    )
    .bind(&ticker)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .flatten();
    let benchmark = match (&benchmark_ticker, dates.get(start)) {
        (Some(bench), Some(_)) => {
            load_benchmark_series(&state.pool, bench, &dates[start..], closes[start]).await
        }
        _ => Vec::new(),
    };

    // Earnings-date pips (Phase 25). Stocks only; each pip is dated to an
    // 8-K item-2.02 filing date (Phase 14 already stored those in
    // `filings.items`). The chart maps each `time` to its matching candle.
    let kind: Option<String> = sqlx::query_scalar("SELECT kind FROM symbols WHERE ticker = ?")
        .bind(&ticker)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();
    let earnings = if kind.as_deref() == Some("stock") {
        let dates = load_past_earnings(&state.pool, &ticker).await;
        dates
            .into_iter()
            .map(|time| EarningsMarker { time })
            .collect()
    } else {
        Vec::new()
    };

    // Supertrend, trimmed to the visible window the same way as the lines but
    // keeping each bar's trend side so the client can colour it.
    let supertrend: Vec<SuperTrendPoint> =
        compute::supertrend(&highs, &lows, &closes, compute::SUPERTREND_PERIOD, compute::SUPERTREND_MULT)
            .into_iter()
            .enumerate()
            .skip(start)
            .filter_map(|(i, p)| {
                p.map(|p| SuperTrendPoint {
                    time: dates[i].clone(),
                    value: p.value,
                    up: p.up,
                })
            })
            .collect();

    let resp = HistoryResponse {
        sma50: line(compute::sma(&closes, 50)),
        sma200: line(compute::sma(&closes, 200)),
        ema21: line(compute::ema(&closes, 21)),
        rsi14: line(compute::rsi(&closes, 14)),
        supertrend,
        candles: rows
            .into_iter()
            .skip(start)
            .map(|(d, open, high, low, close, volume)| Candle {
                time: BarTime::Date(d),
                open,
                high,
                low,
                close,
                volume,
            })
            .collect(),
        benchmark,
        benchmark_ticker,
        earnings,
        prev_close: None,
        intraday: false,
    };

    Json(resp).into_response()
}

/// Calendar days of intraday history the 1W range shows. `intraday_bars` is
/// pruned to a 14-day window (see `INTRADAY_RETENTION_DAYS`), so a week sits
/// comfortably inside what is stored.
const INTRADAY_WEEK_DAYS: i64 = 7;

/// Serve the 1D / 1W intraday ranges (Phase 6) from `intraday_bars`. The bars
/// are stored as UTC epoch-*milliseconds*; lightweight-charts wants UNIX
/// *seconds* for an intraday axis, so each `ts` is divided by 1000. 1D shows
/// the most recent trading day present (so a weekend correctly shows Friday);
/// 1W shows a rolling seven days. None of the daily-only overlays apply, so the
/// indicator series come back empty and the chart hides their toggles.
async fn intraday_history(pool: &sqlx::SqlitePool, ticker: &str, range: &str) -> Response {
    use chrono::{TimeZone as _, Utc};
    use chrono_tz::America::New_York;

    let now_ms = chrono::Utc::now().timestamp_millis();
    let cutoff_ms: i64 = if range == "1W" {
        now_ms - INTRADAY_WEEK_DAYS * 86_400_000
    } else {
        // 1D: the New-York midnight that opens the most recent day with bars,
        // so the view is exactly that session (and weekends show Friday).
        let latest: Option<i64> =
            sqlx::query_scalar("SELECT MAX(ts) FROM intraday_bars WHERE ticker = ?")
                .bind(ticker)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten();
        match latest {
            Some(ms) => Utc
                .timestamp_millis_opt(ms)
                .single()
                .map(|dt| {
                    let day = dt.with_timezone(&New_York).date_naive();
                    New_York
                        .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
                        .single()
                        .map(|midnight| midnight.timestamp_millis())
                        .unwrap_or(ms)
                })
                .unwrap_or(now_ms),
            None => now_ms,
        }
    };

    let rows: Vec<(i64, f64, f64, f64, f64, i64)> = sqlx::query_as(
        "SELECT ts, open, high, low, close, volume FROM intraday_bars \
         WHERE ticker = ? AND ts >= ? ORDER BY ts ASC",
    )
    .bind(ticker)
    .bind(cutoff_ms)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let candles = rows
        .into_iter()
        .map(|(ts, open, high, low, close, volume)| Candle {
            time: BarTime::Unix(ts / 1000),
            open,
            high,
            low,
            close,
            volume,
        })
        .collect();

    // The prior daily close anchors the session reference line. During a live
    // session today's bar is not yet in `daily_prices`, so the most recent row
    // is genuinely the previous close.
    let prev_close: Option<f64> =
        sqlx::query_scalar("SELECT close FROM daily_prices WHERE ticker = ? ORDER BY d DESC LIMIT 1")
            .bind(ticker)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();

    let resp = HistoryResponse {
        candles,
        sma50: Vec::new(),
        sma200: Vec::new(),
        ema21: Vec::new(),
        rsi14: Vec::new(),
        supertrend: Vec::new(),
        benchmark: Vec::new(),
        benchmark_ticker: None,
        earnings: Vec::new(),
        prev_close,
        intraday: true,
    };

    Json(resp).into_response()
}

/// Load a benchmark index's daily closes across the same date span as
/// `visible_dates` (the fund's visible candle dates), then scale each close so the
/// series starts at `fund_anchor` — the fund's first visible close — and
/// only the *relative* movement past that point is plotted. An empty
/// benchmark history or no overlap returns an empty vec, which the
/// `skip_serializing_if` on the response field then drops cleanly.
async fn load_benchmark_series(
    pool: &sqlx::SqlitePool,
    benchmark: &str,
    visible_dates: &[String],
    fund_anchor: f64,
) -> Vec<LinePoint> {
    let Some(first) = visible_dates.first() else {
        return Vec::new();
    };
    let last = visible_dates.last().unwrap_or(first);
    let rows: Vec<(String, f64)> = sqlx::query_as(
        "SELECT d, close FROM daily_prices \
         WHERE ticker = ? AND d >= ? AND d <= ? ORDER BY d ASC",
    )
    .bind(benchmark)
    .bind(first)
    .bind(last)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    if rows.len() < 2 {
        return Vec::new();
    }
    let bench_anchor = rows[0].1;
    if bench_anchor <= 0.0 {
        return Vec::new();
    }
    let scale = fund_anchor / bench_anchor;
    rows.into_iter()
        .map(|(d, c)| LinePoint {
            time: d,
            value: c * scale,
        })
        .collect()
}

// ── ETF growth-of-$10k chart (Phase 28) ────────────────────────────────────

/// Response body for `GET /api/symbols/{ticker}/growth`. Two series scaled
/// so the first point of each reads as $10,000, drawn together so a fund's
/// since-inception path can be eyeballed against its benchmark's.
#[derive(Serialize)]
struct GrowthResponse {
    /// Fund growth series, oldest first.
    fund: Vec<compute::GrowthPoint>,
    /// Benchmark growth series across the same date span, anchored
    /// separately to $10,000 at its own first bar. Empty when the fund has
    /// no curated benchmark or no benchmark history overlaps.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    benchmark: Vec<compute::GrowthPoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    benchmark_ticker: Option<String>,
}

/// Build the growth-of-$10k series over the longest available daily history
/// for `ticker`. ETF symbol page only; on a symbol with no daily history
/// (e.g. a future) the series is empty and the panel hides itself.
async fn growth_api(Path(ticker): Path<String>, State(state): State<AppState>) -> Response {
    let ticker = ticker.to_uppercase();
    let rows: Vec<(String, f64)> = sqlx::query_as(
        "SELECT d, close FROM daily_prices WHERE ticker = ? ORDER BY d ASC",
    )
    .bind(&ticker)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    let bars: Vec<compute::DatedClose<'_>> = rows
        .iter()
        .map(|(d, c)| compute::DatedClose {
            date: d.as_str(),
            close: *c,
        })
        .collect();
    let fund = compute::growth_of_10k(&bars);

    let (benchmark_ticker, benchmark) = match (
        sqlx::query_scalar::<_, Option<String>>("SELECT benchmark FROM symbols WHERE ticker = ?")
            .bind(&ticker)
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten()
            .flatten(),
        fund.first(),
    ) {
        (Some(bench), Some(first)) => {
            // Anchor benchmark to the same first-bar date as the fund, so the
            // two lines start together; benchmark history before the fund's
            // inception is ignored.
            let bench_rows: Vec<(String, f64)> = sqlx::query_as(
                "SELECT d, close FROM daily_prices \
                 WHERE ticker = ? AND d >= ? ORDER BY d ASC",
            )
            .bind(&bench)
            .bind(&first.date)
            .fetch_all(&state.pool)
            .await
            .unwrap_or_default();
            let bench_bars: Vec<compute::DatedClose<'_>> = bench_rows
                .iter()
                .map(|(d, c)| compute::DatedClose {
                    date: d.as_str(),
                    close: *c,
                })
                .collect();
            (Some(bench), compute::growth_of_10k(&bench_bars))
        }
        _ => (None, Vec::new()),
    };

    Json(GrowthResponse {
        fund,
        benchmark,
        benchmark_ticker,
    })
    .into_response()
}

// ── add a symbol to the universe (Phase 9) ─────────────────────────────────

/// `POST /api/symbols` request body.
#[derive(Deserialize)]
struct AddSymbolBody {
    ticker: String,
}

/// `POST /api/symbols` response. `ok` is the success flag the Search page's
/// script keys on; `error` carries a human message on a failure.
#[derive(Serialize)]
struct AddSymbolResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    ticker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    /// True when this call created the symbol; false when it already existed.
    added: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Normalise and validate a user-supplied ticker. Accepts uppercase letters,
/// digits and `. - ^ =` (covering `BRK.B`, `^SPX`, the Yahoo future `CL=F` and
/// the like); rejects the empty string, an over-long one, an unexpected
/// character, or one carrying no alphanumeric. Returns the normalised
/// (trimmed, uppercased) form.
///
/// `pub(crate)` so the Search page offers an "Add" affordance for exactly the
/// strings this endpoint would accept.
pub(crate) fn valid_ticker(raw: &str) -> Option<String> {
    let t = raw.trim().to_uppercase();
    if t.is_empty() || t.len() > 15 {
        return None;
    }
    let charset_ok = t
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '^' | '='));
    let has_alnum = t.chars().any(|c| c.is_ascii_alphanumeric());
    (charset_ok && has_alnum).then_some(t)
}

/// A failed `POST /api/symbols` response with a status and a human message.
fn add_err(status: StatusCode, msg: impl Into<String>) -> Response {
    (
        status,
        Json(AddSymbolResponse {
            ok: false,
            ticker: None,
            name: None,
            kind: None,
            added: false,
            error: Some(msg.into()),
        }),
    )
        .into_response()
}

/// `POST /api/symbols` — add a symbol to the tracked universe.
///
/// The Search page calls this when a query names a ticker the universe does
/// not hold yet. The ticker is validated against Yahoo — one request that also
/// yields its name, kind, exchange and currency — then the symbol row is
/// inserted, the quote that same lookup returned is stored, and the symbol's
/// full backfill (deep daily history and all SEC data) is pulled synchronously
/// before the response, so its page is complete the moment the add returns
/// (PLAN.md Phase 21; see `scheduler::backfill_symbol`). Every outbound call
/// goes through the shared endpoint guard (see PLAN.md's anti-spam policy).
/// The outcome of ensuring a symbol is in the tracked universe.
pub(crate) struct EnsureOutcome {
    pub ticker: String,
    pub name: String,
    pub kind: String,
    /// True when this call created the symbol; false when it already existed.
    pub added: bool,
}

/// Ensure `ticker` is a tracked symbol, adding it to the universe if missing.
///
/// Idempotent: an already-tracked symbol returns at once. A new one is validated
/// against Yahoo (one guarded lookup that also yields its name / kind / exchange
/// / currency), inserted as a user-added (`is_seeded = 0`) row, has the lookup's
/// quote + bars stored, and its full backfill (deep history + SEC data) pulled
/// synchronously, so its page is complete on return. Errors come back as a
/// (status, message) pair the caller surfaces. Shared by `POST /api/symbols`
/// (the Search "Add") and the dashboard watchlist add.
pub(crate) async fn ensure_symbol(
    state: &AppState,
    raw_ticker: &str,
) -> Result<EnsureOutcome, (StatusCode, String)> {
    let Some(ticker) = valid_ticker(raw_ticker) else {
        return Err((
            StatusCode::BAD_REQUEST,
            "That does not look like a ticker symbol.".into(),
        ));
    };

    // Already tracked? Idempotent — report it so the caller can just navigate.
    let existing: Option<(String, String)> =
        sqlx::query_as("SELECT name, kind FROM symbols WHERE ticker = ?")
            .bind(&ticker)
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten();
    if let Some((name, kind)) = existing {
        return Ok(EnsureOutcome { ticker, name, kind, added: false });
    }

    // One guarded Yahoo lookup: validates the symbol and describes it.
    let yahoo = YahooProvider::new(http::build_client(&state.config));
    let guard = EndpointGuard::with_budget(state.pool.clone(), "yahoo", scheduler::YAHOO_BUDGET);
    match guard.acquire().await {
        Ok(Permit::Granted) => {}
        Ok(Permit::Denied(_)) => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "The market data source is busy right now. Try again in a few minutes.".into(),
            ));
        }
        Err(e) => {
            tracing::error!("ensure_symbol guard for {ticker}: {e:#}");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Something went wrong. Try again shortly.".into(),
            ));
        }
    }

    let (info, data) = match yahoo.lookup(&ticker).await {
        Err(e) => {
            let _ = guard.record_failure(&e).await;
            tracing::warn!("ensure_symbol lookup {ticker}: {e:#}");
            return Err((
                StatusCode::BAD_GATEWAY,
                "Could not reach the market data source. Try again shortly.".into(),
            ));
        }
        Ok(outcome) => {
            // The endpoint answered — even an "unknown symbol" is a healthy
            // reply, so the guard records a success either way.
            let _ = guard.record_success().await;
            match outcome {
                SymbolLookup::Found { info, data } => (info, data),
                SymbolLookup::Unknown => {
                    return Err((
                        StatusCode::NOT_FOUND,
                        format!("No symbol called {ticker} was found."),
                    ));
                }
                SymbolLookup::Unsupported(raw_kind) => {
                    let what = raw_kind.to_lowercase();
                    return Err((StatusCode::UNPROCESSABLE_ENTITY, format!(
                        "{ticker} is a {what}. Only stocks, ETFs, indexes, and futures can be added right now."
                    )));
                }
            }
        }
    };

    // Insert the symbol. User-added, so `is_seeded = 0`.
    let now = now_ms();
    let inserted = sqlx::query(
        "INSERT INTO symbols (ticker, name, kind, exchange, currency, is_seeded, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, 0, ?, ?) ON CONFLICT(ticker) DO NOTHING",
    )
    .bind(&ticker)
    .bind(&info.name)
    .bind(&info.kind)
    .bind(&info.exchange)
    .bind(&info.currency)
    .bind(now)
    .bind(now)
    .execute(&state.pool)
    .await;
    if let Err(e) = inserted {
        tracing::error!("ensure_symbol insert {ticker}: {e:#}");
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not save the symbol. Try again shortly.".into(),
        ));
    }

    // Store the quote (and bars) the lookup already paid for, then pull the full
    // backfill before returning, so the symbol's page is complete at once.
    if let Err(e) = scheduler::store_quote(&state.pool, &ticker, &data.quote).await {
        tracing::warn!("ensure_symbol store_quote {ticker}: {e:#}");
    }
    if !data.bars.is_empty() {
        if let Err(e) = scheduler::store_intraday(&state.pool, &ticker, &data.bars).await {
            tracing::warn!("ensure_symbol store_intraday {ticker}: {e:#}");
        }
    }
    scheduler::backfill_symbol(&state.pool, &state.config, &ticker, &info.kind).await;

    tracing::info!("ensure_symbol: added {ticker} ({}, {})", info.name, info.kind);
    Ok(EnsureOutcome { ticker, name: info.name, kind: info.kind, added: true })
}

/// `POST /api/symbols` — add a symbol to the tracked universe (the Search "Add").
async fn add_symbol(State(state): State<AppState>, Json(body): Json<AddSymbolBody>) -> Response {
    match ensure_symbol(&state, &body.ticker).await {
        Ok(o) => Json(AddSymbolResponse {
            ok: true,
            ticker: Some(o.ticker),
            name: Some(o.name),
            kind: Some(o.kind),
            added: o.added,
            error: None,
        })
        .into_response(),
        Err((status, msg)) => add_err(status, msg),
    }
}

#[derive(Deserialize)]
struct RefreshQuery {
    /// `1` = the manual Refresh button: re-pull every source regardless of
    /// staleness. Default `0` = a page load: pull the live price always, the
    /// slow SEC / metadata sources only when their stored copy is stale.
    #[serde(default)]
    force: u8,
}

/// `GET /api/symbols/{ticker}/refresh` — Server-Sent Events driving the symbol
/// page's loading bar. Asks the scheduler which steps to run for this symbol
/// (kind + staleness + `force`), then runs each in turn, emitting a `step`
/// event before and after so the bar advances and names what it is doing. A
/// final `done` event tells the page whether a deep (server-rendered) section
/// changed — if so it reloads to show it; otherwise the live price was already
/// patched in place over the stream and no reload is needed.
async fn refresh_stream(
    Path(ticker): Path<String>,
    Query(q): Query<RefreshQuery>,
    State(state): State<AppState>,
) -> Response {
    let ticker = ticker.to_uppercase();
    let force = q.force != 0;

    let kind: Option<String> = sqlx::query_scalar("SELECT kind FROM symbols WHERE ticker = ?")
        .bind(&ticker)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();
    let Some(kind) = kind else {
        return not_found(&state);
    };

    let steps = scheduler::refresh_plan(&state.pool, &state.config, &ticker, &kind, force).await;
    let any_deep = steps.iter().any(|s| s.deep);

    let body = async_stream::stream! {
        let total = steps.len();
        // Announce the plan up front so the bar can size itself.
        yield sse_json("plan", &format!("{{\"total\":{total}}}"));

        for (i, st) in steps.iter().enumerate() {
            yield sse_json(
                "step",
                &format!(
                    "{{\"i\":{},\"n\":{},\"label\":{},\"state\":\"running\"}}",
                    i + 1, total, json_str(st.label)
                ),
            );
            let status =
                scheduler::refresh_step(&state.pool, &state.config, &state.hub, &ticker, &kind, st.key)
                    .await;
            yield sse_json(
                "step",
                &format!(
                    "{{\"i\":{},\"n\":{},\"label\":{},\"state\":{}}}",
                    i + 1, total, json_str(st.label), json_str(status)
                ),
            );
        }

        yield sse_json("done", &format!("{{\"reload\":{}}}", any_deep));
    };

    Sse::new(body)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response()
}

/// Build a named SSE event with a raw JSON `data` payload.
fn sse_json(event: &str, data: &str) -> Result<axum::response::sse::Event, std::convert::Infallible> {
    Ok(axum::response::sse::Event::default().event(event).data(data))
}

/// Minimal JSON string escaper for the short, known labels/statuses streamed
/// above (no control characters in play; just quote the value safely).
fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}
