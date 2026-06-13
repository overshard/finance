//! `GET /` — the markets dashboard (Phase C), and `GET /api/dashboard` behind it.
//!
//! A TradingView-style read of the day: a normalized %-vs-S&P-500 day graph over
//! the session's market reads (S&P, volume, VIX, the 50/200-day trend) and the
//! browser's personal, editable watchlist. The watchlist is session-scoped (a
//! `fin_sid` cookie; see `crate::watchlist`), seeded with starters on a first
//! visit. The dashboard is the one exception to the demand-only model: the
//! scheduler's active home sweep (`scheduler::run_home_sweep_if_due`) keeps its
//! instruments fresh on a 15-minute cadence even with nobody on the site, so
//! the page always opens current. An open page still gets the faster treatment
//! (the stream interest registry's ~5-minute poll plus the on-open refresh).

use std::collections::HashMap;

use axum::{
    extract::State,
    http::{header, HeaderMap},
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::compute;
use crate::guard::{EndpointGuard, Permit};
use crate::market;
use crate::providers::http;
use crate::providers::yahoo::{Mover, YahooProvider};
use crate::render::render_to_string;
use crate::{db, scheduler, watchlist, AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(home))
        .route("/api/dashboard", get(dashboard_api))
        .route("/api/dashboard/refresh", get(dashboard_refresh))
        .route("/api/movers", get(movers_api))
}

/// The S&P 500 cash index — the SMA-trend read and the day graph's baseline.
const BASELINE: &str = "^SPX";
/// The volatility gauge behind the VIX read.
const VIX: &str = "^VIX";
/// A liquid S&P 500 ETF used as the "market volume" proxy: cash indexes carry no
/// real share volume on Yahoo, so the dashboard reads volume off SPY. Polled
/// while the dashboard is open (it carries a `data-ticker`) so it stays fresh.
const VOLUME_PROXY: &str = "SPY";

/// The high-yield corporate-bond ETF used as the dashboard's credit-stress proxy:
/// a falling HYG means widening high-yield spreads (risk-off), the bond market's
/// tell that a sell-off is a real credit event, not noise. Already in the seed.
const CREDIT: &str = "HYG";

/// One slot in the fixed market-overview grid. Two tickers, so each card can show
/// the full extended-hours day *and* the universally-quoted number:
/// - `chart` draws the line. For an index this is the E-mini **future**, which
///   trades ~24h, so the chart shows pre-market + regular + after-hours movement.
/// - `quote` drives the headline value + %. For an index this is the **cash
///   index** (`regularMarketPrice` vs `chartPreviousClose`) — the number every
///   market site shows, frozen at the closing change after 4pm.
///
/// Instruments that already trade ~24h (gold, crude, BTC) use one ticker for both.
struct OverviewSlot {
    quote: &'static str,
    chart: &'static str,
    name: &'static str,
    /// Priced in dollars (gold, crude, BTC) rather than index points — drives the
    /// `$`-vs-`pts` unit hint the per-instrument chart formats its values with.
    dollar: bool,
}

/// The market overview: a fixed, non-editable read of "how is the whole market
/// doing", separate from the personal watchlist. Each slot is its own chart
/// (pts for indexes, $ for gold/crude/BTC). VIX is deliberately absent — it stays
/// a headline read.
const OVERVIEW: &[OverviewSlot] = &[
    OverviewSlot { quote: "^SPX", chart: "ES=F", name: "S&P 500", dollar: false },
    OverviewSlot { quote: "^DJI", chart: "YM=F", name: "Dow", dollar: false },
    OverviewSlot { quote: "^NDX", chart: "NQ=F", name: "Nasdaq 100", dollar: false },
    OverviewSlot { quote: "GC=F", chart: "GC=F", name: "Gold", dollar: true },
    OverviewSlot { quote: "CL=F", chart: "CL=F", name: "Crude Oil", dollar: true },
    OverviewSlot { quote: "BTC-USD", chart: "BTC-USD", name: "Bitcoin", dollar: true },
];

