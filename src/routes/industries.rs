//! Industry trends (Phase 15).
//!
//! Treats sectors and industries as first-class. Each stock is classified by
//! Yahoo's `quoteSummary.assetProfile`, populated by the `asset_profile`
//! scheduler job; this module aggregates the classifications into:
//!
//! - `GET /industries` — sector index page.
//! - `GET /industries/{sector-slug}` — sector detail page.
//! - `GET /industries/{sector-slug}/{industry-slug}` — industry detail page.
//! - `GET /api/industries/{sector-slug}/{industry-slug?}/history` —
//!   equal-weight composite price series (capped to 5 years) plus the `^SPX`
//!   benchmark anchored to the same first date.
//!
//! Aggregation is in-memory, on every render — same pattern as the home
//! Strongest & Weakest panels (Phase 20). The data the page leans on
//! (current prices and trailing daily closes for the curated stocks) is
//! already loaded by other panels in a similar shape, so the cost is small.
//! ETFs / indexes / futures are excluded; they carry no asset profile.

use std::cmp::Ordering;
use std::collections::HashMap;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Serialize;
use serde_json::json;

use crate::compute::{self, IndustryReturns};
use crate::render::render;
use crate::AppState;

/// Trading-day cap on the composite price chart. Five years is long enough
/// to span a full market cycle, short enough that the equal-weight scan over
/// dozens of members is cheap; matches the deep-history cap in
/// `routes::backtest`'s `HIST_LOOKBACK_DAYS`.
const COMPOSITE_LOOKBACK_DAYS: i64 = 5 * 365 + 2;

/// One row per stock with classification + the figures the page needs.
/// Loaded once and reused for the sector index, the detail pages, and (with
/// a follow-up SQL pass for daily closes) the chart API.
#[derive(Clone)]
struct StockClass {
    ticker: String,
    name: String,
    sector: String,
    industry: String,
    last: Option<f64>,
    prev: Option<f64>,
}

/// Load every stock with both a sector and an industry populated, alongside
/// its live last-price and prev-close (for the day move).
async fn load_classified(state: &AppState) -> Vec<StockClass> {
    type Row = (String, String, String, String, Option<f64>, Option<f64>);
    sqlx::query_as::<_, Row>(
        "SELECT s.ticker, s.name, s.sector, s.industry, \
           COALESCE(s.last_price, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1)), \
           COALESCE(s.prev_close, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1 OFFSET 1)) \
         FROM symbols s \
         WHERE s.kind = 'stock' \
           AND s.sector IS NOT NULL AND s.sector <> '' \
           AND s.industry IS NOT NULL AND s.industry <> ''",
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(ticker, name, sector, industry, last, prev)| StockClass {
        ticker,
        name,
        sector,
        industry,
        last,
        prev,
    })
    .collect()
}

/// Group `rows` by `key(row)`, returning each group's name and members in a
/// deterministic order (groups alphabetical, members alphabetical by ticker).
fn group_by<K, F>(rows: &[StockClass], key: F) -> Vec<(String, Vec<StockClass>)>
where
    K: Into<String>,
    F: Fn(&StockClass) -> K,
{
    let mut map: HashMap<String, Vec<StockClass>> = HashMap::new();
    for r in rows {
        map.entry(key(r).into()).or_default().push(r.clone());
    }
    let mut groups: Vec<(String, Vec<StockClass>)> = map.into_iter().collect();
    for (_, members) in groups.iter_mut() {
        members.sort_by(|a, b| a.ticker.cmp(&b.ticker));
    }
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    groups
}

/// Equal-weight day-move composite across `members`. `None` when no member
/// has both `last` and `prev`.
fn day_composite(members: &[StockClass]) -> Option<f64> {
    let pcts: Vec<f64> = members
        .iter()
        .filter_map(|m| {
            let (last, prev) = (m.last?, m.prev?);
            if prev <= 0.0 {
                return None;
            }
            Some((last - prev) / prev * 100.0)
        })
        .collect();
    if pcts.is_empty() {
        return None;
    }
    Some(pcts.iter().sum::<f64>() / pcts.len() as f64)
}

