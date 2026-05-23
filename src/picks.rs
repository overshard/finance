//! Top picks across four forecast horizons (Phase 30).
//!
//! Each horizon has a separate ranker in `compute` (`pick_day` / `pick_week`
//! / `pick_month` / `pick_year`). This module is the glue: it loads the
//! per-stock inputs once, runs every ranker, and returns the top-N per
//! horizon. Two callers use it — the home page (live, every render) and the
//! scheduler's `picks` snapshot job (once a day, persisted into `picks` so
//! `/backtest` has immutable history to read).
//!
//! Stocks-only across all four horizons, per the user's design call: the
//! short rankers filter on Phase 20 fundamental strength, which only stocks
//! carry, and the year horizon delegates to the Phase 20 standing directly.

use std::collections::HashMap;

use serde::Serialize;
use sqlx::SqlitePool;

use crate::compute::{self, PickInput, Standing};
use crate::db::now_ms;
use crate::models;

/// How many picks each horizon keeps. User's steer: top 5.
pub const PICK_LIMIT: usize = 5;

/// The four horizons the picker covers, in display order. The string keys
/// are what `picks.horizon` carries (stable; never change them, the picks
/// table stores them); labels and descriptions are forward-looking framings
/// — these are forecasts for the named period, ranked off backward-looking
/// signals (recent momentum + quality).
pub const HORIZONS: &[Horizon] = &[
    Horizon {
        key: "day",
        label: "Tomorrow",
        desc: "yesterday's full-day close-to-close momentum + near-52w-high; the live intraday move is intentionally not used (you'd be chasing what already happened)",
    },
    Horizon {
        key: "week",
        label: "Next week",
        desc: "trailing 5-day momentum, gated on RSI not stretched and the close above SMA50",
    },
    Horizon {
        key: "month",
        label: "Next month",
        desc: "trailing 20-day momentum, gated on the close above SMA200 and fundamentals not weak",
    },
    Horizon {
        key: "year",
        label: "Next year",
        desc: "Phase 20 combined fundamentals + 12-month trajectory score (no recent-move signal)",
    },
];

/// One forecast horizon's identity, for both the snapshot table and the UI.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Horizon {
    pub key: &'static str,
    pub label: &'static str,
    /// One-line plain-English description of the horizon's signal, shown
    /// quietly on the home panel so the read of a pick is not opaque.
    pub desc: &'static str,
}

/// One ranked pick within a horizon: the ticker, its rank (1..=PICK_LIMIT),
/// its raw score, and enough context for the home panel to render a row
/// without a second DB pass.
#[derive(Debug, Clone, Serialize)]
pub struct Pick {
    pub rank: u32,
    pub ticker: String,
    pub name: String,
    /// Latest price the ranker scored against (live or the latest close).
    pub price: Option<f64>,
    /// The ranker's raw score. For day/week/month this is a percent return;
    /// for year it is the Phase 20 score on a 100x scale, so the four lines
    /// of the panel read on roughly the same magnitude.
    pub score: f64,
    /// The stock's rolled-up standing, for the row's verdict badge.
    pub standing: Standing,
}

/// One horizon's slate: the horizon's identity over the ranked picks.
#[derive(Debug, Clone, Serialize)]
pub struct PickSlate {
    pub horizon: Horizon,
    pub picks: Vec<Pick>,
}

/// The full per-stock bundle a ranker reads. Owned (not borrowed) so the
/// loader can build it once and the rankers run against it without holding
/// references across an `await`.
pub struct StockBundle {
    pub ticker: String,
    pub name: String,
    /// Latest close (the tail of `closes`), kept separately for display in
    /// the home panel rows.
    pub last_price: Option<f64>,
    /// Daily closes oldest first, up to the lookback window the rankers need
    /// (longest is the 200-day SMA, with comfortable slack).
    pub closes: Vec<f64>,
    pub standing: Option<Standing>,
}

