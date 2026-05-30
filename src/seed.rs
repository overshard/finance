//! First-run universe seed: load the curated starter list, then backfill deep
//! daily history for the symbols that do not have it yet.
//!
//! Resumable and quota-friendly: symbols that already hold history are skipped,
//! so re-running `make seed` after a partial run continues where it stopped.
//! Every history request goes through the persistent `EndpointGuard` (the
//! `yahoo` one), which paces the loop and stops it early if the circuit breaker
//! is open or the hourly budget is spent, instead of grinding the list against
//! a guarded endpoint.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use sqlx::SqlitePool;

use crate::db::{now_ms, set_meta};
use crate::guard::{EndpointGuard, Permit};
use crate::providers::{DailyBar, HistoryProvider};
use crate::Config;

struct SeedSymbol {
    ticker: String,
    name: String,
    kind: String,
    exchange: Option<String>,
    /// Phase 28: the curated benchmark index a fund tracks (e.g. `^SPX`),
    /// for the relative-performance overlay on the ETF symbol page. Only
    /// the broad-market ETFs carry one in `starter.csv`; everything else
    /// (including stocks, futures, indexes, sector / bond / commodity ETFs)
    /// leaves it empty and the overlay is hidden.
    benchmark: Option<String>,
}

fn parse_universe(path: &Path) -> Result<Vec<SeedSymbol>> {
    let mut rdr = csv::Reader::from_path(path)
        .with_context(|| format!("opening universe file {}", path.display()))?;
    let mut out = Vec::new();
    for rec in rdr.records() {
        let rec = rec?;
        let cell = |i: usize| rec.get(i).unwrap_or("").trim().to_string();
        let ticker = cell(0).to_uppercase();
        if ticker.is_empty() {
            continue;
        }
        let opt = |s: String| if s.is_empty() { None } else { Some(s) };
        out.push(SeedSymbol {
            ticker,
            name: cell(1),
            kind: cell(2),
            exchange: opt(cell(3)),
            benchmark: opt(cell(4)),
        });
    }
    Ok(out)
}

/// Outcome of a universe sync, for logging.
pub struct SyncReport {
    /// Symbols in the curated CSV (all upserted).
    pub total: usize,
    /// Seeded symbols deleted because they were dropped from the CSV.
    pub pruned: u64,
}

/// Reconcile the `symbols` table to the curated CSV. Local only, no network:
/// upsert every listed symbol and prune the curated ones that were dropped from
/// the list. Idempotent, so it runs on every boot (see `scheduler::run_boot_seed`)
/// and a CSV edit (added or removed symbols) takes effect on the next deploy
/// without a manual re-seed. The history backfill for any newly-added symbol is
/// handled separately: the first-run seed loop below, or — once the seed has
/// completed — the incremental `run_history` job, which picks up any symbol with
/// a `NULL history_synced_at`.
pub async fn sync_universe(pool: &SqlitePool, config: &Config) -> Result<SyncReport> {
    let path = config.root.join("universe/starter.csv");
    let symbols = parse_universe(&path)?;

    // Upsert every symbol. Phase 28: the curated benchmark column is set from
    // the CSV on each pass, so a re-run picks up any newly-curated mapping. A
    // `NULL` benchmark stays `NULL` (they all live in the CSV).
    for s in &symbols {
        let now = now_ms();
        sqlx::query(
            "INSERT INTO symbols \
               (ticker, name, kind, exchange, benchmark, is_seeded, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, 1, ?, ?) \
             ON CONFLICT(ticker) DO UPDATE SET \
               name = excluded.name, kind = excluded.kind, \
               exchange = excluded.exchange, benchmark = excluded.benchmark, \
               is_seeded = 1, updated_at = excluded.updated_at",
        )
        .bind(&s.ticker)
        .bind(&s.name)
        .bind(&s.kind)
        .bind(&s.exchange)
        .bind(&s.benchmark)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
    }

    // Prune curated symbols dropped from the CSV. Only `is_seeded = 1` rows are
    // eligible, so a user-added symbol (`is_seeded = 0`) is never touched. The
    // delete cascades to every child table (all `REFERENCES symbols(ticker) ON
    // DELETE CASCADE`, foreign keys enabled in db::init), so no orphan rows are
    // left behind. The IN-list is bounded by the curated list size (~560), well
    // under SQLite's bound-parameter limit.
    let placeholders = vec!["?"; symbols.len()].join(",");
    let sql = format!(
        "DELETE FROM symbols WHERE is_seeded = 1 AND ticker NOT IN ({placeholders})"
    );
    let mut q = sqlx::query(&sql);
    for s in &symbols {
        q = q.bind(&s.ticker);
    }
    let pruned = q.execute(pool).await?.rows_affected();

    Ok(SyncReport { total: symbols.len(), pruned })
}

