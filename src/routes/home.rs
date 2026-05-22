//! `GET /` — the markets dashboard.
//!
//! An opinionated, no-customization read of the market: a row of sparkline
//! cards for the major US indexes and the headline commodities, the day's
//! biggest movers, and a strongest / weakest read over the curated large-cap
//! stocks. There is deliberately no per-user layout — the app decides what
//! matters (see PLAN.md Phases 11 and 20). The full, browsable universe lives
//! on `/search`.

use std::cmp::Ordering;
use std::collections::HashMap;

use axum::{extract::State, response::Response, routing::get, Router};
use serde::Serialize;

use crate::compute::{self, Sparkline};
use crate::market;
use crate::models::{self, SymbolCardRow};
use crate::render::render;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(home))
}

/// The dashboard's index cards: each cash index paired with its index future.
/// Outside the regular cash session the future is shown in the card's place
/// (it trades nearly around the clock, while the cash index sits frozen on its
/// last close; see PLAN.md Phase 21). The Nasdaq Composite (`^NDQ`) and the
/// volatility index (`^VIX`) have no clean tradable future, so they always
/// show the cash index. Hardcoded on purpose: the home page is a fixed,
/// opinionated view, not a user-built watchlist.
const INDEXES: &[(&str, Option<&str>)] = &[
    ("^SPX", Some("ES=F")),
    ("^DJI", Some("YM=F")),
    ("^NDX", Some("NQ=F")),
    ("^RUT", Some("RTY=F")),
    ("^NDQ", None),
    ("^VIX", None),
];

/// The dashboard's commodity cards: WTI crude, gold, natural gas. Shown as the
/// futures themselves, since there is no cash instrument to swap to.
const COMMODITIES: &[&str] = &["CL=F", "GC=F", "NG=F"];

/// How many gainers and how many losers each movers panel lists.
const MOVERS_LIMIT: usize = 8;

/// How many stocks each of the strongest / weakest panels lists. Mirrors
/// `MOVERS_LIMIT` so the two pairs of panels read alike.
const STANDING_LIMIT: usize = 8;

/// A symbol's latest session counts as the bars within this window of its most
/// recent intraday bar. The regular-plus-extended session spans ~16h, while
/// the prior session's bars sit a full ~24h earlier, so 23h cleanly isolates
/// just the latest day.
const SESSION_WINDOW_MS: i64 = 23 * 3600 * 1000;

/// Calendar days of daily closes to pull for the trajectory read. Comfortably
/// over the ~252 trading days `compute`'s trend window needs.
const TREND_LOOKBACK_DAYS: i64 = 400;

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
    /// The stock's rolled-up strong / fair / weak badge (Phase 20); `None`
    /// until its SEC fundamentals have synced.
    strength: Option<compute::Standing>,
}

/// One row in a strongest / weakest panel.
#[derive(Serialize, Clone)]
struct StandingRow {
    ticker: String,
    name: String,
    /// The combined fundamentals-and-trajectory standing this row is ranked by.
    standing: compute::Standing,
    /// Trailing 12-month return, percent; `None` when history is too short.
    ret_12m: Option<f64>,
    /// Width (0..100) of the row's magnitude tint, scaled to the largest
    /// absolute score shown across both panels.
    bar: f64,
    /// Colour hook: true when the combined score is not negative.
    up: bool,
}

/// One curated large-cap stock with everything the home panels need: its
/// price, the close it is changing against, its rolled-up standing, and its
/// trailing-year return. Built once per render and fed to both the movers and
/// the strongest / weakest panels.
struct StockRow {
    ticker: String,
    name: String,
    last: Option<f64>,
    prev: Option<f64>,
    standing: Option<compute::Standing>,
    ret_12m: Option<f64>,
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

    let (index_cards, commodity_cards) = dashboard_cards(&state).await;
    // One scan of the curated stocks feeds both the movers and the strongest /
    // weakest panels.
    let stocks = load_stocks(&state).await;
    let (gainers, losers) = movers(&stocks);
    let (strongest, weakest) = strength_panels(&stocks);