/// Trading days of daily closes the rankers need, oldest first. The 200-day
/// SMA is the longest signal; an extra year of slack lets the 52-week-high
/// bias and any later refinement read off the same loaded window.
const LOOKBACK_DAYS: i64 = 500;

/// Run all four rankers over `bundles` and return one [`PickSlate`] per
/// horizon, top picks first. A stock missing inputs is silently skipped by
/// the ranker; a horizon with too few qualifying stocks returns however many
/// it has (down to zero).
pub fn compute_picks(bundles: &[StockBundle]) -> Vec<PickSlate> {
    HORIZONS
        .iter()
        .map(|h| {
            // Score every bundle through this horizon's ranker; keep only the
            // qualifying ones with a real score.
            let mut scored: Vec<(&StockBundle, f64)> = bundles
                .iter()
                .filter_map(|b| {
                    let input = PickInput {
                        closes: &b.closes,
                        standing: b.standing,
                    };
                    let score = match h.key {
                        "day" => compute::pick_day(&input),
                        "week" => compute::pick_week(&input),
                        "month" => compute::pick_month(&input),
                        "year" => compute::pick_year(&input),
                        _ => None,
                    }?;
                    Some((b, score))
                })
                .collect();
            // Highest score first. `partial_cmp` on a finite f64 always
            // succeeds — `Some(Ordering::Equal)` on the rare tie keeps the
            // sort stable.
            scored.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let picks = scored
                .into_iter()
                .take(PICK_LIMIT)
                .enumerate()
                .map(|(i, (b, score))| Pick {
                    rank: (i + 1) as u32,
                    ticker: b.ticker.clone(),
                    name: b.name.clone(),
                    price: b.last_price,
                    score,
                    // The standing is guaranteed `Some` here: every ranker
                    // requires it, so a stock without one has already been
                    // dropped above.
                    standing: b.standing.expect("ranker required standing"),
                })
                .collect();
            PickSlate { horizon: *h, picks }
        })
        .collect()
}

/// Load every curated-stock bundle the rankers need from the DB in three
/// queries — the same shape as `routes::home::load_stocks` (Phase 20), but
/// without the trailing-year cut so the long SMA / 52w-high windows are
/// available. Pure read; safe to call from any caller (route render, the
/// scheduler's snapshot job, the backtest replay).
pub async fn load_bundles(pool: &SqlitePool) -> sqlx::Result<Vec<StockBundle>> {
    // 1. Price per curated stock: live last price falling back to the latest
    //    daily close, used for display alongside each pick row.
    let price_rows: Vec<(String, String, Option<f64>)> = sqlx::query_as(
        "SELECT s.ticker, s.name, \
           COALESCE(s.last_price, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1)) \
         FROM symbols s WHERE s.is_seeded = 1 AND s.kind = 'stock'",
    )
    .fetch_all(pool)
    .await?;
    if price_rows.is_empty() {
        return Ok(Vec::new());
    }

    // 2. Every fundamentals fact for the curated stocks, grouped by ticker.
    let fact_rows: Vec<(String, String, String, i64, Option<i64>, f64)> = sqlx::query_as(
        "SELECT f.ticker, f.metric, f.period, f.fiscal_year, f.fiscal_qtr, f.value \
         FROM fundamentals f JOIN symbols s ON s.ticker = f.ticker \
         WHERE s.is_seeded = 1 AND s.kind = 'stock'",
    )
    .fetch_all(pool)
    .await?;
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

    // 3. Daily closes for the LOOKBACK_DAYS window, oldest first per ticker.
    let cutoff = (chrono::Utc::now().date_naive() - chrono::Duration::days(LOOKBACK_DAYS))
        .to_string();
    let close_rows: Vec<(String, f64)> = sqlx::query_as(
        "SELECT p.ticker, p.close FROM daily_prices p JOIN symbols s ON s.ticker = p.ticker \
         WHERE s.is_seeded = 1 AND s.kind = 'stock' AND p.d >= ? ORDER BY p.ticker, p.d",
    )
    .bind(&cutoff)
    .fetch_all(pool)
    .await?;
    let mut closes: HashMap<String, Vec<f64>> = HashMap::new();
    for (ticker, close) in close_rows {
        closes.entry(ticker).or_default().push(close);
    }

    Ok(price_rows
        .into_iter()
        .map(|(ticker, name, last_price)| {
            let stock_closes = closes.remove(&ticker).unwrap_or_default();
            let standing = facts.get(&ticker).and_then(|f| {
                let inputs = models::latest_annual_inputs(f, last_price)?;
                compute::standing(&compute::compute_ratios(&inputs), &stock_closes)
            });
            StockBundle {
                ticker,
                name,
                last_price,
                closes: stock_closes,
                standing,
            }
        })
        .collect())
}

