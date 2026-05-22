use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::Datelike;
use serde::{Deserialize, Serialize};

use crate::compute;
use crate::db::now_ms;
use crate::guard::{EndpointGuard, Permit};
use crate::market;
use crate::models::SymbolRow;
use crate::providers::http;
use crate::providers::yahoo::{SymbolLookup, YahooProvider};
use crate::render::{not_found, render};
use crate::{scheduler, AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/s/{ticker}", get(symbol_page))
        .route("/api/symbols", post(add_symbol))
        .route("/api/symbols/{ticker}/history", get(history_api))
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

/// Placeholder for a fundamentals cell the company did not report.
const DASH: &str = "\u{00b7}";

/// One fundamentals fact, as stored.
#[derive(sqlx::FromRow)]
struct FundFact {
    metric: String,
    period: String,
    fiscal_year: i64,
    fiscal_qtr: Option<i64>,
    value: f64,
}

/// One row of a financials table: a metric label and its formatted value in
/// each displayed period (`·` where the company reported nothing).
#[derive(Serialize)]
struct FundRow {
    label: String,
    cells: Vec<String>,
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
/// only (see `providers::sec::classify`).
struct TableMetric {
    metric: &'static str,
    label: &'static str,
    is_money: bool,
    in_quarterly: bool,
}

const FUND_TABLE_METRICS: &[TableMetric] = &[
    TableMetric { metric: "revenue", label: "Revenue", is_money: true, in_quarterly: true },
    TableMetric { metric: "net_income", label: "Net income", is_money: true, in_quarterly: true },
    TableMetric { metric: "eps_diluted", label: "Diluted EPS", is_money: false, in_quarterly: true },
    TableMetric { metric: "dividends_per_share", label: "Dividend / share", is_money: false, in_quarterly: true },
    TableMetric { metric: "assets", label: "Total assets", is_money: true, in_quarterly: false },
    TableMetric { metric: "liabilities", label: "Total liabilities", is_money: true, in_quarterly: false },
    TableMetric { metric: "equity", label: "Shareholder equity", is_money: true, in_quarterly: false },
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
/// period_label)`), pulling formatted cells from `lookup`. The quarterly table
/// omits the balance-sheet rows, which are only collected per fiscal year.
fn fund_table(
    periods: &[(i64, String)],
    lookup: &HashMap<(String, String), f64>,
    quarterly: bool,
) -> FundTable {
    let rows = FUND_TABLE_METRICS
        .iter()
        .filter(|m| !quarterly || m.in_quarterly)
        .map(|m| {
            let cells = periods
                .iter()
                .map(
                    |(_, period)| match lookup.get(&(m.metric.to_string(), period.clone())) {
                        Some(v) if m.is_money => fmt_usd_compact(*v),
                        Some(v) => fmt_per_share(*v),
                        None => DASH.to_string(),
                    },
                )
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

/// Assemble the fundamentals view from a company's stored facts plus the
/// latest price. `None` when the company has no fundamentals stored yet.
fn build_fundamentals(facts: &[FundFact], price: Option<f64>) -> Option<FundamentalsView> {
    if facts.is_empty() {
        return None;
    }

    // (metric, period) -> value, for table-cell lookup.
    let mut lookup: HashMap<(String, String), f64> = HashMap::new();
    // (metric, fiscal_year) -> value, for the annual ratio inputs.
    let mut annual_vals: HashMap<(&str, i64), f64> = HashMap::new();
    for f in facts {
        lookup.insert((f.metric.clone(), f.period.clone()), f.value);
        if f.fiscal_qtr.is_none() {
            annual_vals.insert((f.metric.as_str(), f.fiscal_year), f.value);
        }
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

    // Ratios run off the most recent full fiscal year.
    let latest_fy = annual.last().map(|(y, _)| *y);
    let ratios = match latest_fy {
        Some(fy) => {
            let av = |m: &str, y: i64| annual_vals.get(&(m, y)).copied();
            let inputs = compute::RatioInputs {
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
            };
            compute::compute_ratios(&inputs)
        }
        None => Vec::new(),
    };

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
    holdings: Vec<HoldingView>,
}

/// Assemble the fund view from a stored profile row and its holdings.
fn build_fund(profile: FundProfileRow, holdings: Vec<HoldingRow>) -> FundView {
    // Asset mix: the stored JSON `[[label, percent], ...]`. The percentages
    // already sum to ~100, so each is its own segment width directly.
    let asset_mix = profile
        .asset_mix
        .as_deref()
        .and_then(|j| serde_json::from_str::<Vec<(String, f64)>>(j).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|(label, pct)| AssetSlice {
            pct: format!("{pct:.1}%"),
            width: pct.clamp(0.0, 100.0),
            label,
        })
        .collect();

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
        holdings,
    }
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

    let fundamentals = if is_stock {
        let facts: Vec<FundFact> = sqlx::query_as(
            "SELECT metric, period, fiscal_year, fiscal_qtr, value \
             FROM fundamentals WHERE ticker = ?",
        )
        .bind(&ticker)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();
        build_fundamentals(&facts, price)
    } else {
        None
    };

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

    // The ETF fund profile, when the SEC sweep has reached this symbol.
    let fund = if is_etf {
        let profile = sqlx::query_as::<_, FundProfileRow>(
            "SELECT kind, net_assets, holdings_count, report_date, asset_mix \
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
                Some(build_fund(profile, holdings))
            }
            None => None,
        }
    } else {
        None
    };

    let extra = minijinja::context! {
        title => ticker,
        symbol => symbol,
        stats => stats,
        quote => quote,
        fundamentals => fundamentals,
        fund => fund,
        filings => filings,
    };
    render(&state, "pages/symbol.html", &format!("/s/{ticker}"), extra)
}

#[derive(Deserialize)]
struct HistoryQuery {
    range: Option<String>,
}

/// One OHLCV point shaped for lightweight-charts (a `YYYY-MM-DD` `time`).
#[derive(sqlx::FromRow, Serialize)]
struct Candle {
    time: String,
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

/// The symbol chart payload (Phase 8): the candles for the selected range
/// plus the indicator overlays, each already trimmed to the visible window.
#[derive(Serialize)]
struct HistoryResponse {
    candles: Vec<Candle>,
    sma50: Vec<LinePoint>,
    sma200: Vec<LinePoint>,
    ema21: Vec<LinePoint>,
    rsi14: Vec<LinePoint>,
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

    let candles: Vec<Candle> = match &fetch_cutoff {
        Some(cutoff) => {
            sqlx::query_as(
                "SELECT d AS time, open, high, low, close, volume FROM daily_prices \
                 WHERE ticker = ? AND d >= ? ORDER BY d ASC",
            )
            .bind(&ticker)
            .bind(cutoff)
            .fetch_all(&state.pool)
            .await
        }
        None => {
            sqlx::query_as(
                "SELECT d AS time, open, high, low, close, volume FROM daily_prices \
                 WHERE ticker = ? ORDER BY d ASC",
            )
            .bind(&ticker)
            .fetch_all(&state.pool)
            .await
        }
    }
    .unwrap_or_default();

    // First bar inside the visible window; everything before it is lookback,
    // fetched only so the indicators are correct from the very first shown bar.
    let start = match &display_cutoff {
        Some(c) => candles
            .iter()
            .position(|b| b.time.as_str() >= c.as_str())
            .unwrap_or(candles.len()),
        None => 0,
    };

    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    // Zip a raw indicator series to its bar dates, dropping warm-up `None`s
    // and the lookback bars, leaving only points inside the visible window.
    let line = |series: Vec<Option<f64>>| -> Vec<LinePoint> {
        series
            .into_iter()
            .enumerate()
            .skip(start)
            .filter_map(|(i, v)| {
                v.map(|value| LinePoint {
                    time: candles[i].time.clone(),
                    value,
                })
            })
            .collect()
    };

    let resp = HistoryResponse {
        sma50: line(compute::sma(&closes, 50)),
        sma200: line(compute::sma(&closes, 200)),
        ema21: line(compute::ema(&closes, 21)),
        rsi14: line(compute::rsi(&closes, 14)),
        candles: candles.into_iter().skip(start).collect(),
    };

    Json(resp).into_response()
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
/// inserted, the quote that same lookup returned is stored, and the history
/// job is brought forward so the deep daily backfill lands within a tick. The
/// Yahoo request goes through the shared endpoint guard, like every other
/// outbound call (see PLAN.md's anti-spam policy).
async fn add_symbol(State(state): State<AppState>, Json(body): Json<AddSymbolBody>) -> Response {
    let Some(ticker) = valid_ticker(&body.ticker) else {
        return add_err(
            StatusCode::BAD_REQUEST,
            "That does not look like a ticker symbol.",
        );
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
        return Json(AddSymbolResponse {
            ok: true,
            ticker: Some(ticker),
            name: Some(name),
            kind: Some(kind),
            added: false,
            error: None,
        })
        .into_response();
    }

    // One guarded Yahoo lookup: validates the symbol and describes it.
    let yahoo = YahooProvider::new(http::build_client(&state.config));
    let guard = EndpointGuard::with_budget(state.pool.clone(), "yahoo", scheduler::YAHOO_BUDGET);
    match guard.acquire().await {
        Ok(Permit::Granted) => {}
        Ok(Permit::Denied(_)) => {
            return add_err(
                StatusCode::SERVICE_UNAVAILABLE,
                "The market data source is busy right now. Try again in a few minutes.",
            );
        }
        Err(e) => {
            tracing::error!("add_symbol guard for {ticker}: {e:#}");
            return add_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Something went wrong. Try again shortly.",
            );
        }
    }

    let (info, data) = match yahoo.lookup(&ticker).await {
        Err(e) => {
            let _ = guard.record_failure(&e).await;
            tracing::warn!("add_symbol lookup {ticker}: {e:#}");
            return add_err(
                StatusCode::BAD_GATEWAY,
                "Could not reach the market data source. Try again shortly.",
            );
        }
        Ok(outcome) => {
            // The endpoint answered — even an "unknown symbol" is a healthy
            // reply, so the guard records a success either way.
            let _ = guard.record_success().await;
            match outcome {
                SymbolLookup::Found { info, data } => (info, data),
                SymbolLookup::Unknown => {
                    return add_err(
                        StatusCode::NOT_FOUND,
                        format!("No symbol called {ticker} was found."),
                    );
                }
                SymbolLookup::Unsupported(raw_kind) => {
                    let what = raw_kind.to_lowercase();
                    return add_err(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        format!(
                            "{ticker} is a {what}. Only stocks, ETFs, indexes, and futures can be added right now."
                        ),
                    );
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
        tracing::error!("add_symbol insert {ticker}: {e:#}");
        return add_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not save the symbol. Try again shortly.",
        );
    }

    // Store the quote (and bars) the lookup already paid for, so the symbol's
    // page shows a live price at once. Best-effort: a hiccup here does not
    // undo a successful add.
    if let Err(e) = scheduler::store_quote(&state.pool, &ticker, &data.quote).await {
        tracing::warn!("add_symbol store_quote {ticker}: {e:#}");
    }
    if !data.bars.is_empty() {
        if let Err(e) = scheduler::store_intraday(&state.pool, &ticker, &data.bars).await {
            tracing::warn!("add_symbol store_intraday {ticker}: {e:#}");
        }
    }
    // Bring the history job forward so the deep daily backfill runs on the
    // next scheduler tick rather than waiting out the ~6h interval.
    if let Err(e) = scheduler::schedule_next(&state.pool, "history", now).await {
        tracing::warn!("add_symbol schedule history for {ticker}: {e:#}");
    }

    tracing::info!("add_symbol: added {ticker} ({}, {})", info.name, info.kind);
    Json(AddSymbolResponse {
        ok: true,
        ticker: Some(ticker),
        name: Some(info.name),
        kind: Some(info.kind),
        added: true,
        error: None,
    })
    .into_response()
}
