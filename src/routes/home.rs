//! `GET /` — the markets dashboard (Phase C), and `GET /api/dashboard` behind it.
//!
//! A TradingView-style read of the day: a normalized %-vs-S&P-500 day graph over
//! the session's market reads (S&P, volume, VIX, the 50/200-day trend) and the
//! browser's personal, editable watchlist. The watchlist is session-scoped (a
//! `fin_sid` cookie; see `crate::watchlist`), seeded with starters on a first
//! visit. Data is demand-driven: opening this page is what makes the server poll
//! the watchlist + baseline symbols (via the stream interest registry); nothing
//! is polled when nobody is here.

use std::collections::HashMap;

use axum::{
    extract::State,
    http::{header, HeaderMap},
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Serialize;

use crate::compute::{self, Sparkline};
use crate::market;
use crate::render::render_to_string;
use crate::{watchlist, AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(home))
        .route("/api/dashboard", get(dashboard_api))
        .route("/api/dashboard/refresh", get(dashboard_refresh))
}

/// The S&P 500 cash index — the SMA-trend read and the day graph's baseline.
const BASELINE: &str = "^SPX";
/// The volatility gauge behind the VIX read.
const VIX: &str = "^VIX";
/// A liquid S&P 500 ETF used as the "market volume" proxy: cash indexes carry no
/// real share volume on Yahoo, so the dashboard reads volume off SPY. Polled
/// while the dashboard is open (it carries a `data-ticker`) so it stays fresh.
const VOLUME_PROXY: &str = "SPY";

/// One slot in the fixed market-overview graph (Phase E). During the **regular**
/// session the `cash` ticker is drawn (the live cash index); outside it (pre /
/// after-hours / closed) the `off` ticker is drawn instead — the E-mini future,
/// which trades nearly 24h, so the overview keeps moving overnight and shows
/// where the market is heading. Instruments that already trade ~24h (gold, crude,
/// BTC) use the same ticker in both states.
struct OverviewSlot {
    cash: &'static str,
    off: &'static str,
    name: &'static str,
    /// The S&P slot is drawn as the chart's ink baseline line.
    baseline: bool,
}

/// The market overview: a fixed, non-editable read of "how is the whole market
/// doing", separate from the personal watchlist (which is cards only and no
/// longer on this graph). VIX is deliberately absent — it swings ~10x the indexes
/// and would squash a normalized %-from-open overlay; it stays a read instead.
const OVERVIEW: &[OverviewSlot] = &[
    OverviewSlot { cash: "^SPX", off: "ES=F", name: "S&P 500", baseline: true },
    OverviewSlot { cash: "^DJI", off: "YM=F", name: "Dow", baseline: false },
    OverviewSlot { cash: "^NDX", off: "NQ=F", name: "Nasdaq 100", baseline: false },
    OverviewSlot { cash: "^RUT", off: "RTY=F", name: "Russell 2000", baseline: false },
    OverviewSlot { cash: "GC=F", off: "GC=F", name: "Gold", baseline: false },
    OverviewSlot { cash: "CL=F", off: "CL=F", name: "Crude Oil", baseline: false },
    OverviewSlot { cash: "BTC-USD", off: "BTC-USD", name: "Bitcoin", baseline: false },
];

/// The overview tickers + display names for `session`: cash indexes during the
/// regular session, the E-mini futures (and the ~24h instruments) otherwise.
fn overview_for(session: market::Session) -> Vec<(&'static str, &'static str, bool)> {
    let regular = session == market::Session::Regular;
    OVERVIEW
        .iter()
        .map(|s| (if regular { s.cash } else { s.off }, s.name, s.baseline))
        .collect()
}

/// A symbol's latest session = the intraday bars within this window of its most
/// recent bar (regular+extended spans ~16h; the prior session sits ~24h back).
const SESSION_WINDOW_MS: i64 = 23 * 3600 * 1000;

/// Calendar days of daily closes to pull for the 50/200-day SMA trend read.
const SMA_LOOKBACK_DAYS: i64 = 320;

/// Volume vs its recent average: this many trading days form the baseline.
const VOLUME_AVG_DAYS: i64 = 65;