/// Replace `picks` for one snapshot date with the given slates. Idempotent on
/// (snapshot_date, horizon, rank): re-running the snapshot for the same date
/// (e.g. a manual rerun) overwrites cleanly. The whole write is a single
/// transaction so a half-written snapshot is never observable by `/backtest`.
pub async fn write_snapshot(
    pool: &SqlitePool,
    snapshot_date: &str,
    slates: &[PickSlate],
) -> sqlx::Result<usize> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM picks WHERE snapshot_date = ?")
        .bind(snapshot_date)
        .execute(&mut *tx)
        .await?;
    let mut written = 0usize;
    for slate in slates {
        for pick in &slate.picks {
            sqlx::query(
                "INSERT INTO picks \
                   (snapshot_date, horizon, rank, ticker, score, price_at_pick) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(snapshot_date)
            .bind(slate.horizon.key)
            .bind(pick.rank as i64)
            .bind(&pick.ticker)
            .bind(pick.score)
            // The pick's entry price is whatever the ranker scored against
            // (the day's close after `daily_close` runs). The backtest reads
            // it back as the position's cost basis.
            .bind(pick.price.unwrap_or(0.0))
            .execute(&mut *tx)
            .await?;
            written += 1;
        }
    }
    tx.commit().await?;
    Ok(written)
}

/// Take a daily snapshot of every horizon's picks for `date` (an ET trading
/// date, `YYYY-MM-DD`). Called by the scheduler right after `daily_close`,
/// when every stock carries a fresh close.
pub async fn snapshot_today(pool: &SqlitePool, date: &str) -> anyhow::Result<usize> {
    let bundles = load_bundles(pool).await?;
    let slates = compute_picks(&bundles);
    let n = write_snapshot(pool, date, &slates).await?;
    let _ = now_ms(); // touched so a future audit-log row could borrow it
    Ok(n)
}

// ────────────────────────── backtest (Phase 30) ────────────────────────────
//
// Walks the picker over historical `daily_prices`, simulating "what would $X
// have done if you'd followed today's algo over the past N years?". Reads
// every curated stock's full close history once, then at each rebalance date
// truncates each stock's closes to that point and runs the horizon's ranker.
// The picks held for one stride, equal-weight, are sold at the next
// rebalance close.
//
// Acknowledged look-ahead: fundamentals are today's (we do not store a
// per-period history of Phase 7 facts). The page's disclaimer surfaces
// this — the backtest is a stress test "for fun and testing", not a clean
// out-of-sample evaluation.

/// One historical bar a backtest reads off — a trading date and the close.
#[derive(Debug, Clone)]
pub struct HistBar {
    pub date: String,
    pub close: f64,
}

/// Per-stock bundle for the backtest: the standing carried as today's value
/// (look-ahead bias acknowledged), and the full close history the rankers
/// walk over.
pub struct HistBundle {
    pub ticker: String,
    pub bars: Vec<HistBar>,
    pub standing: Option<Standing>,
}

/// The benchmark every horizon's strategy is measured against. Hardcoded to
/// `^SPX` — the broad-market index our universe is built around — so the
/// backtest's "did you beat the market?" read is consistent across horizons.
pub const BENCHMARK_TICKER: &str = "^SPX";

