//! `GET /` — the markets dashboard.
//!
//! An opinionated, no-customization read of the market: a row of sparkline
//! cards for the major US indexes and the headline commodities, and the day's
//! biggest movers among the curated large-cap universe. There is deliberately
//! no per-user layout — the app decides what matters (see PLAN.md Phase 11).
//! The full, browsable universe lives on `/search`.

use std::cmp::Ordering;
use std::collections::HashMap;

use axum::{extract::State, response::Response, routing::get, Router};
use serde::Serialize;

use crate::compute::{self, Sparkline};
use crate::models::SymbolCardRow;
use crate::render::render;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(home))
}

/// The dashboard's curated sparkline row: the major US indexes followed by the
/// headline commodities (WTI crude, gold, natural gas). Hardcoded on purpose —
/// the home page is a fixed, opinionated view, not a user-built watchlist.
const DASHBOARD: &[&str] = &[
    "^SPX", "^DJI", "^NDX", "^NDQ", "^RUT", "^VIX", // indexes
    "CL=F", "GC=F", "NG=F", // crude oil, gold, natural gas
];

/// How many gainers and how many losers each movers panel lists.
const MOVERS_LIMIT: usize = 8;

/// A symbol's latest session counts as the bars within this window of its most
/// recent intraday bar. The regular-plus-extended session spans ~16h, while
/// the prior session's bars sit a full ~24h earlier, so 23h cleanly isolates
/// just the latest day.
const SESSION_WINDOW_MS: i64 = 23 * 3600 * 1000;

/// One sparkline card on the dashboard's top row.
#[derive(Serialize)]
struct SparkCard {
    ticker: String,
    name: String,
    price: Option<f64>,
    change_pct: Option<f64>,
    /// Sparkline geometry, `None` until the symbol has intraday bars.
    spark: Option<Sparkline>,
    /// Colour hook: true when the day's change is not negative (or unknown).
    up: bool,
}

/// One row in a movers panel.
#[derive(Serialize, Clone)]
struct Mover {
    ticker: String,
    name: String,
    price: f64,
    change_abs: f64,
    change_pct: f64,
    /// Width (0..100) of the row's magnitude tint, scaled to the largest
    /// absolute move shown across both panels.
    bar: f64,
}

async fn home(State(state): State<AppState>) -> Response {
    let seeded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM symbols WHERE is_seeded = 1")
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);

    // No curated universe yet: the seed has not run. Show the same guidance
    // the page carried before the redesign.
    if seeded == 0 {
        let extra = minijinja::context! { title => "Markets", empty => true };
        return render(&state, "pages/home.html", "/", extra);
    }

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM symbols")
        .fetch_one(&state.pool)
        .await
        .unwrap_or(seeded);

    let spark_cards = dashboard_cards(&state).await;
    let (gainers, losers) = movers(&state).await;

    let extra = minijinja::context! {
        title => "Markets",
        empty => false,
        spark_cards => spark_cards,
        gainers => gainers,
        losers => losers,
        total => total,
    };
    render(&state, "pages/home.html", "/", extra)
}

/// The curated dashboard symbols, in `DASHBOARD` order, each with a current
/// price, the day's change, and a sparkline of the latest session's bars.
async fn dashboard_cards(state: &AppState) -> Vec<SparkCard> {
    // One query for the price rows. The `IN` list is built from the DASHBOARD
    // const — never user input — so the placeholder count is fixed and safe.
    let placeholders = vec!["?"; DASHBOARD.len()].join(",");
    let sql = format!(
        "SELECT s.ticker, s.name, s.kind, \
           COALESCE(s.last_price, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1)), \
           COALESCE(s.prev_close, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1 OFFSET 1)) \
         FROM symbols s WHERE s.ticker IN ({placeholders})"
    );
    let mut q = sqlx::query_as::<_, SymbolCardRow>(&sql);
    for t in DASHBOARD {
        q = q.bind(*t);
    }
    let rows: Vec<SymbolCardRow> = q.fetch_all(&state.pool).await.unwrap_or_default();
    let mut by_ticker: HashMap<String, SymbolCardRow> =
        rows.into_iter().map(|r| (r.0.clone(), r)).collect();

    let mut cards = Vec::with_capacity(DASHBOARD.len());
    for &t in DASHBOARD {
        // Skip a dashboard symbol the universe somehow does not hold.
        let Some((ticker, name, _kind, last, prev)) = by_ticker.remove(t) else {
            continue;
        };

        // The latest session's intraday closes, oldest first. The window keys
        // off this symbol's own most recent bar (see SESSION_WINDOW_MS).
        let closes: Vec<f64> = sqlx::query_scalar(
            "SELECT close FROM intraday_bars \
             WHERE ticker = ? \
               AND ts >= (SELECT MAX(ts) FROM intraday_bars WHERE ticker = ?) - ? \
             ORDER BY ts",
        )
        .bind(&ticker)
        .bind(&ticker)
        .bind(SESSION_WINDOW_MS)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

        let change_pct = match (last, prev) {
            (Some(l), Some(p)) => Some(compute::change(l, p).pct),
            _ => None,
        };
        cards.push(SparkCard {
            ticker,
            name,
            price: last,
            change_pct,
            spark: compute::sparkline(&closes, prev),
            up: change_pct.map_or(true, |p| p >= 0.0),
        });
    }
    cards
}

/// The day's biggest gainers and losers among the curated large-cap stocks.
///
/// Restricted to `is_seeded` stocks on purpose: the movers are meant to be
/// names worth noticing (an AAPL down 5%), not a small user-added symbol's
/// noise. ETFs, indexes, and futures are excluded — only single stocks.
async fn movers(state: &AppState) -> (Vec<Mover>, Vec<Mover>) {
    let rows: Vec<(String, String, Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT s.ticker, s.name, \
           COALESCE(s.last_price, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1)), \
           COALESCE(s.prev_close, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1 OFFSET 1)) \
         FROM symbols s WHERE s.is_seeded = 1 AND s.kind = 'stock'",
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    // Keep only stocks with a computable change.
    let mut all: Vec<Mover> = rows
        .into_iter()
        .filter_map(|(ticker, name, last, prev)| {
            let (last, prev) = (last?, prev?);
            if prev == 0.0 {
                return None;
            }
            let c = compute::change(last, prev);
            Some(Mover {
                ticker,
                name,
                price: last,
                change_abs: c.abs,
                change_pct: c.pct,
                bar: 0.0,
            })
        })
        .collect();
    if all.is_empty() {
        return (Vec::new(), Vec::new());
    }

    // Sorted by the day's % change: gainers from the top, losers from the
    // bottom (most negative first).
    all.sort_by(|a, b| {
        b.change_pct
            .partial_cmp(&a.change_pct)
            .unwrap_or(Ordering::Equal)
    });
    let mut gainers: Vec<Mover> = all.iter().take(MOVERS_LIMIT).cloned().collect();
    let mut losers: Vec<Mover> = all.iter().rev().take(MOVERS_LIMIT).cloned().collect();

    // Scale every magnitude tint to the largest absolute move on display, so a
    // +1% and a -1% row read the same width.
    let max_abs = gainers
        .iter()
        .chain(losers.iter())
        .map(|m| m.change_pct.abs())
        .fold(0.0_f64, f64::max);
    for m in gainers.iter_mut().chain(losers.iter_mut()) {
        m.bar = if max_abs > 0.0 {
            (m.change_pct.abs() / max_abs * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
    }
    (gainers, losers)
}