/// One card on the dashboard: a symbol's price, day move, and intraday spark.
#[derive(Serialize, Clone)]
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

/// The dashboard's headline market reads, server-rendered and then refreshed by
/// the `/api/dashboard` poll. Every field is best-effort: a missing one renders
/// as a dash and is simply skipped by the live patcher.
#[derive(Serialize, Default)]
struct MarketReads {
    /// VIX level, its tone bucket (calm/steady/elevated/stressed), and a label.
    vix_level: Option<f64>,
    vix_tone: Option<String>,
    /// Market volume proxy (SPY): today's volume, the ratio to its recent
    /// average, a heavy/normal/light label, and when it was last quoted.
    volume: Option<i64>,
    volume_ratio: Option<f64>,
    volume_label: Option<String>,
    volume_asof: Option<i64>,
    /// The S&P's stance vs its 50- and 200-day averages, plus a tone.
    sma_read: Option<String>,
    sma_tone: Option<String>,
    /// Freshest quote time (epoch-ms) across the baseline reads, for the
    /// "prices as of …" caption.
    asof: Option<i64>,
}

/// One overlaid line on the day graph: a symbol's intraday move as % from the
/// session's open, on a shared time axis.
#[derive(Serialize)]
struct Series {
    ticker: String,
    name: String,
    /// True for the S&P baseline (drawn distinctly).
    baseline: bool,
    points: Vec<SeriesPoint>,
}

#[derive(Serialize)]
struct SeriesPoint {
    /// UNIX seconds (lightweight-charts wants seconds, not ms).
    t: i64,
    /// Percent change from the session's first bar.
    v: f64,
}

/// What `/api/dashboard` returns and what `home` seeds the page with.
#[derive(Serialize)]
struct DashboardData {
    session: String,
    reads: MarketReads,
    series: Vec<Series>,
}

async fn home(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let session = watchlist::resolve(&state.pool, &headers).await;
    let tickers = watchlist::list(&state.pool, &session.sid).await;

    let cards = spark_cards_for(
        &state,
        &tickers.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .await;
    let reads = market_reads(&state).await;
    let market_session = market::session_at(chrono::Utc::now());

    // The overview tickers for this session, rendered as hidden `data-ticker`
    // nodes so the live stream registers them with the interest registry and the
    // demand-driven intraday poll keeps their bars fresh while the page is open.
    let overview_tickers: Vec<&str> = overview_for(market_session)
        .into_iter()
        .map(|(t, _, _)| t)
        .collect();

    let extra = minijinja::context! {
        title => "Markets",
        cards => cards,
        empty => tickers.is_empty(),
        reads => reads,
        vix => VIX,
        volume_proxy => VOLUME_PROXY,
        overview_tickers => overview_tickers,
        session => market_session.as_str(),
        session_label => session_label(market_session),
    };

    match render_to_string(&state, "pages/home.html", "/", extra) {
        Ok(html) => {
            let mut resp = Html(html).into_response();
            if let Some(c) = session.set_cookie {
                if let Ok(v) = header::HeaderValue::from_str(&c) {
                    resp.headers_mut().insert(header::SET_COOKIE, v);
                }
            }
            resp
        }
        Err(resp) => resp,
    }
}

/// `GET /api/dashboard` — the day graph series + the market reads, polled by the
/// page (~every minute) so the chart and reads stay live without a reload. The
/// series are normalized %-from-open so the watchlist and the S&P baseline share
/// one axis (the TradingView/Google "compare" shape).
async fn dashboard_api(State(state): State<AppState>) -> Response {
    // No session needed: the overview is fixed, not per-browser. (The watchlist
    // cards live-tick over the base stream; they are not on this graph.)
    let market_session = market::session_at(chrono::Utc::now());

    // The market-overview set for this session: cash indexes during the regular
    // session, the E-mini futures otherwise. The baseline (S&P) leads.
    let overview = overview_for(market_session);
    let mut series = Vec::with_capacity(overview.len());
    for (ticker, name, baseline) in overview {
        if let Some(s) = pct_series(&state, ticker, name, baseline).await {
            series.push(s);
        }
    }

    let data = DashboardData {
        session: market_session.as_str().to_string(),
        reads: market_reads(&state).await,
        series,
    };

    Json(data).into_response()
}

/// `GET /api/dashboard/refresh` — the dashboard's on-open refresh. Pulls fresh
/// quotes for the watchlist + the baseline reads (^SPX/^VIX/SPY) once, so opening
/// the page always shows current data rather than whatever was last stored. The
/// scheduler's staleness gate skips anything quoted in the last few minutes, so a
/// reload doesn't re-hit Yahoo. Published quotes live-tick the open cards.
async fn dashboard_refresh(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let session = watchlist::resolve(&state.pool, &headers).await;
    // The watchlist cards, the session's overview symbols, and the VIX / volume
    // reads — everything the open dashboard shows gets a fresh quote.
    let mut tickers = watchlist::list(&state.pool, &session.sid).await;
    for (t, _, _) in overview_for(market::session_at(chrono::Utc::now())) {
        tickers.push(t.to_string());
    }
    for b in [VIX, VOLUME_PROXY] {
        tickers.push(b.to_string());
    }
    let refreshed =
        crate::scheduler::refresh_quotes(&state.pool, &state.config, &state.hub, &tickers).await;

    let mut resp = Json(serde_json::json!({ "refreshed": refreshed })).into_response();
    if let Some(c) = session.set_cookie {
        if let Ok(v) = header::HeaderValue::from_str(&c) {
            resp.headers_mut().insert(header::SET_COOKIE, v);
        }
    }
    resp
}

/// A human session label for the dashboard's market-hours banner.
fn session_label(s: market::Session) -> &'static str {
    match s {
        market::Session::Regular => "Regular session",
        market::Session::Pre => "Pre-market",
        market::Session::Post => "After hours",
        market::Session::Closed => "Market closed",
    }
}