/// The 11 SPDR Select Sector ETFs, the cheap stand-in for a 500-name S&P heatmap:
/// each is a market-cap slice of one GICS sector, so their day moves show *which*
/// part of the market is driving the index at a glance, for 11 quotes instead of
/// 500. Ordered roughly by S&P weight so the biggest movers read first.
const SECTORS: &[(&str, &str)] = &[
    ("XLK", "Technology"),
    ("XLF", "Financials"),
    ("XLC", "Communication"),
    ("XLY", "Discretionary"),
    ("XLV", "Health Care"),
    ("XLI", "Industrials"),
    ("XLP", "Staples"),
    ("XLE", "Energy"),
    ("XLU", "Utilities"),
    ("XLRE", "Real Estate"),
    ("XLB", "Materials"),
];

/// The sector ETF tickers, for the home sweep / on-open refresh quote set.
fn sector_tickers() -> Vec<&'static str> {
    SECTORS.iter().map(|(t, _)| *t).collect()
}

/// The overview slots as (quote ticker, chart ticker, display name, dollar unit).
fn overview() -> Vec<(&'static str, &'static str, &'static str, bool)> {
    OVERVIEW.iter().map(|s| (s.quote, s.chart, s.name, s.dollar)).collect()
}

/// Every ticker the overview needs polled / quoted — both the quote (cash) and
/// chart (futures) tickers, de-duplicated in slot order.
fn overview_tickers() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for s in OVERVIEW {
        for t in [s.quote, s.chart] {
            if !out.contains(&t) {
                out.push(t);
            }
        }
    }
    out
}

