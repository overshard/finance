//! Live market-summary snapshot — the dashboard hero verdict + market breadth,
//! computed straight from stored prices.
//!
//! This is the one source of truth for the dashboard's plain-language read of
//! the day. It is used two ways:
//!  - the home route renders it at page load (via the verdict vocabulary below,
//!    [`market_verdict`] / [`vix_tone`]); and
//!  - the scheduler recomputes it as index quotes tick intraday and pushes it
//!    over the stream as a [`crate::stream::StreamEvent::Summary`], so the
//!    verdict sentence, the headline figures, and the breadth counts stay live
//!    instead of going stale beside the live-ticking index chips (Phase 7).
//!
//! For a Python reader: think of [`market_summary`] as a small read-model query
//! — two cheap aggregate reads (breadth across the curated stocks, plus the
//! lead index and ^VIX) folded into one serializable struct the browser patches
//! into the page.

use serde::Serialize;
use sqlx::SqlitePool;

use crate::compute;
use crate::market::Session;

/// The broad-market index the hero verdict reads: the cash S&P during the
/// regular session, its E-mini future outside it (so the verdict stays live
/// overnight, matching how `home::dashboard_cards` resolves the lead card).
const BROAD_CASH: &str = "^SPX";
const BROAD_FUTURE: &str = "ES=F";
/// The volatility gauge folded into the risk tone. No tradable future on Yahoo,
/// so it is always the cash level.
const VIX: &str = "^VIX";

/// The dashboard hero verdict + market breadth in one payload, shaped for the
/// browser to patch in place. Mirrors the fields the home template renders at
/// load so a pushed update and the server render agree.
#[derive(Debug, Clone, Serialize, Default)]
pub struct MarketSummary {
    /// The punchy lead, e.g. "Higher, but narrow."
    pub verdict: String,
    /// The supporting clause, e.g. "Markets higher with narrow participation."
    pub detail: String,
    /// The broad-market day change (the lead index move); drives the "S&P +x%"
    /// stat. `None` until that symbol prices.
    pub broad_pct: Option<f64>,
    /// The VIX read folded into one phrase, e.g. "calm at 13.2". `None` until
    /// ^VIX prices.
    pub vix_label: Option<String>,
    /// Advancers vs decliners across the curated large-cap stocks.
    pub breadth: BreadthCounts,
}

/// Market breadth across the curated large-cap stocks: how many are advancing
/// vs declining today, the share green, and the proportion-bar segment widths.
/// Field names match the home template's `breadth.*` so the page render and a
/// pushed update patch the same nodes.
#[derive(Debug, Clone, Serialize, Default)]
pub struct BreadthCounts {
    pub advancers: usize,
    pub decliners: usize,
    pub unchanged: usize,
    /// Stocks with a computable day change (advancers + decliners + unchanged).
    pub total: usize,
    /// Advancers as a percent of `total`, rounded; `None` when `total` is 0.
    pub pct_green: Option<u8>,
    /// Proportion-bar segment widths (percent of `total`): green, flat, red.
    pub up_w: f64,
    pub flat_w: f64,
    pub down_w: f64,
}

/// The dashboard symbols whose live quotes move the hero verdict: the broad
/// index (cash or its future, by session) and ^VIX. The scheduler uses this to
/// decide whether an intraday sweep touched anything worth re-pushing a summary
/// for (the curated stocks behind breadth only move at the daily close).
pub fn pulse_tickers(session: Session) -> [&'static str; 2] {
    let broad = if matches!(session, Session::Regular) {
        BROAD_CASH
    } else {
        BROAD_FUTURE
    };
    [broad, VIX]
}

/// Build the full market summary from stored prices. Two cheap reads: a breadth
/// scan over the curated stocks and a two-symbol pull for the lead index + VIX.
/// Both resolve the live last price, falling back to the latest stored daily
/// close, exactly as the home route's card/breadth queries do — so a pushed
/// update reconciles with what the page rendered.
pub async fn market_summary(pool: &SqlitePool, session: Session) -> MarketSummary {
    let breadth = breadth_snapshot(pool).await;

    let [broad_ticker, _] = pulse_tickers(session);
    let broad_pct = day_change_pct(pool, broad_ticker).await;
    let vix_level = latest_price(pool, VIX).await;
    let vix_pct = day_change_pct(pool, VIX).await;

    let (verdict, detail) = market_verdict(broad_pct, breadth.pct_green, vix_level, vix_pct);
    let vix_label = vix_level.map(|v| format!("{} at {:.1}", vix_tone(v), v));

    MarketSummary {
        verdict,
        detail,
        broad_pct,
        vix_label,
        breadth,
    }
}