/// Run the seed: reconcile the universe, then backfill daily history for any
/// symbol that still lacks it.
pub async fn run(pool: &SqlitePool, config: &Config, history: &dyn HistoryProvider) -> Result<()> {
    let started = Instant::now();
    let report = sync_universe(pool, config).await?;
    tracing::info!(
        "seed: {} symbols synced, {} pruned",
        report.total,
        report.pruned
    );

    // Symbols that still need a deep backfill. Two cases:
    //  - `history_last_date IS NULL`: never fetched (the normal first-run case).
    //  - `history_first_date = history_last_date`: only a single stored bar,
    //    which means the symbol was added after the initial seed and a
    //    `daily_close` snapshot stamped its `history_last_date` before any
    //    range=max backfill ran — so the incremental path, which only asks for
    //    the window since the last bar, can never reach its deep history. These
    //    get re-fetched with range=max (the loop always passes `None`). A symbol
    //    Yahoo genuinely has one bar for simply stays a one-bar re-fetch; cheap.
    // This keeps re-runs cheap and lets a quota-limited run resume later.
    // Futures are included now: Yahoo serves `=F` daily history, unlike the
    // Stooq source this replaced.
    let pending: Vec<String> = sqlx::query_scalar(
        "SELECT ticker FROM symbols \
         WHERE is_seeded = 1 \
           AND (history_last_date IS NULL OR history_first_date = history_last_date) \
         ORDER BY ticker",
    )
    .fetch_all(pool)
    .await?;

    if pending.is_empty() {
        set_meta(pool, "seed_completed", "1").await?;
        tracing::info!("seed: every symbol already has history, nothing to fetch");
        return Ok(());
    }
    tracing::info!("seed: {} symbols need a history backfill", pending.len());

    // Every history request passes through the persistent endpoint guard: it
    // paces the loop and, once the breaker opens or the hourly budget runs out,
    // refuses further requests so the seed stops cleanly rather than grinding
    // the rest of the list. A stopped seed is resumable (see below). History
    // shares the `yahoo` guard with live quotes, so it must carry the same
    // budget rather than the 200-default `new` ceiling.
    let guard = EndpointGuard::with_budget(
        pool.clone(),
        history.name(),
        crate::scheduler::YAHOO_BUDGET,
    );

    let mut ok = 0usize;
    let mut stopped: Option<String> = None;
    for (i, ticker) in pending.iter().enumerate() {
        match guard.acquire().await? {
            Permit::Granted => {}
            Permit::Denied(why) => {
                stopped = Some(why);
                break;
            }
        }
        match history.daily(ticker, None).await {
            Ok(bars) if !bars.is_empty() => {
                guard.record_success().await?;
                let n = bars.len();
                store_daily(pool, ticker, &bars).await?;
                ok += 1;
                tracing::info!(
                    "seed: {ticker} <- {n} daily bars ({}/{})",
                    i + 1,
                    pending.len()
                );
            }
            Ok(_) => {
                // A valid but empty response: the request itself succeeded, so
                // it counts as a success for the guard; the symbol simply has
                // no history to store.
                guard.record_success().await?;
                tracing::warn!("seed: {ticker} returned no data");
            }
            Err(e) => {
                guard.record_failure(&e).await?;
                tracing::warn!("seed: {ticker} failed: {e:#}");
            }
        }
    }
    if let Some(why) = &stopped {
        tracing::warn!(
            "seed: stopped early — {why}; {ok} symbols backfilled and kept, \
             re-run `make seed` (or restart the server) later to continue"
        );
    }

    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM symbols \
         WHERE is_seeded = 1 AND history_last_date IS NULL",
    )
    .fetch_one(pool)
    .await?;
    if remaining == 0 {
        set_meta(pool, "seed_completed", "1").await?;
        set_meta(pool, "seed_at", &now_ms().to_string()).await?;
    }
    tracing::info!(
        "seed: {ok} backfilled, {remaining} still missing, {:.1}s",
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// Upsert one symbol's daily bars in a single transaction and refresh its
/// `symbols` history-range columns.
pub async fn store_daily(pool: &SqlitePool, ticker: &str, bars: &[DailyBar]) -> Result<()> {
    if bars.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for b in bars {
        sqlx::query(
            "INSERT INTO daily_prices (ticker, d, open, high, low, close, volume) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(ticker, d) DO UPDATE SET \
               open = excluded.open, high = excluded.high, low = excluded.low, \
               close = excluded.close, volume = excluded.volume",
        )
        .bind(ticker)
        .bind(&b.d)
        .bind(b.open)
        .bind(b.high)
        .bind(b.low)
        .bind(b.close)
        .bind(b.volume)
        .execute(&mut *tx)
        .await?;
    }
    // Recompute the history range from the stored rows (visible inside this
    // transaction) rather than from just the bars passed in. An incremental
    // window or a single daily-close append carries only recent bars, so
    // taking min/max of the argument alone would clobber the true earliest
    // date with the window start.
    let now = now_ms();
    sqlx::query(
        "UPDATE symbols SET history_synced_at = ?, \
         history_first_date = (SELECT MIN(d) FROM daily_prices WHERE ticker = ?), \
         history_last_date = (SELECT MAX(d) FROM daily_prices WHERE ticker = ?), \
         updated_at = ? WHERE ticker = ?",
    )
    .bind(now)
    .bind(ticker)
    .bind(ticker)
    .bind(now)
    .bind(ticker)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}