/// Build one symbol's normalized intraday series: its latest session's 15-minute
/// bars as % change from the session's first bar's open. `None` when the symbol
/// has no intraday bars (e.g. never polled, or a holiday).
async fn pct_series(state: &AppState, ticker: &str, name: &str, baseline: bool) -> Option<Series> {
    let rows: Vec<(i64, f64, f64)> = sqlx::query_as(
        "SELECT ts, open, close FROM intraday_bars \
         WHERE ticker = ? \
           AND ts >= (SELECT MAX(ts) FROM intraday_bars WHERE ticker = ?) - ? \
         ORDER BY ts",
    )
    .bind(ticker)
    .bind(ticker)
    .bind(SESSION_WINDOW_MS)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    // Base off the first bar's open; fall back to its close if open is zero.
    let base = rows.first().map(|r| if r.1 > 0.0 { r.1 } else { r.2 })?;
    if base <= 0.0 {
        return None;
    }
    let points: Vec<SeriesPoint> = rows
        .iter()
        .map(|(ts, _open, close)| SeriesPoint {
            t: ts / 1000,
            v: (close / base - 1.0) * 100.0,
        })
        .collect();
    if points.is_empty() {
        return None;
    }
    Some(Series {
        ticker: ticker.to_string(),
        name: name.to_string(),
        baseline,
        points,
    })
}