/// Calendar days of history the backtest's loader pulls. Comfortably covers
/// the longest horizon (year, ~5 rebalances × 252 bars ≈ 5 years) plus the
/// warm-up the rankers need (200-day SMA + a year for the standing). Capping
/// here keeps the load query off the deep history some indexes carry
/// (`^SPX` goes back to 1789), trading no real backtest depth for a fast
/// request.
const HIST_LOOKBACK_DAYS: i64 = 365 * 7;

/// One pick within a backtest period — its entry and exit prices and the
/// resulting return.
#[derive(Debug, Clone, Serialize)]
pub struct BacktestPick {
    pub ticker: String,
    pub entry_price: f64,
    pub exit_price: f64,
    /// Percent return over the period this pick was held.
    pub return_pct: f64,
}

/// One rebalance period of the backtest: the picks held, the basket's
/// equal-weight return, and how the benchmark fared over the same dates.
#[derive(Debug, Clone, Serialize)]
pub struct BacktestPeriod {
    pub start_date: String,
    pub end_date: String,
    pub picks: Vec<BacktestPick>,
    pub basket_return_pct: f64,
    pub benchmark_return_pct: f64,
    pub beat_benchmark: bool,
}

/// One point on the equity curve: the strategy's running $-value and the
/// benchmark's running $-value (both starting at the requested capital).
#[derive(Debug, Clone, Serialize)]
pub struct EquityPoint {
    pub date: String,
    pub strategy: f64,
    pub benchmark: f64,
}

/// Summary stats the backtest page surfaces alongside the chart.
#[derive(Debug, Clone, Serialize)]
pub struct BacktestStats {
    pub final_strategy: f64,
    pub final_benchmark: f64,
    pub total_return_pct: f64,
    pub benchmark_return_pct: f64,
    /// Annualised. Equals `total_return_pct` for windows of a year or less.
    pub cagr_pct: f64,
    pub benchmark_cagr_pct: f64,
    /// Fraction of individual picks that closed up over their hold period.
    pub per_pick_win_rate: f64,
    /// Fraction of periods where the basket beat the benchmark.
    pub per_period_win_rate: f64,
    pub num_periods: u32,
    pub num_picks: u32,
    pub period_start: String,
    pub period_end: String,
}

/// One horizon's full backtest result — equity curve, period-by-period
/// detail, summary stats — for the page's JSON feed.
#[derive(Debug, Clone, Serialize)]
pub struct BacktestResult {
    pub horizon: Horizon,
    pub starting_capital: f64,
    pub bench_ticker: &'static str,
    pub equity: Vec<EquityPoint>,
    pub periods: Vec<BacktestPeriod>,
    pub stats: Option<BacktestStats>,
}

/// Trading-day stride per horizon (one rebalance per stride). Day = daily
/// rebalance; year = once a year. Calendar days happen to converge: 252
/// trading days ≈ 1 year.
fn stride_for(horizon_key: &str) -> usize {
    match horizon_key {
        "day" => 1,
        "week" => 5,
        "month" => 20,
        "year" => 252,
        _ => 1,
    }
}

/// Maximum backtest window per horizon, in trading bars. Cap each so the
/// number of rebalances stays meaningful without dragging the request: day
/// has too many otherwise, year too few. Roughly 1y / 2y / 4y / 5y.
fn max_bars_for(horizon_key: &str) -> usize {
    match horizon_key {
        "day" => 252,
        "week" => 504,
        "month" => 1008,
        "year" => 1260,
        _ => 252,
    }
}