    let extra = minijinja::context! {
        title => "Markets",
        empty => false,
        index_cards => index_cards,
        commodity_cards => commodity_cards,
        gainers => gainers,
        losers => losers,
        strongest => strongest,
        weakest => weakest,
        total => total,
    };
    render(&state, "pages/home.html", "/", extra)
}

/// The dashboard's index and commodity sparkline cards.
///
/// Outside the regular cash session each index card resolves to its index
/// future (see `INDEXES`): the future trades nearly around the clock, so the
/// card stays live overnight instead of freezing on the 16:00 ET close.
async fn dashboard_cards(state: &AppState) -> (Vec<SparkCard>, Vec<SparkCard>) {
    let regular = matches!(
        market::session_at(chrono::Utc::now()),
        market::Session::Regular
    );
    // During the regular cash session show each index itself; outside it,
    // swap in the index future where one exists.
    let index_tickers: Vec<&str> = INDEXES
        .iter()
        .map(|&(index, future)| match future {
            Some(fut) if !regular => fut,
            _ => index,
        })
        .collect();
    let indexes = spark_cards_for(state, &index_tickers).await;
    let commodities = spark_cards_for(state, COMMODITIES).await;
    (indexes, commodities)
}

/// Build a sparkline card for each ticker in `tickers`, in that order: a
/// current price, the day's change, and a sparkline of the latest session's
/// bars. A ticker the universe does not hold is skipped.
async fn spark_cards_for(state: &AppState, tickers: &[&str]) -> Vec<SparkCard> {
    if tickers.is_empty() {
        return Vec::new();
    }
    // One query for the price rows. The `IN` list is built from the hardcoded
    // dashboard consts — never user input — so the placeholder count is fixed
    // and safe.
    let placeholders = vec!["?"; tickers.len()].join(",");
    let sql = format!(
        "SELECT s.ticker, s.name, s.kind, \
           COALESCE(s.last_price, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1)), \
           COALESCE(s.prev_close, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1 OFFSET 1)) \
         FROM symbols s WHERE s.ticker IN ({placeholders})"
    );
    let mut q = sqlx::query_as::<_, SymbolCardRow>(&sql);
    for t in tickers {
        q = q.bind(*t);
    }
    let rows: Vec<SymbolCardRow> = q.fetch_all(&state.pool).await.unwrap_or_default();
    let mut by_ticker: HashMap<String, SymbolCardRow> =
        rows.into_iter().map(|r| (r.0.clone(), r)).collect();

    let mut cards = Vec::with_capacity(tickers.len());
    for &t in tickers {
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

/// Every curated large-cap stock, each graded into a [`StockRow`].
///
/// Restricted to `is_seeded` stocks on purpose (see PLAN.md Phase 11): the
/// home panels are meant to show names worth noticing, not a small user-added
/// symbol's noise. Three queries — price, fundamentals, trailing-year closes —
/// then each stock is graded in `compute`. ETFs, indexes and futures are
/// excluded: only single stocks have the SEC fundamentals a standing needs.
async fn load_stocks(state: &AppState) -> Vec<StockRow> {
    // 1. Price per curated stock: the live last price, else the latest daily
    //    close; plus the prior close it is changing against.
    let price_rows: Vec<(String, String, Option<f64>, Option<f64>)> = sqlx::query_as(
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
    if price_rows.is_empty() {
        return Vec::new();
    }

    // 2. Every stored fundamentals fact for the curated stocks, grouped by
    //    ticker — the basis for each stock's graded ratios.
    let fact_rows: Vec<(String, String, String, i64, Option<i64>, f64)> = sqlx::query_as(
        "SELECT f.ticker, f.metric, f.period, f.fiscal_year, f.fiscal_qtr, f.value \
         FROM fundamentals f JOIN symbols s ON s.ticker = f.ticker \
         WHERE s.is_seeded = 1 AND s.kind = 'stock'",
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    let mut facts: HashMap<String, Vec<models::FundFact>> = HashMap::new();
    for (ticker, metric, period, fiscal_year, fiscal_qtr, value) in fact_rows {
        facts.entry(ticker).or_default().push(models::FundFact {
            metric,
            period,
            fiscal_year,
            fiscal_qtr,
            value,
        });
    }

    // 3. The trailing-year daily closes per curated stock, oldest first.
    let cutoff = (chrono::Utc::now().date_naive() - chrono::Duration::days(TREND_LOOKBACK_DAYS))
        .to_string();
    let close_rows: Vec<(String, f64)> = sqlx::query_as(
        "SELECT p.ticker, p.close FROM daily_prices p JOIN symbols s ON s.ticker = p.ticker \
         WHERE s.is_seeded = 1 AND s.kind = 'stock' AND p.d >= ? ORDER BY p.ticker, p.d",
    )
    .bind(&cutoff)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    let mut closes: HashMap<String, Vec<f64>> = HashMap::new();
    for (ticker, close) in close_rows {
        closes.entry(ticker).or_default().push(close);
    }

    // Assemble: grade each stock off its facts and price, read its trajectory
    // off its closes. A stock with no fundamentals stored yet simply has no
    // standing and is left out of the strongest / weakest ranking.
    price_rows
        .into_iter()
        .map(|(ticker, name, last, prev)| {
            let stock_closes = closes.get(&ticker).map(Vec::as_slice).unwrap_or(&[]);
            let standing = facts.get(&ticker).and_then(|f| {
                let inputs = models::latest_annual_inputs(f, last)?;
                compute::standing(&compute::compute_ratios(&inputs), stock_closes)
            });
            StockRow {
                ticker,
                name,
                last,
                prev,
                standing,
                ret_12m: compute::trailing_return(stock_closes),
            }
        })
        .collect()
}

/// The day's biggest gainers and losers among the curated large-cap stocks.
/// Each row also carries the stock's strong / fair / weak badge (Phase 20).
fn movers(stocks: &[StockRow]) -> (Vec<Mover>, Vec<Mover>) {
    // Keep only stocks with a computable change.
    let mut all: Vec<Mover> = stocks
        .iter()
        .filter_map(|s| {
            let (last, prev) = (s.last?, s.prev?);
            if prev == 0.0 {
                return None;
            }
            let c = compute::change(last, prev);
            Some(Mover {
                ticker: s.ticker.clone(),
                name: s.name.clone(),
                price: last,
                change_abs: c.abs,
                change_pct: c.pct,
                bar: 0.0,
                strength: s.standing,
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

/// The strongest and weakest curated stocks by their combined Phase 20 score.
///
/// A fundamentals-and-trajectory lens on the same curated large-caps the
/// movers panels draw from — a broader read than the day's price move. A stock
/// with no graded standing (its SEC fundamentals have not synced) is left out.
fn strength_panels(stocks: &[StockRow]) -> (Vec<StandingRow>, Vec<StandingRow>) {
    // Rank only the stocks that earned a standing, best combined score first.
    let mut ranked: Vec<&StockRow> = stocks.iter().filter(|s| s.standing.is_some()).collect();
    if ranked.is_empty() {
        return (Vec::new(), Vec::new());
    }
    ranked.sort_by(|a, b| {
        let (sa, sb) = (a.standing.unwrap().score, b.standing.unwrap().score);
        sb.partial_cmp(&sa).unwrap_or(Ordering::Equal)
    });

    let row = |s: &StockRow| {
        let standing = s.standing.unwrap();
        StandingRow {
            ticker: s.ticker.clone(),
            name: s.name.clone(),
            standing,
            ret_12m: s.ret_12m,
            bar: 0.0,
            up: standing.score >= 0.0,
        }
    };
    let mut strongest: Vec<StandingRow> =
        ranked.iter().copied().take(STANDING_LIMIT).map(&row).collect();
    let mut weakest: Vec<StandingRow> = ranked
        .iter()
        .copied()
        .rev()
        .take(STANDING_LIMIT)
        .map(&row)
        .collect();

    // Scale every magnitude tint to the largest absolute score shown, so the
    // two panels read against one another (mirrors the movers tint).
    let max_abs = strongest
        .iter()
        .chain(weakest.iter())
        .map(|r| r.standing.score.abs())
        .fold(0.0_f64, f64::max);
    for r in strongest.iter_mut().chain(weakest.iter_mut()) {
        r.bar = if max_abs > 0.0 {
            (r.standing.score.abs() / max_abs * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
    }
    (strongest, weakest)
}
