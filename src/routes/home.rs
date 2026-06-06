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

use crate::compute;
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
    /// The session's first-bar open — the chart's reference line and the % base.
    base: f64,
    /// The latest value (the card's headline figure).
    last: f64,
    /// % change from `base` (the day move shown beside the value).
    change_pct: f64,
    /// True when the day move is not negative — drives the green/red line colour.
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

/// What `/api/dashboard` returns and what `home` seeds the page with.
#[derive(Serialize)]
struct DashboardData {
    session: String,
    reads: MarketReads,
    /// The fixed market-overview charts.
    series: Vec<Series>,
    /// The session's watchlist, drawn with the same per-instrument chart
    /// treatment as the overview (Schwab day, shading, % vs prev close).
    watchlist: Vec<Series>,
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

/// `GET /api/dashboard` — the per-instrument overview series + the market reads,
/// polled by the page (~every minute) so the charts and reads stay live without a
/// reload. Each series carries its own actual values (points or dollars) plus its
/// last value and % change from the open; the page draws one chart per series.
async fn dashboard_api(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let market_session = market::session_at(chrono::Utc::now());

    // The fixed market-overview set. The S&P slot leads.
    let slots = overview();
    let mut series = Vec::with_capacity(slots.len());
    for (quote, chart, name, dollar) in slots {
        if let Some(s) = overview_series(&state, quote, chart, name, dollar).await {
            series.push(s);
        }
    }

    // The session's watchlist, drawn with the same chart treatment as the
    // overview. Each is a single-ticker series (the symbol is both quote + chart;
    // stocks/ETFs carry their own pre/post bars via Yahoo's includePrePost).
    let session = watchlist::resolve(&state.pool, &headers).await;
    let wl = watchlist::list(&state.pool, &session.sid).await;
    let mut watchlist = Vec::with_capacity(wl.len());
    for t in &wl {
        if let Some(s) = watchlist_series(&state, t).await {
            watchlist.push(s);
        }
    }

    let data = DashboardData {
        session: market_session.as_str().to_string(),
        reads: market_reads(&state).await,
        series,
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
    for t in overview_tickers() {
        tickers.push(t.to_string());
    }
    for b in [VIX, VOLUME_PROXY] {
        tickers.push(b.to_string());
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

/// A human session label for the dashboard's market-hours banner.
fn session_label(s: market::Session) -> &'static str {
    match s {
        market::Session::Regular => "Regular session",
        market::Session::Pre => "Pre-market",
        market::Session::Post => "After hours",
        market::Session::Closed => "Market closed",
    }
}

/// Build one slot's overview chart over a single Schwab trading day (extended
/// open through extended close, framed by `start_t`/`end_t`). The **line** is the
/// `chart` ticker's 15-minute bars (the future, so pre-market + regular +
/// after-hours all show), while the headline **value + %** come from the `quote`
/// ticker (the cash index, the universally-quoted number). `None` when the chart
/// ticker has no intraday bars (e.g. never polled, or a holiday).
async fn overview_series(
    state: &AppState,
    quote_ticker: &str,
    chart_ticker: &str,
    name: &str,
    dollar: bool,
) -> Option<Series> {
    // After Friday's close the chart frames the whole trading week; the rest of
    // the time it anchors to the Schwab day the chart ticker's most recent bar
    // falls in, so it shows just that one day and never the previous one.
    let now_ms = chrono::Utc::now().timestamp_millis();
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

    // Headline value + % come from the QUOTE ticker, exactly as every market site
    // shows it: the latest price (`last_price` = Yahoo's regularMarketPrice)
    // against the previous close (`prev_close` = chartPreviousClose). For the
    // cash indexes this is the universally-quoted number — live during the
    // session, frozen at the closing change after the close. `prev_close` is also
    // the chart's dashed reference line. Both fall back to stored daily closes.
    let quote: Option<(Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT \
           COALESCE(s.last_price, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1)), \
           COALESCE(s.prev_close, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1 OFFSET 1)) \
         FROM symbols s WHERE s.ticker = ?",
    )
    .bind(quote_ticker)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten();
    let (last_price, prev_close) = quote.unwrap_or((None, None));

    // Last value: the quote price, else the last drawn bar's close. Over the
    // weekend the quote price is Yahoo's frozen Friday close — exactly the week's
    // end value we want.
    let last = last_price.or_else(|| rows.last().map(|r| r.2))?;
    // Reference: the session's first-bar open (also the % base's last-resort).
    let open = rows.first().map(|r| if r.1 > 0.0 { r.1 } else { r.2 });
    // The % base. Single-day mode: the previous close (the universally-quoted
    // day move). Week mode: the prior Friday's close — the last daily close
    // strictly before this week's Monday — so the move is the full-week change.
    let base = if let Some(monday) = week_monday {
        let prior_close: Option<f64> = sqlx::query_scalar(
            "SELECT close FROM daily_prices WHERE ticker = ? AND d < ? ORDER BY d DESC LIMIT 1",
        )
        .bind(quote_ticker)
        .bind(monday.to_string())
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();
        prior_close.filter(|p| *p > 0.0).or(open)?
    } else {
        prev_close.filter(|p| *p > 0.0).or(open)?
    };
    if base <= 0.0 {
        return None;
    }
    let points: Vec<SeriesPoint> = rows
        .iter()
        .map(|(ts, _open, close)| SeriesPoint { t: ts / 1000, v: *close })
        .collect();
    let change_pct = (last / base - 1.0) * 100.0;
    Some(Series {
        ticker: quote_ticker.to_string(),
        name: name.to_string(),
        unit: if dollar { "$" } else { "pts" },
        base,
        last,
        change_pct,
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