/// Load every curated stock's full close history plus the benchmark's. Two
/// queries. Returns the bundles and the benchmark's bars, separately so the
/// caller can index them independently. The bundles' `standing` is today's
/// value, computed off today's fundamentals and the full history (the
/// acknowledged look-ahead).
pub async fn load_hist_bundles(
    pool: &SqlitePool,
) -> sqlx::Result<(Vec<HistBundle>, Vec<HistBar>)> {
    // 1. The curated stocks' price, plus today's standing as for the live
    //    picks. (Name not needed: the backtest table shows tickers only.)
    let price_rows: Vec<(String, Option<f64>)> = sqlx::query_as(
        "SELECT s.ticker, \
           COALESCE(s.last_price, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1)) \
         FROM symbols s WHERE s.is_seeded = 1 AND s.kind = 'stock'",
    )
    .fetch_all(pool)
    .await?;
    let fact_rows: Vec<(String, String, String, i64, Option<i64>, f64)> = sqlx::query_as(
        "SELECT f.ticker, f.metric, f.period, f.fiscal_year, f.fiscal_qtr, f.value \
         FROM fundamentals f JOIN symbols s ON s.ticker = f.ticker \
         WHERE s.is_seeded = 1 AND s.kind = 'stock'",
    )
    .fetch_all(pool)
    .await?;
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

    // 2. Daily closes for the curated stocks, oldest first per ticker. Capped
    //    at `HIST_LOOKBACK_DAYS` of calendar history so a deep universe (some
    //    indexes go back to the 1800s) does not pull a million-row scan; the
    //    backtest's longest horizon (year, 252-bar stride × ~5 rebalances)
    //    fits comfortably inside this window.
    let cutoff = (chrono::Utc::now().date_naive()
        - chrono::Duration::days(HIST_LOOKBACK_DAYS))
    .to_string();
    let close_rows: Vec<(String, String, f64)> = sqlx::query_as(
        "SELECT p.ticker, p.d, p.close FROM daily_prices p \
         JOIN symbols s ON s.ticker = p.ticker \
         WHERE s.is_seeded = 1 AND s.kind = 'stock' AND p.d >= ? \
         ORDER BY p.ticker, p.d",
    )
    .bind(&cutoff)
    .fetch_all(pool)
    .await?;
    let mut bars_by: HashMap<String, Vec<HistBar>> = HashMap::new();
    for (ticker, date, close) in close_rows {
        bars_by.entry(ticker).or_default().push(HistBar { date, close });
    }

    let bundles: Vec<HistBundle> = price_rows
        .into_iter()
        .map(|(ticker, last_price)| {
            let bars = bars_by.remove(&ticker).unwrap_or_default();
            let closes: Vec<f64> = bars.iter().map(|b| b.close).collect();
            let standing = facts.get(&ticker).and_then(|f| {
                let inputs = models::latest_annual_inputs(f, last_price)?;
                compute::standing(&compute::compute_ratios(&inputs), &closes)
            });
            HistBundle {
                ticker,
                bars,
                standing,
            }
        })
        .collect();

    // 3. The benchmark's history over the same window. It anchors the
    //    timeline: rebalance dates come from its bar list, since every
    //    stock's price is sampled at-or-before those dates.
    let bench_rows: Vec<(String, f64)> = sqlx::query_as(
        "SELECT d, close FROM daily_prices WHERE ticker = ? AND d >= ? ORDER BY d",
    )
    .bind(BENCHMARK_TICKER)
    .bind(&cutoff)
    .fetch_all(pool)
    .await?;
    let bench_bars: Vec<HistBar> = bench_rows
        .into_iter()
        .map(|(date, close)| HistBar { date, close })
        .collect();

    Ok((bundles, bench_bars))
}