/// Everything the dashboard reads a quote for: the overview slots (cash +
/// futures tickers) plus the VIX and volume-proxy headline reads. The
/// scheduler's active home sweep and the on-open refresh both poll exactly
/// this set (each adding the watchlist symbols on top).
pub(crate) fn dashboard_tickers() -> Vec<&'static str> {
    let mut out = overview_tickers();
    for t in [VIX, VOLUME_PROXY, CREDIT] {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    for t in sector_tickers() {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// The overview charts frame exactly one Schwab trading day: extended-hours open
/// (7:00 AM ET) through extended-hours close (8:00 PM ET), so each chart shows
/// just that day — pre-market, the regular session, and after-hours — and never
/// bleeds into the previous day.
const SCHWAB_OPEN_MIN: u32 = 7 * 60; // 7:00 AM ET
const SCHWAB_CLOSE_MIN: u32 = 20 * 60; // 8:00 PM ET

/// Once the regular session closes on Friday (4:00 PM ET) the dashboard switches
/// from the single-day frame to the whole trading week (Mon 7 AM → Fri 8 PM ET),
/// so the weekend read shows how the full week went, not just where Friday landed.
/// It reverts to the single-day frame at Monday's extended-hours open (7:00 AM ET).
const FRIDAY_CLOSE_MIN: u32 = 16 * 60; // 4:00 PM ET

/// Epoch-ms for `min` minutes-of-day on the ET calendar `date`. Picks the earlier
/// instant on a fall-back DST repeat; both are fine for these window bounds.
fn et_ms(date: chrono::NaiveDate, min: u32) -> Option<i64> {
    use chrono::TimeZone as _;
    use chrono_tz::America::New_York;
    let naive = date.and_hms_opt(min / 60, min % 60, 0)?;
    New_York
        .from_local_datetime(&naive)
        .earliest()
        .map(|dt| dt.timestamp_millis())
}

/// The Schwab trading-day window [open, close] in epoch-ms for the ET calendar
/// day that `latest_ms` falls in (so a Friday-evening view frames Friday, a
/// weekend view still frames Friday's last session, etc.).
fn schwab_day_window(latest_ms: i64) -> Option<(i64, i64)> {
    use chrono::TimeZone as _;
    use chrono_tz::America::New_York;
    let date = New_York.timestamp_millis_opt(latest_ms).single()?.date_naive();
    Some((et_ms(date, SCHWAB_OPEN_MIN)?, et_ms(date, SCHWAB_CLOSE_MIN)?))
}

/// The full-week window when the end-of-week view is active, else `None`.
///
/// Active from Friday's regular close (4:00 PM ET) through the weekend until
/// Monday's extended-hours open (7:00 AM ET). When active it frames the trading
/// week that just ended: Monday 7:00 AM → Friday 8:00 PM ET. Returns that window
/// plus the Monday `NaiveDate` (the caller reads the prior Friday's close — the
/// last daily close strictly before Monday — as the week's % base).
fn week_window(now_ms: i64) -> Option<(i64, i64, chrono::NaiveDate)> {
    use chrono::{Datelike as _, Duration, TimeZone as _, Timelike as _, Weekday};
    use chrono_tz::America::New_York;
    let now = New_York.timestamp_millis_opt(now_ms).single()?;
    let minutes = now.hour() * 60 + now.minute();
    // How many days back the just-closed Friday sits from `now`'s ET date.
    let days_back = match now.weekday() {
        Weekday::Fri if minutes >= FRIDAY_CLOSE_MIN => 0,
        Weekday::Sat => 1,
        Weekday::Sun => 2,
        Weekday::Mon if minutes < SCHWAB_OPEN_MIN => 3,
        _ => return None,
    };
    let friday = now.date_naive() - Duration::days(days_back);
    let monday = friday - Duration::days(4);
    Some((
        et_ms(monday, SCHWAB_OPEN_MIN)?,
        et_ms(friday, SCHWAB_CLOSE_MIN)?,
        monday,
    ))
}

/// The value unit for a symbol: index points for equity indexes, dollars for
/// everything else (stocks, ETFs, crypto, dollar-priced commodity futures).
fn unit_for(kind: &str) -> &'static str {
    if kind == "index" {
        "pts"
    } else {
        "$"
    }
}

/// Calendar days of daily closes to pull for the 50/200-day SMA trend read.
const SMA_LOOKBACK_DAYS: i64 = 320;

/// Volume vs its recent average: this many trading days form the baseline.
const VOLUME_AVG_DAYS: i64 = 65;

/// One watchlist card shell, server-rendered for the initial paint, the symbol
/// link, and the remove button. The Schwab-day chart + the live value/% are then
/// drawn into it by `hero.js` from `/api/dashboard` (the same treatment as the
/// overview cards), so a watchlist card and an overview card look identical.
#[derive(Serialize, Clone)]
struct SparkCard {
    ticker: String,
    name: String,
    price: Option<f64>,
    change_pct: Option<f64>,
    /// Colour hook: true when the day's change is not negative (or unknown).
    up: bool,
    /// "$" for dollar-priced symbols (stocks/ETFs/crypto), "pts" for indexes.
    unit: &'static str,
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
    /// The S&P's drawdown from its record close (`<= 0`), the crash-response lead
    /// read, with a tone and a zone label (slight dip / pullback / correction /
    /// bear, the deeper zones flagged as the DCA add zone).
    drawdown_pct: Option<f64>,
    drawdown_tone: Option<String>,
    drawdown_label: Option<String>,
    /// Credit-stress read off the high-yield ETF (HYG) day move, with tone + label.
    credit_pct: Option<f64>,
    credit_tone: Option<String>,
    credit_label: Option<String>,
    /// Freshest quote time (epoch-ms) across the baseline reads, for the
    /// "prices as of …" caption.
    asof: Option<i64>,
}

/// One instrument's own chart in the overview grid: its latest session's actual
/// values (index points or dollars) on its own axis, plus the headline figures
/// the card shows above the chart (last value + % change from the open).
#[derive(Serialize)]
struct Series {
    ticker: String,
    name: String,
    /// "$" for dollar-priced instruments (gold, crude, BTC), "pts" otherwise.
    unit: &'static str,
    /// The % base / the chart's dashed reference line. During the regular session
    /// this is the previous close; off-hours it is the reference the headline move
    /// is measured against (the futures' prior settlement, or the prev close).
    base: f64,
    /// The latest value (the card's headline figure). Off-hours this is the live
    /// extended-hours value (the future for an index, the pre-market bar for a
    /// stock), not the frozen regular-session close.
    last: f64,
    /// % change from `base` — the headline move, session-appropriate (the cash
    /// day move during the regular session, the futures move pre-market/overnight,
    /// the pre-market move for a stock, the close move after hours).
    change_pct: f64,
    /// Week-to-date % move: the cash value vs the cash close before this ET week's
    /// Monday, so the card can show the day AND the week move at once. `None` when
    /// there is no prior-week close to anchor to.
    week_pct: Option<f64>,
    /// Which session the headline reflects, so an off-hours number is never read
    /// as the close: `None` during the regular session (the plain cash number),
    /// else "Futures" / "Pre-market" / "After hours" / "Overnight" / "At close".
    headline_label: Option<&'static str>,
    /// Epoch-ms source time of the headline quote, for the per-card freshness chip
    /// ("live" / "2m ago" / "stale"). `None` when no quote has been stored yet.
    asof: Option<i64>,
    /// True when the headline move is not negative — drives the green/red line colour.
    up: bool,
    /// UNIX seconds bounding the chart frame (extended-hours open and close).
    /// Normally a single Schwab day; in end-of-week mode the whole trading week
    /// (Mon 7 AM → Fri 8 PM ET). A partial frame plots from the left rather than
    /// stretching across the width.
    start_t: i64,
    end_t: i64,
    /// True when the frame spans the whole week (Fri 4 PM → Mon 7 AM ET), so the
    /// chart axis labels days instead of just times.
    week: bool,
    points: Vec<SeriesPoint>,
}

#[derive(Serialize)]
struct SeriesPoint {
    /// UNIX seconds (lightweight-charts wants seconds, not ms).
    t: i64,
    /// The bar's actual close value (index points or dollars).
    v: f64,
}

/// One sector tile in the "what's driving the market" heatmap: a sector ETF's
/// latest-session % move, the cell coloured green/red by it (clamped at ±3% on
/// the client). `change_pct` is `None` until the ETF has been quoted.
#[derive(Serialize)]
struct SectorTile {
    ticker: String,
    name: &'static str,
    change_pct: Option<f64>,
}

/// What `/api/dashboard` returns and what `home` seeds the page with.
#[derive(Serialize)]
struct DashboardData {
    session: String,
    reads: MarketReads,
    /// The fixed market-overview charts.
    series: Vec<Series>,
    /// The sector heatmap (11 SPDR sector ETFs), so the dashboard shows which
    /// part of the market is driving the index, not just the index level.
    sectors: Vec<SectorTile>,
    /// The session's watchlist, drawn with the same per-instrument chart
    /// treatment as the overview (Schwab day, shading, % vs prev close).
    watchlist: Vec<Series>,
}

/// Build the sector heatmap: each SPDR sector ETF's most-recent-session % move
/// (latest price vs its previous close). A cheap, fixed set of local reads.
async fn sector_tiles(state: &AppState) -> Vec<SectorTile> {
    let mut tiles = Vec::with_capacity(SECTORS.len());
    for (ticker, name) in SECTORS {
        let (last, prev, _asof) = quote_row(state, ticker).await;
        let change_pct = match (last, prev) {
            (Some(l), Some(p)) if p > 0.0 => Some((l / p - 1.0) * 100.0),
            _ => None,
        };
        tiles.push(SectorTile {
            ticker: (*ticker).to_string(),
            name,
            change_pct,
        });
    }
    tiles
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

    // The overview tickers (both cash + futures), rendered as hidden `data-ticker`
    // nodes so the live stream registers them with the interest registry and the
    // demand-driven intraday poll keeps their bars fresh while the page is open.
    let overview_tickers: Vec<&str> = overview_tickers();

    let extra = minijinja::context! {
        title => "Markets",
        cards => cards,
        empty => tickers.is_empty(),
        reads => reads,
        vix => VIX,
        volume_proxy => VOLUME_PROXY,
        credit_proxy => CREDIT,
        overview_tickers => overview_tickers,
        sector_tickers => sector_tickers(),
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

/// `GET /api/dashboard` — the per-instrument overview series + the market reads,
/// polled by the page (~every minute) so the charts and reads stay live without a
/// reload. Each series carries its own actual values (points or dollars) plus its
/// last value and % change from the open; the page draws one chart per series.
async fn dashboard_api(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let market_session = market::session_at(chrono::Utc::now());

    // The fixed market-overview set (the S&P slot leads) and the session's
    // watchlist, drawn with the same chart treatment. Each watchlist symbol is
    // a single-ticker series (the symbol is both quote + chart; stocks/ETFs
    // carry their own pre/post bars via Yahoo's includePrePost). Every series
    // is a handful of independent SQLite reads, so they are all built
    // concurrently rather than one after another — ~12+ series on a typical
    // dashboard, and the response is what gates the page's first chart paint.
    let session = watchlist::resolve(&state.pool, &headers).await;
    let wl = watchlist::list(&state.pool, &session.sid).await;
    let (series, watchlist) = tokio::join!(
        futures_util::future::join_all(
            overview()
                .into_iter()
                .map(|(quote, chart, name, dollar)| overview_series(
                    &state, quote, chart, name, dollar
                )),
        ),
        futures_util::future::join_all(wl.iter().map(|t| watchlist_series(&state, t))),
    );
    let series: Vec<Series> = series.into_iter().flatten().collect();
    let watchlist: Vec<Series> = watchlist.into_iter().flatten().collect();

    let data = DashboardData {
        session: market_session.as_str().to_string(),
        reads: market_reads(&state).await,
        series,
        sectors: sector_tiles(&state).await,
        watchlist,
    };

    Json(data).into_response()
}

/// One watchlist symbol as a chart series, identical in shape to an overview
/// slot: the symbol is its own quote + chart ticker, with the unit (points for an
/// index, dollars otherwise) read off its kind.
async fn watchlist_series(state: &AppState, ticker: &str) -> Option<Series> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT name, kind FROM symbols WHERE ticker = ?")
            .bind(ticker)
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten();
    let (name, kind) = row?;
    overview_series(state, ticker, ticker, &name, unit_for(&kind) == "$").await
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
    let wl = watchlist::list(&state.pool, &session.sid).await;
    let mut tickers = wl.clone();
    for t in dashboard_tickers() {
        tickers.push(t.to_string());
    }
    let refreshed =
        crate::scheduler::refresh_quotes(&state.pool, &state.config, &state.hub, &tickers).await;

    // In the end-of-week view the charts span Mon–Fri, but the routine poll only
    // ever stores one day of 15-minute bars at a time, so any day the dashboard
    // wasn't open is missing. Backfill the whole week (one guarded range=5d pull
    // per still-incomplete symbol) for the symbols actually drawn as charts: the
    // overview's chart tickers (the futures lines) and the watchlist. It runs
    // detached — the paced, guarded pulls take many seconds and only need to
    // happen once per weekend, so we don't hold the refresh request open for
    // them; the filled bars land on the next ~60s dashboard poll. Already-covered
    // symbols are skipped, so this is a no-op once the week is complete.
    if let Some((start_ms, end_ms, _monday)) =
        week_window(chrono::Utc::now().timestamp_millis())
    {
        let mut charted: Vec<String> = OVERVIEW.iter().map(|s| s.chart.to_string()).collect();
        charted.extend(wl.iter().cloned());
        let bg = state.clone();
        tokio::spawn(async move {
            crate::scheduler::backfill_intraday_week(
                &bg.pool,
                &bg.config,
                &charted,
                start_ms,
                end_ms,
            )
            .await;
        });
    }

    let mut resp = Json(serde_json::json!({ "refreshed": refreshed })).into_response();
    if let Some(c) = session.set_cookie {
        if let Ok(v) = header::HeaderValue::from_str(&c) {
            resp.headers_mut().insert(header::SET_COOKIE, v);
        }
    }
    resp
}

/// Movers cache: re-fetch the screener at most this often. The pull is a guarded,
/// paced 3-call job; in between, the cached blob is served. Demand-driven — only
/// fetched when the dashboard is open and the cache has aged out.
const MOVERS_TTL_MS: i64 = 8 * 60 * 1000;
const MOVERS_META_KEY: &str = "movers_json";
/// Rows per movers list.
const MOVERS_COUNT: u32 = 10;

/// The three market-movers lists for the dashboard's "what's driving it" tables.
#[derive(Serialize, Deserialize)]
struct MoversData {
    /// Epoch-ms the lists were fetched, for the freshness caption. `None` = empty.
    asof: Option<i64>,
    gainers: Vec<Mover>,
    losers: Vec<Mover>,
    actives: Vec<Mover>,
}

impl MoversData {
    fn empty() -> Self {
        MoversData { asof: None, gainers: vec![], losers: vec![], actives: vec![] }
    }
}

/// `GET /api/movers` — top gainers / losers / most active. Served from an 8-minute
/// `meta` cache; on a miss it does one guarded, paced pull of the three predefined
/// Yahoo screeners and stores the result. On a guard stop or fetch failure it
/// falls back to the (stale) cache, else an empty set, so the dashboard degrades
/// quietly and never hammers Yahoo. The page fetches this after first paint, so a
/// cold pull never blocks the dashboard.
async fn movers_api(State(state): State<AppState>) -> Response {
    let now = db::now_ms();
    let cached: Option<MoversData> = db::get_meta(&state.pool, MOVERS_META_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok());

    if let Some(c) = &cached {
        if c.asof.is_some_and(|t| now - t < MOVERS_TTL_MS) {
            return Json(c).into_response();
        }
    }

    if let Some(fresh) = fetch_movers_fresh(&state, now).await {
        return Json(fresh).into_response();
    }

    // Refresh did not land (guard stop or all calls failed): serve stale, else empty.
    match cached {
        Some(c) => Json(c).into_response(),
        None => Json(MoversData::empty()).into_response(),
    }
}

/// One guarded, paced pull of the three movers screeners, stored to the cache.
/// `None` when the guard denies the first call or every call fails (so the caller
/// can fall back to the cache); a partial result (some lists empty) still returns.
async fn fetch_movers_fresh(state: &AppState, now: i64) -> Option<MoversData> {
    let yahoo = YahooProvider::new(http::build_client(&state.config));
    let guard = EndpointGuard::with_budget(state.pool.clone(), "yahoo", scheduler::YAHOO_BUDGET);
    let mut out = MoversData::empty();
    let mut any = false;
    for (scr, slot) in [("day_gainers", 0u8), ("day_losers", 1), ("most_actives", 2)] {
        match guard.acquire().await {
            Ok(Permit::Granted) => {}
            // Denied (breaker/budget/pacing) or an acquire error: stop and let the
            // caller serve the cache rather than push against the guard.
            _ => break,
        }
        match yahoo.fetch_movers(scr, MOVERS_COUNT).await {
            Ok(rows) => {
                let _ = guard.record_success().await;
                any = true;
                match slot {
                    0 => out.gainers = rows,
                    1 => out.losers = rows,
                    _ => out.actives = rows,
                }
            }
            Err(e) => {
                let _ = guard.record_failure(&e).await;
            }
        }
    }
    if !any {
        return None;
    }
    out.asof = Some(now);
    if let Ok(json) = serde_json::to_string(&out) {
        let _ = db::set_meta(&state.pool, MOVERS_META_KEY, &json).await;
    }
    Some(out)
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

/// One symbol's latest value, prior close, and quote age: the live last price
/// (else the latest stored daily close), the close before it, and the epoch-ms
/// the quote was sourced at (for the freshness chip).
async fn quote_row(state: &AppState, ticker: &str) -> (Option<f64>, Option<f64>, Option<i64>) {
    sqlx::query_as(
        "SELECT \
           COALESCE(s.last_price, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1)), \
           COALESCE(s.prev_close, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1 OFFSET 1)), \
           s.last_quote_at \
         FROM symbols s WHERE s.ticker = ?",
    )
    .bind(ticker)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .unwrap_or((None, None, None))
}

/// The cash close strictly before the current ET week's Monday — the week-to-date
/// % base, so "this week" reads as the move since last week ended (the prior
/// Friday's close).
async fn week_base_close(state: &AppState, ticker: &str, now_ms: i64) -> Option<f64> {
    use chrono::{Datelike as _, Duration, TimeZone as _};
    use chrono_tz::America::New_York;
    let now = New_York.timestamp_millis_opt(now_ms).single()?;
    let monday = now.date_naive() - Duration::days(now.weekday().num_days_from_monday() as i64);
    sqlx::query_scalar(
        "SELECT close FROM daily_prices WHERE ticker = ? AND d < ? ORDER BY d DESC LIMIT 1",
    )
    .bind(ticker)
    .bind(monday.to_string())
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .filter(|c: &f64| *c > 0.0)
}

/// The session-appropriate headline source for a card: which value, which % base,
/// and how to label it, so a number is never read as something it isn't.
///
/// - **Regular session:** the cash value vs its previous close, unlabelled — the
///   universally-quoted day move.
/// - **Pre-market:** for an index, the E-mini **future** vs its prior settlement
///   ("Futures", what every market site shows at 7am); for a stock, its
///   pre-market bar vs the previous close ("Pre-market").
/// - **After hours:** the regular-session **close** ("At close") — what everyone
///   still quotes as the day's result after 4pm.
/// - **Overnight (closed):** for an index, the future ("Overnight"); for a stock,
///   the last close ("At close").
///
/// `cash` and `fut` are each `(last, prev_close, asof_ms)`; `fut` is `Some` only
/// for an index slot (a distinct futures chart ticker). `ext_bar` is the last
/// drawn bar's close, the live extended-hours value for a stock pre-market.
fn headline(
    session: market::Session,
    cash: (Option<f64>, Option<f64>, Option<i64>),
    fut: Option<(Option<f64>, Option<f64>, Option<i64>)>,
    ext_bar: Option<f64>,
) -> (Option<f64>, Option<f64>, Option<&'static str>, Option<i64>) {
    use market::Session::{Closed, Post, Pre, Regular};
    match session {
        Regular => (cash.0, cash.1, None, cash.2),
        Pre => match fut {
            Some(f) => (f.0, f.1, Some("Futures"), f.2),
            None => (ext_bar.or(cash.0), cash.1, Some("Pre-market"), cash.2),
        },
        Post => (cash.0, cash.1, Some("At close"), cash.2),
        Closed => match fut {
            Some(f) => (f.0, f.1, Some("Overnight"), f.2),
            None => (cash.0, cash.1, Some("At close"), cash.2),
        },
    }
}

/// Build one slot's overview chart over a single Schwab trading day (extended open
/// through extended close, framed by `start_t`/`end_t`). The **line** is the
/// `chart` ticker's 15-minute bars (the future for an index, so pre-market +
/// regular + after-hours all show). The headline **value + %** are session-aware
/// (see [`headline`]): the cash index during the regular session, the future
/// pre-market and overnight, the pre-market bar for a stock, the close after
/// hours — so the number matches what Yahoo/MarketWatch show at the moment you
/// look, instead of a frozen cash close at 7am or midnight. `week_pct` carries the
/// week-to-date move alongside. `None` when the chart ticker has no intraday bars.
async fn overview_series(
    state: &AppState,
    quote_ticker: &str,
    chart_ticker: &str,
    name: &str,
    dollar: bool,
) -> Option<Series> {
    let now = chrono::Utc::now();
    let now_ms = now.timestamp_millis();
    let session = market::session_at(now);
    // An index slot pairs a cash quote ticker with a distinct futures chart ticker;
    // gold/crude/BTC and watchlist symbols are a single ticker (no futures proxy).
    let is_index_slot = quote_ticker != chart_ticker;

    // Chart frame: the whole trading week after Friday's close, else the Schwab
    // day the chart ticker's most recent bar falls in.
    let (start_ms, end_ms, week_monday) = match week_window(now_ms) {
        Some((s, e, mon)) => (s, e, Some(mon)),
        None => {
            let latest_ms: i64 =
                sqlx::query_scalar("SELECT MAX(ts) FROM intraday_bars WHERE ticker = ?")
                    .bind(chart_ticker)
                    .fetch_optional(&state.pool)
                    .await
                    .ok()
                    .flatten()
                    .flatten()?;
            let (s, e) = schwab_day_window(latest_ms)?;
            (s, e, None)
        }
    };

    let rows: Vec<(i64, f64, f64)> = sqlx::query_as(
        "SELECT ts, open, close FROM intraday_bars \
         WHERE ticker = ? AND ts >= ? AND ts <= ? \
         ORDER BY ts",
    )
    .bind(chart_ticker)
    .bind(start_ms)
    .bind(end_ms)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    // The session's first-bar open (the % base's last resort) and the last drawn
    // bar's close (a stock's live extended-hours value pre-market).
    let open = rows.first().map(|r| if r.1 > 0.0 { r.1 } else { r.2 });
    let ext_bar = rows.last().map(|r| r.2);

    // The cash quote (regular-session number + week base) and, for an index slot,
    // the futures quote (the off-hours number). The tuples are Copy.
    let cash = quote_row(state, quote_ticker).await;
    let fut = if is_index_slot {
        Some(quote_row(state, chart_ticker).await)
    } else {
        None
    };

    // Headline value / base / label / age. In the end-of-week frame the move is
    // the whole-week change (prior Friday's close → Friday's close); otherwise it
    // is the session-aware headline.
    let (last_opt, base_opt, label, asof) = if let Some(monday) = week_monday {
        let prior: Option<f64> = sqlx::query_scalar(
            "SELECT close FROM daily_prices WHERE ticker = ? AND d < ? ORDER BY d DESC LIMIT 1",
        )
        .bind(quote_ticker)
        .bind(monday.to_string())
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();
        (cash.0.or(ext_bar), prior, Some("This week"), cash.2)
    } else {
        headline(session, cash, fut, ext_bar)
    };

    let last = last_opt.or(ext_bar)?;
    let base = base_opt.filter(|p| *p > 0.0).or(open)?;
    if base <= 0.0 {
        return None;
    }
    let change_pct = (last / base - 1.0) * 100.0;

    // Week-to-date move (cash value vs the close before this week's Monday), shown
    // alongside the day move. In the weekend frame the headline already IS the
    // week move, so reuse it.
    let week_pct = if week_monday.is_some() {
        Some(change_pct)
    } else {
        match (cash.0, week_base_close(state, quote_ticker, now_ms).await) {
            (Some(c), Some(wb)) => Some((c / wb - 1.0) * 100.0),
            _ => None,
        }
    };

    let points: Vec<SeriesPoint> = rows
        .iter()
        .map(|(ts, _open, close)| SeriesPoint { t: ts / 1000, v: *close })
        .collect();
    Some(Series {
        ticker: quote_ticker.to_string(),
        name: name.to_string(),
        unit: if dollar { "$" } else { "pts" },
        base,
        last,
        change_pct,
        week_pct,
        headline_label: label,
        asof,
        up: change_pct >= 0.0,
        start_t: start_ms / 1000,
        end_t: end_ms / 1000,
        week: week_monday.is_some(),
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

    // S&P drawdown from its record close — the crash-response lead read. The
    // record is the deepest daily history we hold (seeded ~10y, enough for the
    // recent peak); a live value above it just reads as 0% (at highs).
    if let Some((Some(last), _)) = last_and_prev(state, BASELINE).await {
        let ath: Option<f64> =
            sqlx::query_scalar("SELECT MAX(close) FROM daily_prices WHERE ticker = ?")
                .bind(BASELINE)
                .fetch_optional(&state.pool)
                .await
                .ok()
                .flatten();
        if let Some(high) = ath.filter(|a| *a > 0.0) {
            let dd = (last / high.max(last) - 1.0) * 100.0;
            let (tone, label) = compute::drawdown_read(dd);
            r.drawdown_pct = Some(dd);
            r.drawdown_tone = Some(tone.to_string());
            r.drawdown_label = Some(label.to_string());
        }
    }

    // Credit stress via the high-yield ETF's day move.
    if let Some((Some(last), Some(prev))) = last_and_prev(state, CREDIT).await {
        if prev > 0.0 {
            let pct = (last / prev - 1.0) * 100.0;
            let (tone, label) = compute::credit_read(pct);
            r.credit_pct = Some(pct);
            r.credit_tone = Some(tone.to_string());
            r.credit_label = Some(label.to_string());
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

/// Build a watchlist card shell per ticker, in order: ticker, name, current
/// price, the day's change (vs prev close), and the points/dollars unit. The
/// chart itself is drawn client-side from `/api/dashboard`. A ticker the universe
/// does not hold is skipped.
async fn spark_cards_for(state: &AppState, tickers: &[&str]) -> Vec<SparkCard> {
    if tickers.is_empty() {
        return Vec::new();
    }
    // One query for the price rows; the `IN` placeholder count matches `tickers`.
    type SparkRow = (String, String, String, Option<f64>, Option<f64>);
    let placeholders = vec!["?"; tickers.len()].join(",");
    let sql = format!(
        "SELECT s.ticker, s.name, s.kind, \
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
        let Some((ticker, name, kind, last, prev)) = by_ticker.remove(t) else {
            continue;
        };
        let change_pct = match (last, prev) {
            (Some(l), Some(p)) => Some(compute::change(l, p).pct),
            _ => None,
        };
        cards.push(SparkCard {
            ticker,
            name,
            price: last,
            change_pct,
            up: change_pct.map_or(true, |p| p >= 0.0),
            unit: unit_for(&kind),
        });
    }
    cards
}