/// Lowercase, hyphenated slug for a sector / industry name. ASCII-only —
/// our universe is US-listed so this is enough. Whitespace and `&` collapse;
/// anything not `[a-z0-9-]` is dropped. Empty input maps to `"other"`.
pub fn slug(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = false;
    for c in s.chars() {
        let mapped = if c.is_ascii_alphanumeric() {
            prev_dash = false;
            c.to_ascii_lowercase()
        } else if c.is_whitespace() || c == '-' || c == '&' || c == '/' || c == ',' {
            if prev_dash {
                continue;
            }
            prev_dash = true;
            '-'
        } else {
            // Drop punctuation entirely so "Industrials, Inc." doesn't gain
            // an extra dash from the comma + the space after it.
            continue;
        };
        out.push(mapped);
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "other".to_string()
    } else {
        trimmed
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/industries", get(index_page))
        .route("/industries/{sector}", get(sector_page))
        .route("/industries/{sector}/{industry}", get(industry_page))
        .route(
            "/api/industries/{sector}/history",
            get(sector_history_api),
        )
        .route(
            "/api/industries/{sector}/{industry}/history",
            get(industry_history_api),
        )
}

/// One row in the `/industries` sector index.
#[derive(Serialize)]
struct SectorIndexRow {
    name: String,
    slug: String,
    members: usize,
    industries: usize,
    /// Composite returns (day / month / quarter / year). Each is built from
    /// equal-weight averages over the sector's members.
    returns: IndustryReturns,
}

async fn index_page(State(state): State<AppState>) -> Response {
    let rows = load_classified(&state).await;
    if rows.is_empty() {
        let extra = minijinja::context! {
            title => "Industries",
            empty => true,
        };
        return render(&state, "pages/industries_index.html", "/industries", extra);
    }

    let tickers: Vec<&str> = rows.iter().map(|r| r.ticker.as_str()).collect();
    let closes_map = load_closes_map(&state, &tickers).await;

    let groups = group_by(&rows, |r| r.sector.clone());
    let mut sectors: Vec<SectorIndexRow> = groups
        .into_iter()
        .map(|(name, members)| {
            let industries = members
                .iter()
                .map(|m| m.industry.as_str())
                .collect::<std::collections::HashSet<_>>()
                .len();
            let member_closes: Vec<&[f64]> = members
                .iter()
                .filter_map(|m| closes_map.get(&m.ticker).map(Vec::as_slice))
                .collect();
            let returns = compute::industry_returns(&member_closes);
            SectorIndexRow {
                slug: slug(&name),
                name,
                members: members.len(),
                industries,
                returns,
            }
        })
        .collect();
    // Default order: best day move first, but keep a stable secondary sort by
    // name so a no-data day still reads alphabetically.
    sectors.sort_by(|a, b| {
        let (ar, br) = (a.returns.d1.unwrap_or(f64::NEG_INFINITY), b.returns.d1.unwrap_or(f64::NEG_INFINITY));
        br.partial_cmp(&ar).unwrap_or(Ordering::Equal).then(a.name.cmp(&b.name))
    });

    let total = sectors.iter().map(|s| s.members).sum::<usize>();
    let total_industries: usize = sectors.iter().map(|s| s.industries).sum();
    let extra = minijinja::context! {
        title => "Industries",
        empty => false,
        sectors => sectors,
        total => total,
        total_industries => total_industries,
    };
    render(&state, "pages/industries_index.html", "/industries", extra)
}

/// Bulk-load the trailing-N-day daily closes for every ticker in `tickers`,
/// grouped by ticker, oldest first. One scan over `daily_prices` joined to
/// the symbol list.
async fn load_closes_map(state: &AppState, tickers: &[&str]) -> HashMap<String, Vec<f64>> {
    if tickers.is_empty() {
        return HashMap::new();
    }
    let cutoff = (chrono::Utc::now().date_naive()
        - chrono::Duration::days(COMPOSITE_LOOKBACK_DAYS))
    .to_string();
    let placeholders = vec!["?"; tickers.len()].join(",");
    let sql = format!(
        "SELECT p.ticker, p.close FROM daily_prices p \
         WHERE p.ticker IN ({placeholders}) AND p.d >= ? ORDER BY p.ticker, p.d"
    );
    let mut q = sqlx::query_as::<_, (String, f64)>(&sql);
    for t in tickers {
        q = q.bind(*t);
    }
    q = q.bind(&cutoff);
    let rows = q.fetch_all(&state.pool).await.unwrap_or_default();
    let mut out: HashMap<String, Vec<f64>> = HashMap::new();
    for (t, c) in rows {
        out.entry(t).or_default().push(c);
    }
    out
}

/// Bulk-load every ticker's dated daily closes (oldest first, paired with the
/// trading-date string). Used by the chart API where we need each bar's date
/// to align the composite.
async fn load_dated_closes_map(
    state: &AppState,
    tickers: &[&str],
) -> HashMap<String, Vec<(String, f64)>> {
    if tickers.is_empty() {
        return HashMap::new();
    }
    let cutoff = (chrono::Utc::now().date_naive()
        - chrono::Duration::days(COMPOSITE_LOOKBACK_DAYS))
    .to_string();
    let placeholders = vec!["?"; tickers.len()].join(",");
    let sql = format!(
        "SELECT p.ticker, p.d, p.close FROM daily_prices p \
         WHERE p.ticker IN ({placeholders}) AND p.d >= ? ORDER BY p.ticker, p.d"
    );
    let mut q = sqlx::query_as::<_, (String, String, f64)>(&sql);
    for t in tickers {
        q = q.bind(*t);
    }
    q = q.bind(&cutoff);
    let rows = q.fetch_all(&state.pool).await.unwrap_or_default();
    let mut out: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for (t, d, c) in rows {
        out.entry(t).or_default().push((d, c));
    }
    out
}

/// One member row on a sector / industry detail page.
#[derive(Serialize, Clone)]
struct MemberRow {
    ticker: String,
    name: String,
    industry: String,
    price: Option<f64>,
    change_pct: Option<f64>,
    /// Magnitude tint (0..100), scaled to the largest day move shown.
    bar: f64,
}

/// Build a list of member rows, scaling each row's tint to the largest
/// absolute day-move shown.
fn member_rows(members: &[StockClass]) -> Vec<MemberRow> {
    let mut rows: Vec<MemberRow> = members
        .iter()
        .map(|m| {
            let change_pct = match (m.last, m.prev) {
                (Some(l), Some(p)) if p > 0.0 => Some((l - p) / p * 100.0),
                _ => None,
            };
            MemberRow {
                ticker: m.ticker.clone(),
                name: m.name.clone(),
                industry: m.industry.clone(),
                price: m.last,
                change_pct,
                bar: 0.0,
            }
        })
        .collect();
    let max_abs = rows
        .iter()
        .filter_map(|r| r.change_pct.map(f64::abs))
        .fold(0.0_f64, f64::max);
    for r in rows.iter_mut() {
        if let (Some(p), true) = (r.change_pct, max_abs > 0.0) {
            r.bar = (p.abs() / max_abs * 100.0).clamp(0.0, 100.0);
        }
    }
    // Best-first by day move; missing values sink to the bottom.
    rows.sort_by(|a, b| {
        let (ax, bx) = (
            a.change_pct.unwrap_or(f64::NEG_INFINITY),
            b.change_pct.unwrap_or(f64::NEG_INFINITY),
        );
        bx.partial_cmp(&ax).unwrap_or(Ordering::Equal)
    });
    rows
}

/// Roll the per-industry breakdown out for a sector page.
#[derive(Serialize)]
struct IndustryBreakdownRow {
    name: String,
    slug: String,
    members: usize,
    day_pct: Option<f64>,
}

async fn sector_page(
    State(state): State<AppState>,
    Path(sector_slug): Path<String>,
) -> Response {
    let rows = load_classified(&state).await;
    let members: Vec<StockClass> = rows
        .into_iter()
        .filter(|r| slug(&r.sector) == sector_slug)
        .collect();
    if members.is_empty() {
        return not_found(&state, &format!("Sector '{sector_slug}' not found"));
    }
    let sector_name = members[0].sector.clone();

    let tickers: Vec<&str> = members.iter().map(|m| m.ticker.as_str()).collect();
    let closes_map = load_closes_map(&state, &tickers).await;
    let member_closes: Vec<&[f64]> = members
        .iter()
        .filter_map(|m| closes_map.get(&m.ticker).map(Vec::as_slice))
        .collect();
    let returns = compute::industry_returns(&member_closes);

    // Per-industry breakdown inside this sector.
    let by_industry = group_by(&members, |r| r.industry.clone());
    let mut breakdown: Vec<IndustryBreakdownRow> = by_industry
        .into_iter()
        .map(|(name, group)| IndustryBreakdownRow {
            slug: slug(&name),
            name,
            members: group.len(),
            day_pct: day_composite(&group),
        })
        .collect();
    breakdown.sort_by(|a, b| {
        let (ar, br) = (a.day_pct.unwrap_or(f64::NEG_INFINITY), b.day_pct.unwrap_or(f64::NEG_INFINITY));
        br.partial_cmp(&ar).unwrap_or(Ordering::Equal).then(a.name.cmp(&b.name))
    });

    // Seasonality computed on the composite — average across members of each
    // member's per-month average daily return. A member with too short a
    // history contributes whichever months it covers.
    let dated_map = load_dated_closes_map(&state, &tickers).await;
    let seasonality_months = sector_seasonality(&members, &dated_map);
    let seasonality_max = seasonality_months
        .iter()
        .filter_map(|v| v.map(f64::abs))
        .fold(0.0_f64, f64::max);

    let member_view = member_rows(&members);
    let extra = minijinja::context! {
        title => sector_name.clone(),
        scope => "sector",
        sector_name => sector_name,
        sector_slug => sector_slug,
        industry_name => "",
        industry_slug => "",
        returns => returns,
        breakdown => breakdown,
        seasonality => seasonality_months,
        seasonality_max => seasonality_max,
        members => member_view,
    };
    render(&state, "pages/industries_detail.html", "/industries", extra)
}

async fn industry_page(
    State(state): State<AppState>,
    Path((sector_slug, industry_slug)): Path<(String, String)>,
) -> Response {
    let rows = load_classified(&state).await;
    let members: Vec<StockClass> = rows
        .into_iter()
        .filter(|r| slug(&r.sector) == sector_slug && slug(&r.industry) == industry_slug)
        .collect();
    if members.is_empty() {
        return not_found(
            &state,
            &format!("Industry '{sector_slug}/{industry_slug}' not found"),
        );
    }
    let sector_name = members[0].sector.clone();
    let industry_name = members[0].industry.clone();

    let tickers: Vec<&str> = members.iter().map(|m| m.ticker.as_str()).collect();
    let closes_map = load_closes_map(&state, &tickers).await;
    let member_closes: Vec<&[f64]> = members
        .iter()
        .filter_map(|m| closes_map.get(&m.ticker).map(Vec::as_slice))
        .collect();
    let returns = compute::industry_returns(&member_closes);

    let dated_map = load_dated_closes_map(&state, &tickers).await;
    let seasonality_months = sector_seasonality(&members, &dated_map);
    let seasonality_max = seasonality_months
        .iter()
        .filter_map(|v| v.map(f64::abs))
        .fold(0.0_f64, f64::max);

    let member_view = member_rows(&members);
    let extra = minijinja::context! {
        title => format!("{industry_name} · {sector_name}"),
        scope => "industry",
        sector_name => sector_name,
        sector_slug => sector_slug,
        industry_name => industry_name,
        industry_slug => industry_slug,
        returns => returns,
        seasonality => seasonality_months,
        seasonality_max => seasonality_max,
        members => member_view,
        breakdown => Vec::<IndustryBreakdownRow>::new(),
    };
    render(&state, "pages/industries_detail.html", "/industries", extra)
}

/// Average each member's per-month seasonality into one 12-cell composite. A
/// member that can't produce a reading for a given month simply does not vote
/// in that month; an empty month maps to `None`.
fn sector_seasonality(
    members: &[StockClass],
    dated_map: &HashMap<String, Vec<(String, f64)>>,
) -> Vec<Option<f64>> {
    let mut sums = [0.0_f64; 12];
    let mut counts = [0u32; 12];
    for m in members {
        let Some(rows) = dated_map.get(&m.ticker) else {
            continue;
        };
        let dates: Vec<&str> = rows.iter().map(|(d, _)| d.as_str()).collect();
        let closes: Vec<f64> = rows.iter().map(|(_, c)| *c).collect();
        let Some(s) = compute::seasonality(&closes, &dates) else {
            continue;
        };
        for (i, v) in s.months.iter().enumerate() {
            if let Some(v) = v {
                sums[i] += *v;
                counts[i] += 1;
            }
        }
    }
    (0..12)
        .map(|i| {
            if counts[i] > 0 {
                Some(sums[i] / counts[i] as f64)
            } else {
                None
            }
        })
        .collect()
}

// ──────────────────────────── chart API ────────────────────────────────────

async fn sector_history_api(
    State(state): State<AppState>,
    Path(sector_slug): Path<String>,
) -> Response {
    composite_history(state, sector_slug, None).await
}

async fn industry_history_api(
    State(state): State<AppState>,
    Path((sector_slug, industry_slug)): Path<(String, String)>,
) -> Response {
    composite_history(state, sector_slug, Some(industry_slug)).await
}

/// Build the equal-weight composite daily series for a sector (or industry,
/// when `industry_slug` is `Some`) plus the `^SPX` benchmark scaled to the
/// composite's first date. JSON shape:
///
/// ```json
/// { "composite": [{"d": "YYYY-MM-DD", "v": 100.0}, ...],
///   "benchmark": [{"d": "YYYY-MM-DD", "v": 100.0}, ...] }
/// ```
async fn composite_history(
    state: AppState,
    sector_slug: String,
    industry_slug: Option<String>,
) -> Response {
    let rows = load_classified(&state).await;
    let members: Vec<StockClass> = rows
        .into_iter()
        .filter(|r| {
            slug(&r.sector) == sector_slug
                && industry_slug
                    .as_ref()
                    .map_or(true, |is| slug(&r.industry) == *is)
        })
        .collect();
    if members.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "no members"})),
        )
            .into_response();
    }

    let tickers: Vec<&str> = members.iter().map(|m| m.ticker.as_str()).collect();
    let dated = load_dated_closes_map(&state, &tickers).await;

    // Build a per-date equal-weight indexed series: for each trading date the
    // composite is the average across members of (member_close / member_base),
    // where each member's base is its first close inside the lookback window.
    // Members whose history starts later simply do not contribute until their
    // first bar; the composite continues with the available members. This
    // mirrors how an equal-weight index handles a new constituent.
    let mut bases: HashMap<&str, f64> = HashMap::new();
    for m in &members {
        if let Some(rows) = dated.get(&m.ticker) {
            if let Some((_, first_close)) = rows.first() {
                if *first_close > 0.0 {
                    bases.insert(m.ticker.as_str(), *first_close);
                }
            }
        }
    }
    // Union of every trading date seen in the lookback window.
    let mut date_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for rows in dated.values() {
        for (d, _) in rows {
            date_set.insert(d.clone());
        }
    }

    // Per-member, per-date close lookup.
    let mut member_dates: HashMap<&str, HashMap<&str, f64>> = HashMap::new();
    for m in &members {
        let Some(rows) = dated.get(&m.ticker) else {
            continue;
        };
        let mut by_date: HashMap<&str, f64> = HashMap::new();
        for (d, c) in rows.iter() {
            by_date.insert(d.as_str(), *c);
        }
        member_dates.insert(m.ticker.as_str(), by_date);
    }

    #[derive(Serialize)]
    struct Point {
        d: String,
        v: f64,
    }
    let mut composite: Vec<Point> = Vec::with_capacity(date_set.len());
    for d in &date_set {
        let mut sum = 0.0;
        let mut n = 0u32;
        for m in &members {
            let Some(by_date) = member_dates.get(m.ticker.as_str()) else {
                continue;
            };
            let Some(close) = by_date.get(d.as_str()) else {
                continue;
            };
            let Some(base) = bases.get(m.ticker.as_str()) else {
                continue;
            };
            sum += close / base * 100.0;
            n += 1;
        }
        if n > 0 {
            composite.push(Point {
                d: d.clone(),
                v: sum / n as f64,
            });
        }
    }

    // Benchmark: ^SPX scaled to 100 at the composite's first date.
    let benchmark = load_benchmark_series(&state, composite.first().map(|p| p.d.as_str())).await;

    Json(json!({
        "composite": composite,
        "benchmark": benchmark,
    }))
    .into_response()
}

/// Pull `^SPX` daily closes from `anchor_date` forward, scaled so the first
/// bar at or after the anchor reads `100`. Empty when the anchor is missing
/// or the index has no bars there.
async fn load_benchmark_series(
    state: &AppState,
    anchor_date: Option<&str>,
) -> Vec<serde_json::Value> {
    let Some(anchor) = anchor_date else {
        return Vec::new();
    };
    let rows: Vec<(String, f64)> = sqlx::query_as(
        "SELECT d, close FROM daily_prices WHERE ticker = '^SPX' AND d >= ? ORDER BY d",
    )
    .bind(anchor)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    let Some(base) = rows.first().map(|(_, c)| *c) else {
        return Vec::new();
    };
    if base <= 0.0 {
        return Vec::new();
    }
    rows.into_iter()
        .map(|(d, c)| json!({"d": d, "v": c / base * 100.0}))
        .collect()
}

fn not_found(state: &AppState, msg: &str) -> Response {
    let extra = minijinja::context! {
        title => "Industries",
        message => msg,
    };
    let mut response = render(state, "pages/not_found.html", "/industries", extra);
    *response.status_mut() = StatusCode::NOT_FOUND;
    response
}