/// Market breadth across the curated large-cap stocks. One read of each stock's
/// resolved last/prev price (live quote, else latest daily close), folded the
/// same way [`crate::routes::home`]'s in-memory breadth does: a stock without a
/// computable change (no price, or a non-positive prior close) is left out of
/// every count so a missing quote never reads as "flat".
async fn breadth_snapshot(pool: &SqlitePool) -> BreadthCounts {
    let rows: Vec<(Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT \
           COALESCE(s.last_price, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1)), \
           COALESCE(s.prev_close, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1 OFFSET 1)) \
         FROM symbols s WHERE s.is_seeded = 1 AND s.kind = 'stock'",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let mut b = BreadthCounts::default();
    for (last, prev) in rows {
        let (Some(last), Some(prev)) = (last, prev) else {
            continue;
        };
        if prev <= 0.0 {
            continue;
        }
        b.total += 1;
        if last > prev {
            b.advancers += 1;
        } else if last < prev {
            b.decliners += 1;
        } else {
            b.unchanged += 1;
        }
    }
    if b.total > 0 {
        let total = b.total as f64;
        b.pct_green = Some((b.advancers as f64 / total * 100.0).round() as u8);
        b.up_w = b.advancers as f64 / total * 100.0;
        b.down_w = b.decliners as f64 / total * 100.0;
        b.flat_w = (100.0 - b.up_w - b.down_w).max(0.0);
    }
    b
}

/// One symbol's resolved last price (live quote, else latest daily close).
async fn latest_price(pool: &SqlitePool, ticker: &str) -> Option<f64> {
    sqlx::query_scalar(
        "SELECT COALESCE(s.last_price, \
           (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1)) \
         FROM symbols s WHERE s.ticker = ?",
    )
    .bind(ticker)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .flatten()
}

/// One symbol's day change percent from its resolved last vs prior close.
async fn day_change_pct(pool: &SqlitePool, ticker: &str) -> Option<f64> {
    let row: Option<(Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT \
           COALESCE(s.last_price, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1)), \
           COALESCE(s.prev_close, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1 OFFSET 1)) \
         FROM symbols s WHERE s.ticker = ?",
    )
    .bind(ticker)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    match row {
        Some((Some(last), Some(prev))) => Some(compute::change(last, prev).pct),
        _ => None,
    }
}

/// A VIX level read into one plain word. The bands suit the ^VIX cash gauge:
/// sub-14 is a placid tape, the teens are normal, the low-20s start to show
/// stress, and 28+ is outright fear.
pub fn vix_tone(level: f64) -> &'static str {
    match level {
        v if v < 14.0 => "calm",
        v if v < 20.0 => "steady",
        v if v < 28.0 => "elevated",
        _ => "stressed",
    }
}

/// Blend the broad-market move, breadth, and the VIX read into the hero's
/// two-line verdict. `broad_pct` is the lead index card's day change (the cash
/// S&P during the regular session, its future outside it); `green_pct` the
/// share of curated stocks green; `vix_level` / `vix_pct` the volatility gauge
/// and its move. Returns `(lead, detail)`. A descriptive read of the tape, not
/// a forecast — direction comes from the broad move, falling back to breadth
/// when the index is flat; width and risk tone colour the wording.
pub fn market_verdict(
    broad_pct: Option<f64>,
    green_pct: Option<u8>,
    vix_level: Option<f64>,
    vix_pct: Option<f64>,
) -> (String, String) {
    // Direction: the broad index move is the headline truth, so the verdict's
    // direction tracks its sign — never the opposite of the "S&P +x%" figure
    // shown beside it. A near-flat index (|move| < 0.05%) reads as mixed even if
    // breadth skews, since that is genuinely a directionless tape. Breadth only
    // sets direction when there is no index price at all (e.g. it never quoted).
    // 1 up / -1 down / 0 flat.
    let dir = match broad_pct {
        Some(p) if p > 0.05 => 1,
        Some(p) if p < -0.05 => -1,
        Some(_) => 0,
        None => match green_pct {
            Some(g) if g >= 55 => 1,
            Some(g) if g <= 45 => -1,
            _ => 0,
        },
    };
    // Breadth width: 2 broad / 1 split / 0 narrow.
    let width = match green_pct {
        Some(g) if g >= 60 => 2,
        Some(g) if g <= 40 => 0,
        _ => 1,
    };
    let vix_rising = vix_pct.is_some_and(|p| p > 4.0);
    let vix_elevated = vix_level.is_some_and(|v| v >= 20.0);

    let lead = match (dir, width) {
        (1, 2) if !vix_elevated => "Risk-on, and broad.",
        (1, 2) => "Higher across the board.",
        (1, 0) => "Higher, but narrow.",
        (1, _) => "Modestly higher.",
        (-1, _) if vix_rising || vix_elevated => "Risk-off.",
        (-1, 0) => "Broadly lower.",
        (-1, _) => "Softer today.",
        _ => "Quiet, mixed tape.",
    };
    let move_word = match dir {
        1 => "higher",
        -1 => "lower",
        _ => "little changed",
    };
    let part_word = match width {
        2 => "wide participation",
        0 => "narrow participation",
        _ => "mixed participation",
    };
    (
        lead.to_string(),
        format!("Markets {move_word} with {part_word}."),
    )
}
