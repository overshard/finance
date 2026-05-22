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
    // (metric, fiscal_year) -> fiscal_qtr (None = full year) -> value.
    let mut by: HashMap<(&str, i64), HashMap<Option<i64>, f64>> = HashMap::new();
    for f in facts {
        by.entry((f.metric.as_str(), f.fiscal_year))
            .or_default()
            .insert(f.fiscal_qtr, f.value);
    }
    let mut derived = Vec::new();
    for ((metric, year), vals) in by {
        // A genuine Q4 row (rare, but XBRL does carry a few) always wins.
        if !Q4_DERIVABLE.contains(&metric) || vals.contains_key(&Some(4)) {
            continue;
        }
        let (Some(&fy), Some(&q1), Some(&q2), Some(&q3)) = (
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
            value: fy - q1 - q2 - q3,
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
        let facts: Vec<models::FundFact> = sqlx::query_as(
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

    // The overall strong / fair / weak standing (Phase 20): the ratios above
    // rolled up, with the daily-close trajectory folded into its score. Shown
    // as a single badge over the ratio cards. `bars` is newest-first, so it is
    // reversed into an oldest-first close series.
    let standing = fundamentals.as_ref().and_then(|f| {
        let closes: Vec<f64> = bars.iter().rev().map(|b| b.4).collect();
        compute::standing(&f.ratios, &closes)
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

    // The leadership roster + change feed (Phase 14): stocks only, like the
    // fundamentals above.
    let leadership = if is_stock {
        Some(build_leadership(&state.pool, &ticker, symbol.leadership_synced_at.is_some()).await)
    } else {
        None
    };

    let extra = minijinja::context! {
        title => ticker,
        symbol => symbol,
        stats => stats,
        quote => quote,
        fundamentals => fundamentals,
        standing => standing,
        fund => fund,
        leadership => leadership,
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
/// inserted, the quote that same lookup returned is stored, and the symbol's
/// full backfill (deep daily history and all SEC data) is pulled synchronously
/// before the response, so its page is complete the moment the add returns
/// (PLAN.md Phase 21; see `scheduler::backfill_symbol`). Every outbound call
/// goes through the shared endpoint guard (see PLAN.md's anti-spam policy).
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
    // Pull the full backfill — deep daily history and all SEC data — before
    // responding, so the new symbol's page is complete the moment the add
    // returns rather than filling in over later scheduler cycles. Best-effort
    // and guard-routed; see `scheduler::backfill_symbol`.
    scheduler::backfill_symbol(&state.pool, &state.config, &ticker, &info.kind).await;

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