/// Rank `bundles` as of bar index `idx` (within the benchmark's date list).
/// Each bundle is sliced to its own bars at-or-before the benchmark's
/// `as_of` date, with `last_price` and `prev_close` derived from that slice.
/// A stock that does not yet have two bars by that date is silently dropped.
fn rank_at(
    bundles: &[HistBundle],
    as_of: &str,
    horizon_key: &str,
) -> Vec<(String, f64, f64)> {
    let mut scored: Vec<(String, f64, f64)> = bundles
        .iter()
        .filter_map(|b| {
            // Find the index of the bundle's last bar at-or-before `as_of`.
            // `partition_point` returns the first index *after* the predicate
            // holds, so subtracting one yields the bar at-or-before. The
            // bars are date-string sorted (YYYY-MM-DD compares correctly).
            let upper = b.bars.partition_point(|x| x.date.as_str() <= as_of);
            if upper < 2 {
                return None;
            }
            let idx = upper - 1;
            let last_price = b.bars[idx].close;
            let prev_close = b.bars[idx - 1].close;
            let closes: Vec<f64> = b.bars[..=idx].iter().map(|x| x.close).collect();
            let input = PickInput {
                closes: &closes,
                standing: b.standing,
            };
            // `last_price` and `prev_close` are read above only for the
            // entry-price recording below; the rankers themselves consume
            // `closes` exclusively.
            let _ = (last_price, prev_close);
            let score = match horizon_key {
                "day" => compute::pick_day(&input),
                "week" => compute::pick_week(&input),
                "month" => compute::pick_month(&input),
                "year" => compute::pick_year(&input),
                _ => None,
            }?;
            Some((b.ticker.clone(), score, last_price))
        })
        .collect();
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(PICK_LIMIT);
    scored
}

/// Find the close at-or-before `date` in a bundle's bars. The bars are
/// date-string sorted (YYYY-MM-DD compares lexicographically), so a binary-
/// search partition_point is exact.
fn close_at_or_before(bars: &[HistBar], date: &str) -> Option<f64> {
    let upper = bars.partition_point(|x| x.date.as_str() <= date);
    (upper > 0).then(|| bars[upper - 1].close).filter(|c| *c > 0.0)
}