/// Build the dashboard's headline reads from stored data (no network): the S&P
/// level/move, the VIX, the SPY-proxied market volume, and the S&P's 50/200-day
/// stance.
async fn market_reads(state: &AppState) -> MarketReads {
    let mut r = MarketReads::default();

    // VIX level + tone.
    if let Some((Some(level), _)) = last_and_prev(state, VIX).await {
        r.vix_level = Some(level);
        r.vix_tone = Some(compute::vix_tone(level).to_string());
    }

    // Market volume proxy (SPY): today's volume vs its recent average.
    let vol: Option<(Option<i64>, Option<i64>)> =
        sqlx::query_as("SELECT volume, fetched_at FROM quotes WHERE ticker = ?")
            .bind(VOLUME_PROXY)
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten();
    if let Some((Some(today), asof)) = vol {
        if today > 0 {
            r.volume = Some(today);
            r.volume_asof = asof;
            let avg: Option<f64> = sqlx::query_scalar(
                "SELECT AVG(volume) FROM (SELECT volume FROM daily_prices \
                 WHERE ticker = ? AND volume > 0 ORDER BY d DESC LIMIT ?)",
            )
            .bind(VOLUME_PROXY)
            .bind(VOLUME_AVG_DAYS)
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten();
            if let Some(avg) = avg.filter(|a| *a > 0.0) {
                let ratio = today as f64 / avg;
                r.volume_ratio = Some(ratio);
                r.volume_label = Some(
                    if ratio >= 1.15 {
                        "Heavy"
                    } else if ratio <= 0.85 {
                        "Light"
                    } else {
                        "Normal"
                    }
                    .to_string(),
                );
            }
        }
    }

    // S&P stance vs its 50- and 200-day moving averages.
    let closes_desc: Vec<f64> = sqlx::query_scalar(
        "SELECT close FROM daily_prices WHERE ticker = ? ORDER BY d DESC LIMIT ?",
    )
    .bind(BASELINE)
    .bind(SMA_LOOKBACK_DAYS)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    if let Some(&last) = closes_desc.first() {
        let closes: Vec<f64> = closes_desc.iter().rev().copied().collect();
        let sma50 = compute::sma(&closes, 50).last().copied().flatten();
        let sma200 = compute::sma(&closes, 200).last().copied().flatten();
        if let (Some(s50), Some(s200)) = (sma50, sma200) {
            let (read, tone) = match (last >= s50, last >= s200) {
                (true, true) => ("Above its 50- and 200-day average", "up"),
                (false, false) => ("Below its 50- and 200-day average", "down"),
                (true, false) => ("Above its 50-day, below its 200-day", "warn"),
                (false, true) => ("Below its 50-day, above its 200-day", "warn"),
            };
            r.sma_read = Some(read.to_string());
            r.sma_tone = Some(tone.to_string());
        }
    }

    // Freshest quote across the baseline reads, for the "prices as of" caption.
    r.asof = sqlx::query_scalar(
        "SELECT MAX(fetched_at) FROM quotes WHERE ticker IN (?, ?, ?)",
    )
    .bind(BASELINE)
    .bind(VIX)
    .bind(VOLUME_PROXY)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .flatten();

    r
}

/// The latest price and the prior close for one symbol: the live last price
/// (else the latest stored daily close) and the close before it.
async fn last_and_prev(state: &AppState, ticker: &str) -> Option<(Option<f64>, Option<f64>)> {
    sqlx::query_as(
        "SELECT \
           COALESCE(s.last_price, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1)), \
           COALESCE(s.prev_close, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1 OFFSET 1)) \
         FROM symbols s WHERE s.ticker = ?",
    )
    .bind(ticker)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
}

/// Build a sparkline card per ticker, in order: current price, the day's change,
/// and a sparkline of the latest session's bars. A ticker the universe does not
/// hold is skipped.
async fn spark_cards_for(state: &AppState, tickers: &[&str]) -> Vec<SparkCard> {
    if tickers.is_empty() {
        return Vec::new();
    }
    // One query for the price rows; the `IN` placeholder count matches `tickers`.
    type SparkRow = (String, String, Option<f64>, Option<f64>);
    let placeholders = vec!["?"; tickers.len()].join(",");
    let sql = format!(
        "SELECT s.ticker, s.name, \
           COALESCE(s.last_price, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1)), \
           COALESCE(s.prev_close, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1 OFFSET 1)) \
         FROM symbols s WHERE s.ticker IN ({placeholders})"
    );
    let mut q = sqlx::query_as::<_, SparkRow>(&sql);
    for t in tickers {
        q = q.bind(*t);
    }
    let rows: Vec<SparkRow> = q.fetch_all(&state.pool).await.unwrap_or_default();
    let mut by_ticker: HashMap<String, SparkRow> =
        rows.into_iter().map(|r| (r.0.clone(), r)).collect();

    let mut cards = Vec::with_capacity(tickers.len());
    for &t in tickers {
        let Some((ticker, name, last, prev)) = by_ticker.remove(t) else {
            continue;
        };
        // The latest session's intraday closes, oldest first.
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