/// Run one horizon's backtest over the data already loaded. Walks back from
/// the benchmark's last bar by `stride` until either `max_bars` is exhausted
/// or the start of history is reached, then runs the periods forward in
/// time so the equity curve is chronological. `starting_capital` is the
/// strategy's and the benchmark's initial $-value.
pub fn run_backtest(
    bundles: &[HistBundle],
    bench: &[HistBar],
    horizon: Horizon,
    starting_capital: f64,
) -> BacktestResult {
    let stride = stride_for(horizon.key);
    let max_bars = max_bars_for(horizon.key);
    let empty = || BacktestResult {
        horizon,
        starting_capital,
        bench_ticker: BENCHMARK_TICKER,
        equity: Vec::new(),
        periods: Vec::new(),
        stats: None,
    };
    if bench.len() < stride * 2 + 1 {
        return empty();
    }

    // Rebalance points: indices into `bench`, oldest first, spaced `stride`
    // apart. The newest possible point is one stride from the end (so we
    // have a "hold for stride days" window past it).
    let end_idx = bench.len() - 1;
    let last_rebal = end_idx - stride;
    let earliest_rebal = bench.len().saturating_sub(max_bars).max(stride);
    if earliest_rebal > last_rebal {
        return empty();
    }
    let mut rebal_idxs: Vec<usize> = Vec::new();
    let mut i = last_rebal;
    while i >= earliest_rebal {
        rebal_idxs.push(i);
        if i < stride {
            break;
        }
        i -= stride;
    }
    rebal_idxs.reverse();

    let mut periods: Vec<BacktestPeriod> = Vec::new();
    let mut equity: Vec<EquityPoint> = Vec::new();
    let mut strat_val = starting_capital;
    let mut bench_val = starting_capital;

    // The equity curve starts at the first rebalance's date. Anchoring it
    // there keeps the strategy and benchmark series aligned.
    equity.push(EquityPoint {
        date: bench[rebal_idxs[0]].date.clone(),
        strategy: strat_val,
        benchmark: bench_val,
    });

    for win in rebal_idxs.windows(2) {
        let (start_idx, end_idx) = (win[0], win[1]);
        let start_date = bench[start_idx].date.as_str();
        let end_date = bench[end_idx].date.as_str();
        let bench_start = bench[start_idx].close;
        let bench_end = bench[end_idx].close;
        if bench_start <= 0.0 {
            continue;
        }
        let bench_ret_pct = (bench_end - bench_start) / bench_start * 100.0;

        // The picks the horizon's ranker would have named at `start_date`,
        // entry prices captured from the rank, exit prices from each
        // stock's at-or-before-`end_date` close.
        let ranked = rank_at(bundles, start_date, horizon.key);
        let picks: Vec<BacktestPick> = ranked
            .into_iter()
            .filter_map(|(ticker, _score, entry)| {
                let bundle = bundles.iter().find(|b| b.ticker == ticker)?;
                let exit = close_at_or_before(&bundle.bars, end_date)?;
                if entry <= 0.0 {
                    return None;
                }
                let ret = (exit - entry) / entry * 100.0;
                Some(BacktestPick {
                    ticker,
                    entry_price: entry,
                    exit_price: exit,
                    return_pct: ret,
                })
            })
            .collect();

        // Skip a period where nothing qualified — the strategy is "all cash"
        // that period, which is not what the backtest is meant to show. The
        // benchmark also does not advance, so equity simply pauses.
        if picks.is_empty() {
            continue;
        }

        let basket_ret_pct = picks.iter().map(|p| p.return_pct).sum::<f64>()
            / picks.len() as f64;
        strat_val *= 1.0 + basket_ret_pct / 100.0;
        bench_val *= 1.0 + bench_ret_pct / 100.0;

        periods.push(BacktestPeriod {
            start_date: start_date.to_string(),
            end_date: end_date.to_string(),
            picks,
            basket_return_pct: basket_ret_pct,
            benchmark_return_pct: bench_ret_pct,
            beat_benchmark: basket_ret_pct > bench_ret_pct,
        });
        equity.push(EquityPoint {
            date: end_date.to_string(),
            strategy: strat_val,
            benchmark: bench_val,
        });
    }

    let stats = if let (Some(first), Some(last)) = (periods.first(), periods.last()) {
        let total = (strat_val / starting_capital - 1.0) * 100.0;
        let bench_total = (bench_val / starting_capital - 1.0) * 100.0;
        let num_picks: u32 = periods.iter().map(|p| p.picks.len() as u32).sum();
        let per_pick_wins: u32 = periods
            .iter()
            .flat_map(|p| p.picks.iter())
            .filter(|p| p.return_pct > 0.0)
            .count() as u32;
        let per_period_wins =
            periods.iter().filter(|p| p.beat_benchmark).count() as u32;
        // Years between the first period's start and the last period's end —
        // by actual calendar days / 365.25, so a 2.5-year backtest annualises
        // correctly.
        let years = chrono::NaiveDate::parse_from_str(&last.end_date, "%Y-%m-%d")
            .ok()
            .zip(chrono::NaiveDate::parse_from_str(&first.start_date, "%Y-%m-%d").ok())
            .map(|(a, b)| (a - b).num_days() as f64 / 365.25)
            .unwrap_or(1.0)
            .max(1.0 / 12.0);
        let cagr = if years > 1.0 {
            ((strat_val / starting_capital).powf(1.0 / years) - 1.0) * 100.0
        } else {
            total
        };
        let bench_cagr = if years > 1.0 {
            ((bench_val / starting_capital).powf(1.0 / years) - 1.0) * 100.0
        } else {
            bench_total
        };
        Some(BacktestStats {
            final_strategy: strat_val,
            final_benchmark: bench_val,
            total_return_pct: total,
            benchmark_return_pct: bench_total,
            cagr_pct: cagr,
            benchmark_cagr_pct: bench_cagr,
            per_pick_win_rate: per_pick_wins as f64 / num_picks.max(1) as f64 * 100.0,
            per_period_win_rate: per_period_wins as f64 / periods.len().max(1) as f64
                * 100.0,
            num_periods: periods.len() as u32,
            num_picks,
            period_start: first.start_date.clone(),
            period_end: last.end_date.clone(),
        })
    } else {
        None
    };

    BacktestResult {
        horizon,
        starting_capital,
        bench_ticker: BENCHMARK_TICKER,
        equity,
        periods,
        stats,
    }
}
