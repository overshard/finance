//! Background job scheduler.
//!
//! One long-lived tokio task wakes on a fixed tick and runs the market-data
//! maintenance jobs as they fall due:
//!  - the first-run universe seed, once, while `meta.seed_completed` is unset;
//!  - an incremental daily-history refresh from Stooq (~every 6h), touching
//!    only symbols whose stored history has gone stale;
//!  - market-hours intraday quotes from Yahoo for the symbols a browser is
//!    actually viewing right now — demand-driven via the stream hub's interest
//!    registry, so nothing is polled when nobody is watching;
//!  - a once-a-day close fetch that snapshots the whole universe from Yahoo
//!    shortly after the regular session ends;
//!  - a prune of aged `intraday_bars` and `fetch_log` rows (~daily).
//!
//! Every data job records a `fetch_log` row, refreshes its `data_status` row,
//! and pings the stream hub so the `/health` page reflects it live. Outbound
//! Stooq calls go through the persistent `EndpointGuard` (see `src/guard.rs`),
//! which paces requests and stops a job early when the circuit breaker is open
//! or the hourly request budget is spent.
//!
//! Modelled on `status/src/scheduler.rs`, scaled down: finance's jobs run
//! hours apart, not seconds, and are inherently sequential (the pacing is the
//! point), so they run inline in the loop task rather than via semaphores.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sqlx::SqlitePool;
use tokio::task::JoinHandle;

use crate::db::{get_meta, now_ms, set_meta};
use crate::guard::{EndpointGuard, Permit};
use crate::market;
use crate::providers::sec::SecProvider;
use crate::providers::yahoo::YahooProvider;
use crate::providers::{
    self, DailyBar, DividendEvent, Fact, FilingRecord, FundId, FundMetadata,
    FundShape, FundamentalsProvider, HistoryProvider, IntradayBar, OwnershipPerson,
    PortfolioData, Quote, QuoteProvider,
};
use crate::stream::{Hub, QuoteUpdate, StreamEvent};
use crate::{seed, Config};

/// How often the loop wakes to check whether a job is due. The jobs themselves
/// run hours apart; a one-minute tick is plenty responsive and nearly free
/// (two small SELECTs per wake).
const TICK: Duration = Duration::from_secs(60);

/// Incremental daily-history refresh cadence.
const HISTORY_INTERVAL_SECS: i64 = 6 * 3600;

/// Retry cadence after a history run stops early (guard breaker open or hourly
/// budget spent) with work still pending — far shorter than the full interval
/// so a large backlog (e.g. the symbols a deploy just added, plus any coarse
/// series being healed) drains over the next hour rather than over days. The
/// guard still gates every request, so a too-soon retry is a cheap no-op while
/// the breaker is open.
const HISTORY_RETRY_SECS: i64 = 30 * 60;

/// A symbol's daily history counts as stale once `history_synced_at` is older
/// than this. Kept under 24h so the ~6-hourly job refreshes each symbol about
/// once per trading day (markets emit one new daily bar a day) without
/// re-fetching symbols already touched a few hours ago.
const HISTORY_STALE_SECS: i64 = 20 * 3600;

/// Prune cadence and the two retention windows it enforces.
const PRUNE_INTERVAL_SECS: i64 = 24 * 3600;
const INTRADAY_RETENTION_DAYS: i64 = 14;
const FETCH_LOG_RETENTION_DAYS: i64 = 30;

/// Per-hour request ceiling for the Yahoo endpoint guard. Higher than Stooq's
/// default 200: an intraday tick sweeps every viewed symbol, and a daily-close
/// run touches the whole ~144-symbol universe at once. Still a hard cap that
/// stops a runaway loop well short of anything Yahoo would refuse.
/// `pub(crate)` so the add-symbol route builds its Yahoo guard with the same
/// ceiling (see `routes::symbols`).
pub(crate) const YAHOO_BUDGET: i64 = 1000;

/// SEC fundamentals job: how often the loop checks whether the sweep is due,
/// and how stale a company's SEC data may go before it is re-fetched. SEC
/// filings land quarterly, so a weekly refresh is ample.
const SEC_INTERVAL_SECS: i64 = 24 * 3600;
const SEC_STALE_SECS: i64 = 7 * 24 * 3600;

/// Per-hour request ceiling for the SEC endpoint guard. A first-run full sweep
/// is one bulk ticker-map call plus two calls per stock (~220 for the starter
/// universe); 600 clears that in a single budget hour while still capping a
/// runaway loop well short of anything SEC's fair-access policy would refuse.
const SEC_BUDGET: i64 = 600;

/// Company-leadership refresh (Phase 14). Leadership changes slowly, so the
/// roster is rebuilt monthly rather than on the weekly SEC cadence above. Each
/// sweep parses at most this many of a company's most recent ownership filings
/// (one HTTP request each).
///
/// At 10 filings per sweep, a first-time backfill of one company captures the
/// recently-filing officers and board (a Form 3/4/5 from each active director
/// + a handful of officer trades), while the steady-state monthly refresh is
/// tiny — only the filings since `leadership_synced_at` are pulled. A higher
/// chunk eagerly grabs more history but, multiplied by the few-hundred-stock
/// universe, churned through the SEC endpoint's hourly budget during the
/// Phase 14 backfill (markets-closed weekend burn). Smaller chunks spread
/// that initial fill across more sweeps without changing the eventual roster.
const LEADERSHIP_STALE_SECS: i64 = 30 * 24 * 3600;
const LEADERSHIP_MAX_FILINGS: usize = 10;

/// Dividend history refresh (Phase 26). Declared dividends are confirmed and
/// stable once an ex-date passes, so the data changes slowly: a stock pays out
/// at most a handful of times a year. A weekly cadence is plenty to land each
/// new payment within a few days while keeping the Yahoo budget light.
const DIVIDENDS_INTERVAL_SECS: i64 = 24 * 3600;
const DIVIDENDS_STALE_SECS: i64 = 7 * 24 * 3600;

/// ETF fund metadata refresh (Phase 28). The Yahoo `quoteSummary` figures —
/// expense ratio, distribution yield, NAV, inception, category, fund family,
/// strategy summary — change slowly (the prospectus is updated once a year),
/// so a monthly staleness window is enough. The due-check is daily so a
/// newly-stale ETF is picked up within a day, but the steady-state cost is
/// one request per ETF per month.
const FUND_METADATA_INTERVAL_SECS: i64 = 24 * 3600;
const FUND_METADATA_STALE_SECS: i64 = 30 * 24 * 3600;

/// Earnings calendar refresh (Phase 25). Yahoo's `calendarEvents.earnings`
/// rolls forward one quarter at a time as each print lands, and a stock's
/// next date is irrelevant before it shifts — so a monthly cadence is
/// enough to land each new date within a few weeks of when Yahoo learns it.
/// Daily due-check; one request per stock through the shared `yahoo`
/// `EndpointGuard`. The whole sweep also re-runs once the stored
/// `next_earnings_at` passes, so a missed roll-forward never sits stale.
const EARNINGS_INTERVAL_SECS: i64 = 24 * 3600;
const EARNINGS_STALE_SECS: i64 = 30 * 24 * 3600;

/// Asset-profile refresh (Phase 15). A company's sector and industry
/// classification changes only on a structural shift (a reverse-merger or a
/// reclassification by the index provider), so a monthly cadence is plenty.
/// Daily due-check; one request per stock through the shared `yahoo`
/// `EndpointGuard`. ~512 stocks × monthly = ~17 requests/day in steady
/// state, well below the 1000/hr budget.
const ASSET_PROFILE_INTERVAL_SECS: i64 = 24 * 3600;
const ASSET_PROFILE_STALE_SECS: i64 = 30 * 24 * 3600;

/// Spawn the scheduler. The returned handle is normally dropped: dropping it
/// detaches the task, which then runs for the lifetime of the process.
pub fn spawn(pool: SqlitePool, config: Arc<Config>, hub: Arc<Hub>) -> JoinHandle<()> {
    tokio::spawn(async move {
        tracing::info!("[scheduler] started");

        if let Err(e) = reset_states_on_boot(&pool).await {
            tracing::warn!("[scheduler] reset states: {e}");
        }
        if let Err(e) = register_endpoints(&pool).await {
            tracing::warn!("[scheduler] register endpoints: {e}");
        }

        // History comes from Yahoo now (no API key needed), so the seed and
        // incremental refresh always run.
        if let Err(e) = run_boot_seed(&pool, &config, &hub).await {
            tracing::warn!("[scheduler] boot seed: {e:#}");
        }

        // SEC's fair-access policy asks consumers to identify themselves; with
        // no contact email configured the SEC job stays off rather than make
        // anonymous requests.
        let sec_enabled = !config.sec_contact_email.is_empty();
        if !sec_enabled {
            tracing::warn!(
                "[scheduler] SEC_CONTACT_EMAIL unset: SEC fundamentals & filings job disabled"
            );
        } else if let Err(e) = schedule_next(&pool, "sec", now_ms()).await {
            // Bring the SEC job forward to the first tick. The sweep is
            // resumable and cheap when nothing is stale, so running it on
            // each boot is harmless — and it means a deploy that introduces
            // new SEC-backed data (e.g. the Phase 18 ETF profiles) backfills
            // within a tick instead of waiting out the ~24h interval.
            tracing::warn!("[scheduler] bring sec job forward: {e}");
        }

        // Dividends job (Phase 26): bring it forward the same way the SEC job
        // is, so a deploy adding the table backfills the universe within a
        // tick rather than waiting out the daily interval. The sweep is
        // resumable and the no-stale fast path is free.
        if let Err(e) = schedule_next(&pool, "dividends", now_ms()).await {
            tracing::warn!("[scheduler] bring dividends job forward: {e}");
        }

        // Fund-metadata job (Phase 28): same pattern as the SEC and dividends
        // jobs — bring it forward to the first tick so a deploy that adds the
        // table backfills the ETF universe within a tick rather than waiting
        // out the daily interval. Resumable; the no-stale fast path is free.
        if let Err(e) = schedule_next(&pool, "fund_metadata", now_ms()).await {
            tracing::warn!("[scheduler] bring fund_metadata job forward: {e}");
        }

        // Earnings calendar job (Phase 25): same bring-forward pattern, so a
        // deploy adding the columns backfills the stock universe within a
        // tick rather than the daily interval. Resumable; no-stale fast path
        // is free.
        if let Err(e) = schedule_next(&pool, "earnings_calendar", now_ms()).await {
            tracing::warn!("[scheduler] bring earnings_calendar job forward: {e}");
        }

        // Asset profile job (Phase 15): same bring-forward pattern. A fresh
        // deploy populates each curated stock's sector and industry within
        // hours rather than the daily interval; resumable, the no-stale fast
        // path is free.
        if let Err(e) = schedule_next(&pool, "asset_profile", now_ms()).await {
            tracing::warn!("[scheduler] bring asset_profile job forward: {e}");
        }

        // Prune's last-run time is loop-local: a restart simply re-prunes once,
        // which is harmless (local-only DELETEs, no network).
        let mut last_prune: Option<i64> = None;
        // The session last broadcast, so a transition (e.g. into after-hours)
        // is pushed to connected clients exactly once.
        let mut last_session: Option<market::Session> = None;
        loop {
            // Broadcast a market-session change so open pages update their pill.
            let session = market::session_at(chrono::Utc::now());
            if last_session != Some(session) {
                if let Some(prev) = last_session {
                    tracing::info!(
                        "[scheduler] market {} -> {}",
                        prev.as_str(),
                        session.as_str()
                    );
                }
                hub.publish(StreamEvent::Market {
                    session: session.as_str().to_string(),
                });
                last_session = Some(session);
            }

            match is_due(&pool, "history", now_ms()).await {
                Ok(true) => {
                    if let Err(e) = run_history(&pool, &config, &hub).await {
                        tracing::warn!("[scheduler] history: {e:#}");
                    }
                }
                Ok(false) => {}
                Err(e) => tracing::warn!("[scheduler] history due-check: {e}"),
            }

            if sec_enabled {
                match is_due(&pool, "sec", now_ms()).await {
                    Ok(true) => {
                        if let Err(e) = run_sec(&pool, &config, &hub).await {
                            tracing::warn!("[scheduler] sec: {e:#}");
                        }
                    }
                    Ok(false) => {}
                    Err(e) => tracing::warn!("[scheduler] sec due-check: {e}"),
                }
            }

            // Dividend payouts (Phase 26 + 28): sweep stocks AND ETFs whose
            // dividend / distribution history has gone stale (weekly). Runs
            // only when Yahoo is reachable; gated nowhere else — declared
            // events drift after the ex-date passes, so a fresh pull every
            // week is enough.
            match is_due(&pool, "dividends", now_ms()).await {
                Ok(true) => {
                    if let Err(e) = run_dividends(&pool, &config, &hub).await {
                        tracing::warn!("[scheduler] dividends: {e:#}");
                    }
                }
                Ok(false) => {}
                Err(e) => tracing::warn!("[scheduler] dividends due-check: {e}"),
            }

            // ETF fund metadata (Phase 28): sweep ETFs whose Yahoo
            // quoteSummary snapshot has gone stale (monthly). Same Yahoo
            // guard as intraday + daily-close + dividends; expense / yield /
            // NAV / strategy change rarely, so this is cheap in steady state.
            match is_due(&pool, "fund_metadata", now_ms()).await {
                Ok(true) => {
                    if let Err(e) = run_fund_metadata(&pool, &config, &hub).await {
                        tracing::warn!("[scheduler] fund_metadata: {e:#}");
                    }
                }
                Ok(false) => {}
                Err(e) => tracing::warn!("[scheduler] fund_metadata due-check: {e}"),
            }

            // Earnings calendar (Phase 25): sweep stocks whose next-expected
            // earnings date has gone stale or already passed. Same Yahoo
            // guard; one request per stock per month in steady state.
            match is_due(&pool, "earnings_calendar", now_ms()).await {
                Ok(true) => {
                    if let Err(e) = run_earnings_calendar(&pool, &config, &hub).await {
                        tracing::warn!("[scheduler] earnings_calendar: {e:#}");
                    }
                }
                Ok(false) => {}
                Err(e) => tracing::warn!("[scheduler] earnings_calendar due-check: {e}"),
            }

            // Asset profile (Phase 15): sweep stocks whose sector / industry
            // classification has gone stale (monthly). Same Yahoo guard; one
            // request per stock per month in steady state.
            match is_due(&pool, "asset_profile", now_ms()).await {
                Ok(true) => {
                    if let Err(e) = run_asset_profile(&pool, &config, &hub).await {
                        tracing::warn!("[scheduler] asset_profile: {e:#}");
                    }
                }
                Ok(false) => {}
                Err(e) => tracing::warn!("[scheduler] asset_profile due-check: {e}"),
            }

            // Intraday quotes: demand-driven (only symbols a browser is
            // viewing). Inside a trading session every viewed symbol is
            // polled; outside it, only viewed futures, which trade nearly
            // around the clock. Does no network work when neither applies.
            if let Err(e) = run_intraday(&pool, &config, &hub, session).await {
                tracing::warn!("[scheduler] intraday: {e:#}");
            }

            if let Err(e) = run_daily_close_if_due(&pool, &config, &hub).await {
                tracing::warn!("[scheduler] daily close: {e:#}");
            }

            if let Err(e) = run_prune_if_due(&pool, &mut last_prune, &hub).await {
                tracing::warn!("[scheduler] prune: {e:#}");
            }
            tokio::time::sleep(TICK).await;
        }
    })
}

/// Register the known data endpoints at startup, so the data-health page lists
/// each one — with its correct hourly budget — from the first boot, rather than
/// only once that endpoint's first request lazily creates its guard row. The
/// ids and budgets mirror how each job below constructs its `EndpointGuard`.
async fn register_endpoints(pool: &SqlitePool) -> anyhow::Result<()> {
    // Stooq was retired 2026-05-30 (Yahoo now serves history too). Drop its
    // stale guard row so it no longer lingers on the data-health page.
    sqlx::query("DELETE FROM endpoint_guard WHERE endpoint = 'stooq'")
        .execute(pool)
        .await?;
    EndpointGuard::with_budget(pool.clone(), "yahoo", YAHOO_BUDGET)
        .ensure_registered()
        .await?;
    EndpointGuard::with_budget(pool.clone(), "sec", SEC_BUDGET)
        .ensure_registered()
        .await?;
    Ok(())
}

/// Clear any `fetching` state left behind by a crash mid-job. The owning task
/// did not survive the restart, so the row must not stay stuck `fetching`.
async fn reset_states_on_boot(pool: &SqlitePool) -> sqlx::Result<()> {
    sqlx::query("UPDATE data_status SET state = 'idle', updated_at = ? WHERE state = 'fetching'")
        .bind(now_ms())
        .execute(pool)
        .await?;
    Ok(())
}

/// Run the first-run universe seed while `meta.seed_completed` is unset.
///
/// `seed::run` is itself idempotent (symbols are upserted) and resumable
/// (symbols that already hold history are skipped), so re-running it on each
/// boot until the seed completes is cheap. Afterwards the incremental history
/// job is deferred one full interval so it does not re-fetch, on this same
/// boot, the handful of symbols the seed just touched.
async fn run_boot_seed(pool: &SqlitePool, config: &Config, hub: &Hub) -> anyhow::Result<()> {
    if get_meta(pool, "seed_completed").await?.as_deref() == Some("1") {
        // The full backfill is done, but still reconcile the curated list each
        // boot so a CSV edit (symbols added to or dropped from the universe)
        // takes effect on deploy without a manual re-seed. This is local-only
        // and cheap; any newly-added symbol's history is backfilled by the
        // incremental `run_history` job (it picks up `history_synced_at IS NULL`).
        match seed::sync_universe(pool, config).await {
            Ok(r) if r.pruned > 0 => {
                tracing::info!("[scheduler] universe sync: {} symbols, {} pruned", r.total, r.pruned)
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("[scheduler] universe sync: {e:#}"),
        }
        // Run the history job promptly (next tick) rather than waiting out the
        // remaining interval from before the restart. A deploy that adds symbols
        // or ships a history fix should reconcile data soon, not up to
        // HISTORY_INTERVAL_SECS later: the job backfills freshly-added symbols
        // and self-heals any coarse stored series. It is a cheap no-op when
        // nothing needs work.
        schedule_next(pool, "history", now_ms()).await?;
        return Ok(());
    }
    tracing::info!("[scheduler] seed_completed unset: running first-run seed");

    let started = now_ms();
    mark_fetching(pool, "seed").await?;
    notify_health(hub);
    let yahoo = YahooProvider::new(providers::http::build_client(config));
    let t0 = Instant::now();
    let result = seed::run(pool, config, &yahoo).await;
    let dur = t0.elapsed().as_millis() as i64;

    match &result {
        Ok(()) => {
            let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM symbols WHERE is_seeded = 1")
                .fetch_one(pool)
                .await?;
            let with_history: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM symbols WHERE is_seeded = 1 AND history_last_date IS NOT NULL",
            )
            .fetch_one(pool)
            .await?;
            let detail = format!("{with_history}/{total} seeded symbols have history");
            log_fetch(pool, "seed", "yahoo", "ok", Some(&detail), Some(with_history), dur, started)
                .await?;
            mark_ok(pool, "seed", None).await?;
        }
        Err(e) => {
            let msg = format!("{e:#}");
            log_fetch(pool, "seed", "yahoo", "error", Some(&msg), None, dur, started).await?;
            mark_error(pool, "seed", &msg, None).await?;
        }
    }

    // Defer the first incremental refresh: it would otherwise re-fetch the same
    // still-stale symbols the seed just handled.
    schedule_next(pool, "history", now_ms() + HISTORY_INTERVAL_SECS * 1000).await?;
    notify_health(hub);
    result
}

/// Incremental daily-history refresh: re-fetch only the symbols whose stored
/// history has gone stale, asking Yahoo for the window since each symbol's
/// last stored bar. Paced and circuit-broken like the seed.
///
/// This is a backstop: on a normal trading day the `daily_close` job already
/// appends each symbol's bar (and stamps `history_synced_at`), so nothing is
/// stale here. It still earns its keep for symbols missed while the server was
/// down at the close, weekends, and freshly added tickers.
/// True when a stored daily series is actually coarser than daily, judged from
/// its *recent* density: `recent_bars` is the number of bars in the trailing 90
/// days of the series, which is ~63 for a genuine daily series (trading days),
/// ~13 for weekly and ~3 for monthly — so under 30 means coarser than daily.
/// Only judged once the series spans more than 180 days, so a young-but-daily
/// symbol is not flagged for merely having a short history. Mirrors the SQL test
/// in [`run_history`]'s selection query.
fn is_coarser_than_daily(first: &Option<String>, last: &Option<String>, recent_bars: i64) -> bool {
    let (Some(f), Some(l)) = (first, last) else {
        return false;
    };
    if f == l {
        return false;
    }
    let (Ok(fd), Ok(ld)) = (
        chrono::NaiveDate::parse_from_str(f, "%Y-%m-%d"),
        chrono::NaiveDate::parse_from_str(l, "%Y-%m-%d"),
    ) else {
        return false;
    };
    (ld - fd).num_days() > 180 && recent_bars < 30
}

async fn run_history(pool: &SqlitePool, config: &Config, hub: &Hub) -> anyhow::Result<()> {
    let started = now_ms();
    let next = started + HISTORY_INTERVAL_SECS * 1000;
    mark_fetching(pool, "history").await?;
    notify_health(hub);

    // Every symbol we track, curated or user-added, is eligible: a symbol
    // added through the Phase 9 add-symbol flow needs its history backfilled
    // exactly as a seeded one does. (The seed itself stays curated-list-only;
    // it is the only job that keys on `is_seeded`.) Futures are included now —
    // Yahoo serves `=F` daily history, unlike the Stooq source this replaced.
    // A symbol is refreshed when any of these hold:
    //   - its history has gone stale (the normal incremental case);
    //   - it holds a single stored bar (`history_first_date = history_last_date`)
    //     — a symbol added after the initial seed whose `history_last_date` was
    //     stamped by a `daily_close` snapshot before any range=max backfill ran,
    //     so the incremental window can never reach its deep history;
    //   - its stored history is coarser than daily. Yahoo silently downsamples
    //     `interval=1d` to weekly / monthly bars when a range spans many years
    //     (see `YahooProvider::is_downsampled`), so a symbol first backfilled via
    //     range=max can hold monthly bars. Detect it by *recent* density — the
    //     bar count in the trailing 90 days of the series: a daily series carries
    //     ~63 (trading days), a weekly one ~13, a monthly one ~3, so fewer than
    //     30 means coarser than daily. The trailing window (rather than whole-
    //     span density) is what keeps a deep symbol with a sparse pre-modern tail
    //     — e.g. ^SPX, daily for decades but with century-old gaps — from being
    //     misread as coarse. Only judged once a symbol spans over 180 days, so a
    //     young-but-daily symbol is not flagged for its short history.
    // The last two are selected regardless of staleness so the heal is not gated
    // behind the stale cutoff; both are repaired with a full daily re-fetch (the
    // `deep` branch below). `recent_bars` rides along so the same test can pick
    // the fetch window without a second query.
    let cutoff = started - HISTORY_STALE_SECS * 1000;
    let stale: Vec<(String, Option<String>, Option<String>, i64)> = sqlx::query_as(
        "SELECT s.ticker, s.history_first_date, s.history_last_date, \
                (SELECT COUNT(*) FROM daily_prices d WHERE d.ticker = s.ticker \
                   AND d.d >= date(s.history_last_date, '-90 days')) AS recent_bars \
         FROM symbols s \
         WHERE s.history_synced_at IS NULL OR s.history_synced_at < ? \
            OR s.history_first_date = s.history_last_date \
            OR (s.history_last_date IS NOT NULL \
                AND julianday(s.history_last_date) - julianday(s.history_first_date) > 180 \
                AND (SELECT COUNT(*) FROM daily_prices d WHERE d.ticker = s.ticker \
                       AND d.d >= date(s.history_last_date, '-90 days')) < 30) \
         ORDER BY s.ticker",
    )
    .bind(cutoff)
    .fetch_all(pool)
    .await?;

    if stale.is_empty() {
        log_fetch(pool, "history", "yahoo", "ok", Some("no stale symbols"), Some(0), 0, started)
            .await?;
        mark_ok(pool, "history", Some(next)).await?;
        notify_health(hub);
        return Ok(());
    }
    tracing::info!("[scheduler] history: refreshing {} stale symbols", stale.len());

    let yahoo = YahooProvider::new(providers::http::build_client(config));
    let t0 = Instant::now();

    // Route every request through the persistent endpoint guard: it paces the
    // loop and refuses requests once the breaker opens or the hourly budget is
    // spent, so the job stops cleanly instead of hammering a guarded endpoint.
    // History shares the `yahoo` guard with live quotes, so build it with the
    // same budget rather than the 200-default `new` ceiling.
    let guard = EndpointGuard::with_budget(pool.clone(), "yahoo", YAHOO_BUDGET);
    let mut ok = 0usize;
    let mut total_bars = 0i64;
    let mut stopped: Option<String> = None;

    for (ticker, first_date, last_date, recent_bars) in &stale {
        match guard.acquire().await? {
            Permit::Granted => {}
            Permit::Denied(why) => {
                stopped = Some(why);
                break;
            }
        }
        // Decide the fetch window. A symbol with no full daily history yet —
        // none stored, a single bar, or a coarser-than-daily series — needs a
        // deep re-fetch, so ask for range=max (`None`); the provider re-asks a
        // bounded window when Yahoo downsamples it. Everything else just wants
        // the incremental window since its last stored bar.
        let single_bar = first_date.is_some() && first_date == last_date;
        let coarse = is_coarser_than_daily(first_date, last_date, *recent_bars);
        let deep = last_date.is_none() || single_bar || coarse;
        let since = if deep { None } else { last_date.as_deref() };
        match yahoo.daily(ticker, since).await {
            Ok(bars) if !bars.is_empty() => {
                guard.record_success().await?;
                // A coarse series is replaced, not merged: drop its weekly /
                // monthly bars so the fresh daily ones do not interleave with
                // leftover old-granularity bars. Done only once we have new
                // bars in hand, so the symbol is never left empty.
                if coarse {
                    sqlx::query("DELETE FROM daily_prices WHERE ticker = ?")
                        .bind(ticker)
                        .execute(pool)
                        .await?;
                }
                total_bars += bars.len() as i64;
                seed::store_daily(pool, ticker, &bars).await?;
                ok += 1;
            }
            Ok(_) => {
                // A valid but empty response: the request succeeded and the
                // endpoint simply had nothing new. Stamp the symbol checked so
                // it is not re-fetched until it goes stale again.
                guard.record_success().await?;
                mark_history_checked(pool, ticker).await?;
            }
            Err(e) => {
                guard.record_failure(&e).await?;
                tracing::warn!("[scheduler] history {ticker} failed: {e:#}");
            }
        }
    }

    let dur = t0.elapsed().as_millis() as i64;
    match stopped {
        Some(why) => {
            // The guard cut the run short (breaker open or hourly budget
            // spent). That is the guard doing its job, not a failure of this
            // job: record it as `skipped` and let the next cycle retry.
            let detail = format!(
                "stopped early ({why}); {ok}/{} refreshed, {total_bars} bars",
                stale.len()
            );
            tracing::warn!("[scheduler] history: {detail}");
            log_fetch(pool, "history", "yahoo", "skipped", Some(&detail), Some(total_bars), dur, started)
                .await?;
            // Work remains, so retry soon rather than after the full interval.
            mark_ok(pool, "history", Some(started + HISTORY_RETRY_SECS * 1000)).await?;
        }
        None => {
            let detail = format!("{ok}/{} symbols refreshed, {total_bars} bars", stale.len());
            tracing::info!("[scheduler] history: {detail}");
            log_fetch(pool, "history", "yahoo", "ok", Some(&detail), Some(total_bars), dur, started)
                .await?;
            mark_ok(pool, "history", Some(next)).await?;
        }
    }
    notify_health(hub);
    Ok(())
}

/// Demand-driven intraday quote refresh.
///
/// Polls Yahoo only for the symbols a browser is currently viewing (the stream
/// hub's interest registry). With nobody watching, `hub.viewed()` is empty and
/// this returns at once having done no network work: the user's hard rule is
/// to poll only what is on screen.
///
/// Which viewed symbols are polled depends on the session. Inside any trading
/// session (pre, regular, post) every viewed symbol is fair game. Outside it,
/// only viewed futures are polled: index futures and commodities trade nearly
/// around the clock, while indexes, stocks and ETFs sit frozen until the next
/// session, so polling them off-hours would only re-fetch a flat quote. This
/// is what keeps the dashboard's commodity cards live overnight.
///
/// A clean run is recorded only in `data_status` (plus each `quotes.fetched_at`
/// row); a `fetch_log` row is written only for a notable run, an error or a
/// guard stop, so the minute-cadence job does not bury the log.
async fn run_intraday(
    pool: &SqlitePool,
    config: &Config,
    hub: &Hub,
    session: market::Session,
) -> anyhow::Result<()> {
    let viewed = hub.viewed();
    if viewed.is_empty() {
        return Ok(());
    }
    // Inside a session, poll every viewed symbol; outside it, only the futures.
    let targets: Vec<String> = if session.is_open() {
        viewed
    } else {
        let futures: HashSet<String> =
            sqlx::query_scalar("SELECT ticker FROM symbols WHERE kind = 'future'")
                .fetch_all(pool)
                .await?
                .into_iter()
                .collect();
        viewed.into_iter().filter(|t| futures.contains(t)).collect()
    };
    if targets.is_empty() {
        return Ok(());
    }
    let started = now_ms();
    mark_fetching(pool, "intraday").await?;
    notify_health(hub);

    let yahoo = YahooProvider::new(providers::http::build_client(config));
    let guard = EndpointGuard::with_budget(pool.clone(), "yahoo", YAHOO_BUDGET);
    let t0 = Instant::now();

    let mut ok = 0i64;
    let mut errors = 0i64;
    let mut stopped: Option<String> = None;

    for ticker in &targets {
        match guard.acquire().await? {
            Permit::Granted => {}
            Permit::Denied(why) => {
                stopped = Some(why);
                break;
            }
        }
        match yahoo.quote(ticker).await {
            Ok(data) => {
                guard.record_success().await?;
                store_quote(pool, ticker, &data.quote).await?;
                if !data.bars.is_empty() {
                    store_intraday(pool, ticker, &data.bars).await?;
                }
                hub.publish(StreamEvent::Quote(QuoteUpdate::new(
                    ticker.clone(),
                    data.quote.price,
                    data.quote.prev_close,
                    data.quote.market_state.clone(),
                )));
                ok += 1;
            }
            Err(e) => {
                guard.record_failure(&e).await?;
                errors += 1;
                tracing::warn!("[scheduler] intraday {ticker} failed: {e:#}");
            }
        }
    }

    let dur = t0.elapsed().as_millis() as i64;
    if let Some(why) = stopped {
        let detail = format!("stopped early ({why}); {ok} ok, {errors} errors");
        tracing::warn!("[scheduler] intraday: {detail}");
        log_fetch(pool, "intraday", "yahoo", "skipped", Some(&detail), Some(ok), dur, started)
            .await?;
        mark_ok(pool, "intraday", None).await?;
    } else if ok == 0 && errors > 0 {
        let detail = format!("all {errors} quotes failed");
        log_fetch(pool, "intraday", "yahoo", "error", Some(&detail), Some(0), dur, started).await?;
        mark_error(pool, "intraday", &detail, None).await?;
    } else {
        if errors > 0 {
            let detail = format!("{ok} ok, {errors} errors");
            log_fetch(pool, "intraday", "yahoo", "ok", Some(&detail), Some(ok), dur, started)
                .await?;
        }
        mark_ok(pool, "intraday", None).await?;
    }
    notify_health(hub);
    Ok(())
}

/// Build a daily OHLCV bar from a closing quote. `close` is the quote's last
/// price; open/high/low fall back to the close when Yahoo's `meta` left them
/// null (some indexes carry only a level), and volume defaults to 0.
fn daily_bar_from_quote(date: &str, q: &Quote) -> DailyBar {
    let close = q.price;
    DailyBar {
        d: date.to_string(),
        open: q.open.unwrap_or(close),
        high: q.day_high.unwrap_or(close),
        low: q.day_low.unwrap_or(close),
        close,
        volume: q.volume.unwrap_or(0),
    }
}

/// Once-a-day closing snapshot of the whole universe.
///
/// Shortly after the regular session closes (>= 16:05 ET on a weekday), fetch
/// a Yahoo quote for every seeded symbol so each one carries a same-day close
/// even if nobody viewed it. Keyed on the ET trading date in `meta` so it runs
/// exactly once per day; a guard stop leaves the date unset so the next cycle
/// finishes the rest.
async fn run_daily_close_if_due(
    pool: &SqlitePool,
    config: &Config,
    hub: &Hub,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now();
    if !market::is_et_weekday(now) || !market::after_close(now) {
        return Ok(());
    }
    let date = market::et_date(now);
    if get_meta(pool, "daily_close_date").await?.as_deref() == Some(date.as_str()) {
        return Ok(());
    }

    // The whole tracked universe, curated and user-added alike, so every
    // symbol carries a same-day close even if nobody viewed it.
    let symbols: Vec<String> =
        sqlx::query_scalar("SELECT ticker FROM symbols ORDER BY ticker")
            .fetch_all(pool)
            .await?;
    if symbols.is_empty() {
        return Ok(());
    }
    tracing::info!("[scheduler] daily close: snapshotting {} symbols for {date}", symbols.len());

    let started = now_ms();
    mark_fetching(pool, "daily_close").await?;
    notify_health(hub);
    let yahoo = YahooProvider::new(providers::http::build_client(config));
    let guard = EndpointGuard::with_budget(pool.clone(), "yahoo", YAHOO_BUDGET);
    let t0 = Instant::now();

    let mut ok = 0i64;
    let mut errors = 0i64;
    let mut stopped: Option<String> = None;

    for ticker in &symbols {
        match guard.acquire().await? {
            Permit::Granted => {}
            Permit::Denied(why) => {
                stopped = Some(why);
                break;
            }
        }
        match yahoo.quote(ticker).await {
            Ok(data) => {
                guard.record_success().await?;
                store_quote(pool, ticker, &data.quote).await?;
                if !data.bars.is_empty() {
                    store_intraday(pool, ticker, &data.bars).await?;
                }
                // Append today's daily bar from the same quote (no extra
                // request), so `daily_prices` stays current with Yahoo as the
                // sole price source — no separate per-symbol history sweep.
                seed::store_daily(pool, ticker, &[daily_bar_from_quote(&date, &data.quote)])
                    .await?;
                hub.publish(StreamEvent::Quote(QuoteUpdate::new(
                    ticker.clone(),
                    data.quote.price,
                    data.quote.prev_close,
                    data.quote.market_state.clone(),
                )));
                ok += 1;
            }
            Err(e) => {
                guard.record_failure(&e).await?;
                errors += 1;
                tracing::warn!("[scheduler] daily close {ticker} failed: {e:#}");
            }
        }
    }

    let dur = t0.elapsed().as_millis() as i64;
    match stopped {
        Some(why) => {
            // Leave `daily_close_date` unset: the next cycle retries and
            // finishes the symbols this run did not reach.
            let detail = format!("stopped early ({why}); {ok}/{} done", symbols.len());
            tracing::warn!("[scheduler] daily close: {detail}");
            log_fetch(pool, "daily_close", "yahoo", "skipped", Some(&detail), Some(ok), dur, started)
                .await?;
            mark_ok(pool, "daily_close", None).await?;
        }
        None => {
            set_meta(pool, "daily_close_date", &date).await?;
            let detail = format!("{ok}/{} symbols, {errors} errors", symbols.len());
            tracing::info!("[scheduler] daily close: {detail}");
            log_fetch(pool, "daily_close", "yahoo", "ok", Some(&detail), Some(ok), dur, started)
                .await?;
            mark_ok(pool, "daily_close", None).await?;
        }
    }
    notify_health(hub);
    Ok(())
}

/// SEC fundamentals, filings & ETF fund-profile sweep.
///
/// On the first run (and whenever new symbols appear) two bulk ticker-map
/// fetches fill in CIKs — `company_tickers.json` for stocks,
/// `company_tickers_mf.json` for ETFs. Then every symbol whose SEC data has
/// gone stale is refreshed:
///  - a stock's XBRL `companyfacts` into `fundamentals`, its submission
///    history into `filings`;
///  - an ETF's latest N-PORT into `fund_profiles` + `fund_holdings`, its
///    filing history into `filings`. A physical-commodity grantor trust files
///    no N-PORT, so its AUM comes from `companyfacts` instead.
///  - a stock's officer/board roster into `leadership`, parsed from its recent
///    Form 3/4/5 ownership filings (Phase 14, on a slower monthly cadence).
/// Indexes are skipped; they do not file with the SEC.
///
/// Resumable like the history job: each symbol's sync timestamps are stamped
/// only on a successful fetch, so a guard stop simply leaves the rest for the
/// next cycle.
async fn run_sec(pool: &SqlitePool, config: &Config, hub: &Hub) -> anyhow::Result<()> {
    let started = now_ms();
    let next = started + SEC_INTERVAL_SECS * 1000;
    mark_fetching(pool, "sec").await?;
    notify_health(hub);

    let sec = SecProvider::new(providers::http::build_sec_client(config));
    let guard = EndpointGuard::with_budget(pool.clone(), sec.name(), SEC_BUDGET);
    let t0 = Instant::now();

    // 1. Stock CIK resolution. One bulk call maps the whole market; only
    //    needed while some stock still lacks a CIK.
    let missing: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM symbols WHERE kind = 'stock' AND cik IS NULL",
    )
    .fetch_one(pool)
    .await?;
    if missing > 0 {
        match guard.acquire().await? {
            Permit::Granted => match sec.cik_map().await {
                Ok(map) => {
                    guard.record_success().await?;
                    let resolved = resolve_ciks(pool, &map).await?;
                    tracing::info!("[scheduler] sec: resolved {resolved}/{missing} CIKs");
                }
                Err(e) => {
                    guard.record_failure(&e).await?;
                    let msg = format!("CIK map: {e:#}");
                    let dur = t0.elapsed().as_millis() as i64;
                    log_fetch(pool, "sec", "sec", "error", Some(&msg), None, dur, started).await?;
                    mark_error(pool, "sec", &msg, Some(next)).await?;
                    notify_health(hub);
                    return Ok(());
                }
            },
            Permit::Denied(why) => {
                let dur = t0.elapsed().as_millis() as i64;
                let detail = format!("stopped before CIK map ({why})");
                log_fetch(pool, "sec", "sec", "skipped", Some(&detail), Some(0), dur, started)
                    .await?;
                mark_ok(pool, "sec", Some(next)).await?;
                notify_health(hub);
                return Ok(());
            }
        }
    }

    // 1b. ETF fund-CIK resolution from the mutual-fund ticker map. Best-effort:
    //     a guard stop or error here only leaves the ETFs for a later cycle,
    //     rather than aborting the stock sweep below.
    let etfs_missing: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM symbols WHERE kind = 'etf' AND cik IS NULL",
    )
    .fetch_one(pool)
    .await?;
    if etfs_missing > 0 {
        match guard.acquire().await? {
            Permit::Granted => match sec.fund_ticker_map().await {
                Ok(map) => {
                    guard.record_success().await?;
                    let resolved = resolve_fund_ciks(pool, &map).await?;
                    tracing::info!("[scheduler] sec: resolved {resolved}/{etfs_missing} fund CIKs");
                }
                Err(e) => {
                    guard.record_failure(&e).await?;
                    tracing::warn!("[scheduler] sec fund CIK map: {e:#}");
                }
            },
            Permit::Denied(why) => {
                tracing::info!("[scheduler] sec: fund CIK map skipped ({why})");
            }
        }
    }

    // 2. Stale sweep. A symbol is due when one of its SEC timestamps is unset
    //    or older than the staleness window.
    let cutoff = started - SEC_STALE_SECS * 1000;
    let stale_stocks: Vec<(String, String, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT ticker, cik, fundamentals_synced_at, filings_synced_at FROM symbols \
         WHERE kind = 'stock' AND cik IS NOT NULL \
           AND (fundamentals_synced_at IS NULL OR filings_synced_at IS NULL \
                OR fundamentals_synced_at < ? OR filings_synced_at < ?) \
         ORDER BY ticker",
    )
    .bind(cutoff)
    .bind(cutoff)
    .fetch_all(pool)
    .await?;
    let stale_etfs: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT ticker, cik, series_id FROM symbols \
         WHERE kind = 'etf' AND cik IS NOT NULL \
           AND (fund_synced_at IS NULL OR fund_synced_at < ?) \
         ORDER BY ticker",
    )
    .bind(cutoff)
    .fetch_all(pool)
    .await?;
    // Leadership has its own, longer staleness window (see LEADERSHIP_STALE_SECS).
    let leadership_cutoff = started - LEADERSHIP_STALE_SECS * 1000;
    let stale_leadership: Vec<(String, String, Option<i64>)> = sqlx::query_as(
        "SELECT ticker, cik, leadership_synced_at FROM symbols \
         WHERE kind = 'stock' AND cik IS NOT NULL \
           AND (leadership_synced_at IS NULL OR leadership_synced_at < ?) \
         ORDER BY ticker",
    )
    .bind(leadership_cutoff)
    .fetch_all(pool)
    .await?;

    if stale_stocks.is_empty() && stale_etfs.is_empty() && stale_leadership.is_empty() {
        log_fetch(pool, "sec", "sec", "ok", Some("no stale companies or funds"), Some(0), 0, started)
            .await?;
        mark_ok(pool, "sec", Some(next)).await?;
        notify_health(hub);
        return Ok(());
    }
    tracing::info!(
        "[scheduler] sec: refreshing {} companies, {} funds, {} rosters",
        stale_stocks.len(),
        stale_etfs.len(),
        stale_leadership.len()
    );

    // A metric is due when its timestamp is unset or past the cutoff.
    let due = |at: Option<i64>| at.map_or(true, |t| t < cutoff);
    let mut funds_ok = 0i64;
    let mut filings_ok = 0i64;
    let mut etfs_ok = 0i64;
    let mut leaders_ok = 0i64;
    let mut errors = 0i64;
    let mut stopped: Option<String> = None;

    'stocks: for (ticker, cik, f_at, fl_at) in &stale_stocks {
        if due(*f_at) {
            match guard.acquire().await? {
                Permit::Granted => match sec.facts(cik).await {
                    Ok(facts) => {
                        guard.record_success().await?;
                        store_fundamentals(pool, ticker, &facts).await?;
                        mark_sec_synced(pool, ticker, "fundamentals_synced_at").await?;
                        funds_ok += 1;
                    }
                    Err(e) => {
                        guard.record_failure(&e).await?;
                        errors += 1;
                        tracing::warn!("[scheduler] sec facts {ticker} failed: {e:#}");
                    }
                },
                Permit::Denied(why) => {
                    stopped = Some(why);
                    break 'stocks;
                }
            }
        }
        if due(*fl_at) {
            match guard.acquire().await? {
                Permit::Granted => match sec.filings(cik).await {
                    Ok(filings) => {
                        guard.record_success().await?;
                        store_filings(pool, ticker, &filings).await?;
                        mark_sec_synced(pool, ticker, "filings_synced_at").await?;
                        filings_ok += 1;
                    }
                    Err(e) => {
                        guard.record_failure(&e).await?;
                        errors += 1;
                        tracing::warn!("[scheduler] sec filings {ticker} failed: {e:#}");
                    }
                },
                Permit::Denied(why) => {
                    stopped = Some(why);
                    break 'stocks;
                }
            }
        }
    }

    // 3. ETF fund-profile sweep, sharing the guard and the early-exit. Skipped
    //    wholesale if the stock sweep above already hit a guard stop.
    if stopped.is_none() {
        'funds: for (ticker, cik, series_id) in &stale_etfs {
            let id = FundId {
                cik: cik.clone(),
                series_id: series_id.clone(),
            };
            // 3a. The filing list, and what it says about the fund's shape.
            let shape = match guard.acquire().await? {
                Permit::Granted => match sec.fund_filings(&id).await {
                    Ok(ff) => {
                        guard.record_success().await?;
                        store_filings(pool, ticker, &ff.filings).await?;
                        ff.shape
                    }
                    Err(e) => {
                        guard.record_failure(&e).await?;
                        errors += 1;
                        tracing::warn!("[scheduler] sec fund_filings {ticker} failed: {e:#}");
                        continue 'funds;
                    }
                },
                Permit::Denied(why) => {
                    stopped = Some(why);
                    break 'funds;
                }
            };
            // 3b. Holdings from N-PORT, or AUM for a commodity trust.
            match shape {
                FundShape::Portfolio { nport_href } => match guard.acquire().await? {
                    Permit::Granted => match sec.fund_portfolio(&nport_href).await {
                        Ok(portfolio) => {
                            guard.record_success().await?;
                            store_fund_portfolio(pool, ticker, &portfolio).await?;
                            mark_fund_synced(pool, ticker).await?;
                            etfs_ok += 1;
                        }
                        Err(e) => {
                            guard.record_failure(&e).await?;
                            errors += 1;
                            tracing::warn!("[scheduler] sec fund_portfolio {ticker} failed: {e:#}");
                        }
                    },
                    Permit::Denied(why) => {
                        stopped = Some(why);
                        break 'funds;
                    }
                },
                FundShape::CommodityTrust => match guard.acquire().await? {
                    Permit::Granted => match sec.fund_aum(cik).await {
                        Ok(aum) => {
                            guard.record_success().await?;
                            store_fund_commodity(pool, ticker, aum).await?;
                            mark_fund_synced(pool, ticker).await?;
                            etfs_ok += 1;
                        }
                        Err(e) => {
                            guard.record_failure(&e).await?;
                            errors += 1;
                            tracing::warn!("[scheduler] sec fund_aum {ticker} failed: {e:#}");
                        }
                    },
                    Permit::Denied(why) => {
                        stopped = Some(why);
                        break 'funds;
                    }
                },
                FundShape::Unknown => {
                    // The filing list synced but there is no portfolio to
                    // record. Stamp it so it is not retried until next stale.
                    mark_fund_synced(pool, ticker).await?;
                    etfs_ok += 1;
                }
            }
        }
    }

    // 4. Leadership sweep (Phase 14): for each stale stock, parse a window of
    //    its recent Form 3/4/5 ownership filings into the officer/board roster.
    //    Shares the guard and the early-exit; skipped wholesale once the sweeps
    //    above have already hit a guard stop.
    if stopped.is_none() {
        'leaders: for (ticker, cik, lead_at) in &stale_leadership {
            // The company's recent ownership filings, newest first.
            let index = match guard.acquire().await? {
                Permit::Granted => match sec.ownership_index(cik).await {
                    Ok(idx) => {
                        guard.record_success().await?;
                        idx
                    }
                    Err(e) => {
                        guard.record_failure(&e).await?;
                        errors += 1;
                        tracing::warn!("[scheduler] sec ownership_index {ticker} failed: {e:#}");
                        continue 'leaders;
                    }
                },
                Permit::Denied(why) => {
                    stopped = Some(why);
                    break 'leaders;
                }
            };
            // First sweep (no prior sync): the most recent filings. Later
            // sweeps: only filings since the last sync, with a few days' slack,
            // so the steady-state cost is a handful of requests per company.
            let since: Option<String> = lead_at.and_then(|ms| {
                chrono::DateTime::from_timestamp_millis(ms - 5 * 86_400_000)
                    .map(|dt| dt.format("%Y-%m-%d").to_string())
            });
            let to_parse: Vec<_> = index
                .into_iter()
                .filter(|f| since.as_deref().map_or(true, |s| f.filed_at.as_str() >= s))
                .take(LEADERSHIP_MAX_FILINGS)
                .collect();

            // Parse each filing's XML, keeping the directors and officers (a
            // filer who is only a >10% owner is not leadership and is dropped).
            let mut roster: Vec<(OwnershipPerson, String)> = Vec::new();
            for f in &to_parse {
                match guard.acquire().await? {
                    Permit::Granted => {
                        match sec.ownership_doc(cik, &f.accession, &f.primary_doc).await {
                            Ok(people) => {
                                guard.record_success().await?;
                                for p in people {
                                    if p.is_director || p.is_officer {
                                        roster.push((p, f.filed_at.clone()));
                                    }
                                }
                            }
                            Err(e) => {
                                guard.record_failure(&e).await?;
                                errors += 1;
                                tracing::warn!(
                                    "[scheduler] sec ownership_doc {ticker} failed: {e:#}"
                                );
                            }
                        }
                    }
                    Permit::Denied(why) => {
                        stopped = Some(why);
                        break;
                    }
                }
            }

            // Upsert what was gathered. A guard stop mid-company still stores
            // the partial roster (the upsert is idempotent) but leaves
            // `leadership_synced_at` unset so the next cycle finishes the rest.
            store_leadership(pool, ticker, &roster).await?;
            if stopped.is_some() {
                break 'leaders;
            }
            mark_sec_synced(pool, ticker, "leadership_synced_at").await?;
            leaders_ok += 1;
        }
    }

    let dur = t0.elapsed().as_millis() as i64;
    let counts = format!(
        "{funds_ok} fundamentals, {filings_ok} filings, {etfs_ok} fund profiles, \
         {leaders_ok} rosters, {errors} errors"
    );
    match stopped {
        Some(why) => {
            let detail = format!("stopped early ({why}); {counts}");
            tracing::warn!("[scheduler] sec: {detail}");
            log_fetch(pool, "sec", "sec", "skipped", Some(&detail), Some(funds_ok), dur, started)
                .await?;
        }
        None => {
            tracing::info!("[scheduler] sec: {counts}");
            log_fetch(pool, "sec", "sec", "ok", Some(&counts), Some(funds_ok), dur, started)
                .await?;
        }
    }
    mark_ok(pool, "sec", Some(next)).await?;
    notify_health(hub);
    Ok(())
}

/// Dividend payout sweep (Phase 26).
///
/// For every stock whose dividend history is stale, ask Yahoo for the last
/// five years of declared dividends and upsert them. Stocks only — ETFs,
/// indexes and futures do not pay regular dividends in this app's sense; an
/// ETF's distributions live in the fund profile, not here. Routed through the
/// shared `yahoo` `EndpointGuard` so it shares pacing and the per-hour budget
/// with the intraday and daily-close jobs.
///
/// Resumable in the same way as the SEC job: each stock's
/// `dividends_synced_at` is stamped only on a successful fetch, so a guard
/// stop leaves the rest for the next cycle.
async fn run_dividends(pool: &SqlitePool, config: &Config, hub: &Hub) -> anyhow::Result<()> {
    let started = now_ms();
    let next = started + DIVIDENDS_INTERVAL_SECS * 1000;
    let cutoff = started - DIVIDENDS_STALE_SECS * 1000;
    // Phase 28: dividends now covers ETF distributions too — same Yahoo
    // event series, same store path. Indexes and futures still skipped (the
    // former do not pay anything, the latter have no concept of dividends).
    let stale: Vec<String> = sqlx::query_scalar(
        "SELECT ticker FROM symbols \
         WHERE kind IN ('stock', 'etf') \
           AND (dividends_synced_at IS NULL OR dividends_synced_at < ?) \
         ORDER BY ticker",
    )
    .bind(cutoff)
    .fetch_all(pool)
    .await?;

    if stale.is_empty() {
        // The fast path: nothing stale, nothing to do. No fetching banner.
        mark_ok(pool, "dividends", Some(next)).await?;
        return Ok(());
    }

    mark_fetching(pool, "dividends").await?;
    notify_health(hub);
    tracing::info!("[scheduler] dividends: refreshing {} symbols", stale.len());

    let yahoo = YahooProvider::new(providers::http::build_client(config));
    let guard = EndpointGuard::with_budget(pool.clone(), "yahoo", YAHOO_BUDGET);
    let t0 = Instant::now();
    let mut ok = 0i64;
    let mut payouts = 0i64;
    let mut errors = 0i64;
    let mut stopped: Option<String> = None;

    for ticker in &stale {
        match guard.acquire().await? {
            Permit::Granted => {}
            Permit::Denied(why) => {
                stopped = Some(why);
                break;
            }
        }
        match yahoo.dividends(ticker).await {
            Ok(events) => {
                guard.record_success().await?;
                payouts += events.len() as i64;
                if let Err(e) = store_dividends(pool, ticker, &events).await {
                    tracing::warn!("[scheduler] dividends store {ticker}: {e:#}");
                    errors += 1;
                    continue;
                }
                mark_dividends_synced(pool, ticker).await?;
                ok += 1;
            }
            Err(e) => {
                guard.record_failure(&e).await?;
                errors += 1;
                tracing::warn!("[scheduler] dividends {ticker}: {e:#}");
            }
        }
    }

    let dur = t0.elapsed().as_millis() as i64;
    let detail = format!("{ok}/{} symbols, {payouts} payouts, {errors} errors", stale.len());
    match stopped {
        Some(why) => {
            let full = format!("stopped early ({why}); {detail}");
            tracing::warn!("[scheduler] dividends: {full}");
            log_fetch(pool, "dividends", "yahoo", "skipped", Some(&full), Some(ok), dur, started)
                .await?;
        }
        None => {
            tracing::info!("[scheduler] dividends: {detail}");
            log_fetch(pool, "dividends", "yahoo", "ok", Some(&detail), Some(ok), dur, started)
                .await?;
        }
    }
    mark_ok(pool, "dividends", Some(next)).await?;
    notify_health(hub);
    Ok(())
}

/// Replace one stock's dividend history with what Yahoo returned. Yahoo serves
/// the canonical, corrected history each call, so a `DELETE` + `INSERT` keeps
/// the table honest if a payout is later retracted or restated. `pub(crate)`:
/// the add-symbol backfill reuses it.
pub(crate) async fn store_dividends(
    pool: &SqlitePool,
    ticker: &str,
    events: &[DividendEvent],
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM dividends WHERE ticker = ?")
        .bind(ticker)
        .execute(&mut *tx)
        .await?;
    for e in events {
        sqlx::query(
            "INSERT INTO dividends (ticker, ex_date, amount) VALUES (?, ?, ?) \
             ON CONFLICT(ticker, ex_date) DO UPDATE SET amount = excluded.amount",
        )
        .bind(ticker)
        .bind(&e.ex_date)
        .bind(e.amount)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Stamp a stock as freshly dividend-synced.
async fn mark_dividends_synced(pool: &SqlitePool, ticker: &str) -> sqlx::Result<()> {
    let now = now_ms();
    sqlx::query("UPDATE symbols SET dividends_synced_at = ?, updated_at = ? WHERE ticker = ?")
        .bind(now)
        .bind(now)
        .bind(ticker)
        .execute(pool)
        .await?;
    Ok(())
}

// ────────────────── ETF fund_metadata sweep (Phase 28) ────────────────────

/// Sweep ETFs whose Yahoo `quoteSummary` snapshot has gone stale (monthly).
/// One request per ETF through the shared `yahoo` `EndpointGuard`; expense
/// ratio, distribution yield, NAV, inception, category, fund family, and
/// the strategy paragraph are all returned by one request, so even the full
/// 28-ETF sweep is well inside the hourly budget. Mirrors `run_dividends`'s
/// shape — guard-paced, resumable, broadcast-to-/health.
async fn run_fund_metadata(pool: &SqlitePool, config: &Config, hub: &Hub) -> anyhow::Result<()> {
    let started = now_ms();
    let next = started + FUND_METADATA_INTERVAL_SECS * 1000;
    let cutoff = started - FUND_METADATA_STALE_SECS * 1000;
    let stale: Vec<String> = sqlx::query_scalar(
        "SELECT ticker FROM symbols \
         WHERE kind = 'etf' \
           AND (fund_metadata_synced_at IS NULL OR fund_metadata_synced_at < ?) \
         ORDER BY ticker",
    )
    .bind(cutoff)
    .fetch_all(pool)
    .await?;

    if stale.is_empty() {
        // Fast path: no ETFs need refreshing. No fetching banner, no log row.
        mark_ok(pool, "fund_metadata", Some(next)).await?;
        return Ok(());
    }

    mark_fetching(pool, "fund_metadata").await?;
    notify_health(hub);
    tracing::info!("[scheduler] fund_metadata: refreshing {} ETFs", stale.len());

    let yahoo = YahooProvider::new(providers::http::build_client(config));
    let guard = EndpointGuard::with_budget(pool.clone(), "yahoo", YAHOO_BUDGET);
    let t0 = Instant::now();
    let mut ok = 0i64;
    let mut empty = 0i64;
    let mut errors = 0i64;
    let mut stopped: Option<String> = None;

    for ticker in &stale {
        match guard.acquire().await? {
            Permit::Granted => {}
            Permit::Denied(why) => {
                stopped = Some(why);
                break;
            }
        }
        match yahoo.fund_metadata(ticker).await {
            Ok(Some(meta)) => {
                guard.record_success().await?;
                if let Err(e) = store_fund_metadata(pool, ticker, &meta).await {
                    tracing::warn!("[scheduler] fund_metadata store {ticker}: {e:#}");
                    errors += 1;
                    continue;
                }
                mark_fund_metadata_synced(pool, ticker).await?;
                ok += 1;
            }
            // Yahoo answered cleanly but the ETF has no fund modules (a tiny
            // or obscure ticker): stamp it checked so we do not re-fetch the
            // same empty answer next tick, but log the empty.
            Ok(None) => {
                guard.record_success().await?;
                mark_fund_metadata_synced(pool, ticker).await?;
                empty += 1;
            }
            Err(e) => {
                guard.record_failure(&e).await?;
                errors += 1;
                tracing::warn!("[scheduler] fund_metadata {ticker}: {e:#}");
            }
        }
    }

    let dur = t0.elapsed().as_millis() as i64;
    let detail = format!(
        "{ok}/{} ETFs ({empty} empty, {errors} errors)",
        stale.len()
    );
    match stopped {
        Some(why) => {
            let full = format!("stopped early ({why}); {detail}");
            tracing::warn!("[scheduler] fund_metadata: {full}");
            log_fetch(
                pool, "fund_metadata", "yahoo", "skipped",
                Some(&full), Some(ok), dur, started,
            ).await?;
        }
        None => {
            tracing::info!("[scheduler] fund_metadata: {detail}");
            log_fetch(
                pool, "fund_metadata", "yahoo", "ok",
                Some(&detail), Some(ok), dur, started,
            ).await?;
        }
    }
    mark_ok(pool, "fund_metadata", Some(next)).await?;
    notify_health(hub);
    Ok(())
}

/// Upsert one ETF's Yahoo `quoteSummary` metadata. Yahoo serves the full
/// current snapshot each call, so the row is replaced wholesale: a field
/// Yahoo no longer carries on a refresh becomes `NULL` here, rather than
/// keeping a stale value. `pub(crate)`: the add-symbol backfill reuses it.
pub(crate) async fn store_fund_metadata(
    pool: &SqlitePool,
    ticker: &str,
    m: &FundMetadata,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO fund_metadata \
           (ticker, expense_ratio, yield_pct, trailing_yield_pct, nav_price, \
            inception_date, category, fund_family, strategy_summary, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(ticker) DO UPDATE SET \
           expense_ratio = excluded.expense_ratio, \
           yield_pct = excluded.yield_pct, \
           trailing_yield_pct = excluded.trailing_yield_pct, \
           nav_price = excluded.nav_price, \
           inception_date = excluded.inception_date, \
           category = excluded.category, \
           fund_family = excluded.fund_family, \
           strategy_summary = excluded.strategy_summary, \
           updated_at = excluded.updated_at",
    )
    .bind(ticker)
    .bind(m.expense_ratio)
    .bind(m.yield_pct)
    .bind(m.trailing_yield_pct)
    .bind(m.nav_price)
    .bind(&m.inception_date)
    .bind(&m.category)
    .bind(&m.fund_family)
    .bind(&m.strategy_summary)
    .bind(now_ms())
    .execute(pool)
    .await?;
    Ok(())
}

/// Stamp an ETF as freshly fund-metadata-synced. ETFs only — the column is
/// `NULL` forever on every non-ETF row.
async fn mark_fund_metadata_synced(pool: &SqlitePool, ticker: &str) -> sqlx::Result<()> {
    let now = now_ms();
    sqlx::query(
        "UPDATE symbols SET fund_metadata_synced_at = ?, updated_at = ? WHERE ticker = ?",
    )
    .bind(now)
    .bind(now)
    .bind(ticker)
    .execute(pool)
    .await?;
    Ok(())
}

// ────────────── stock earnings calendar sweep (Phase 25) ──────────────────

/// Sweep stocks whose next-expected earnings date has gone stale (monthly)
/// or already passed. One request per stock through the shared `yahoo`
/// `EndpointGuard`; mirrors `run_fund_metadata`'s shape — guard-paced,
/// resumable, broadcast-to-/health.
async fn run_earnings_calendar(
    pool: &SqlitePool,
    config: &Config,
    hub: &Hub,
) -> anyhow::Result<()> {
    let started = now_ms();
    let next = started + EARNINGS_INTERVAL_SECS * 1000;
    let cutoff = started - EARNINGS_STALE_SECS * 1000;
    // Refresh a stock when either its sync-timestamp has aged out OR its
    // stored next date has already passed (the print landed; Yahoo should
    // carry the following quarter's date by now).
    let stale: Vec<String> = sqlx::query_scalar(
        "SELECT ticker FROM symbols \
         WHERE kind = 'stock' \
           AND ( \
                earnings_synced_at IS NULL OR earnings_synced_at < ? \
                OR (next_earnings_at IS NOT NULL AND next_earnings_at < ?) \
           ) \
         ORDER BY ticker",
    )
    .bind(cutoff)
    .bind(started)
    .fetch_all(pool)
    .await?;

    if stale.is_empty() {
        mark_ok(pool, "earnings_calendar", Some(next)).await?;
        return Ok(());
    }

    mark_fetching(pool, "earnings_calendar").await?;
    notify_health(hub);
    tracing::info!(
        "[scheduler] earnings_calendar: refreshing {} stocks",
        stale.len()
    );

    let yahoo = YahooProvider::new(providers::http::build_client(config));
    let guard = EndpointGuard::with_budget(pool.clone(), "yahoo", YAHOO_BUDGET);
    let t0 = Instant::now();
    let mut ok = 0i64;
    let mut empty = 0i64;
    let mut errors = 0i64;
    let mut stopped: Option<String> = None;

    for ticker in &stale {
        match guard.acquire().await? {
            Permit::Granted => {}
            Permit::Denied(why) => {
                stopped = Some(why);
                break;
            }
        }
        match yahoo.earnings_calendar(ticker).await {
            Ok(Some(ts_ms)) => {
                guard.record_success().await?;
                if let Err(e) = store_earnings_next(pool, ticker, Some(ts_ms)).await {
                    tracing::warn!("[scheduler] earnings_calendar store {ticker}: {e:#}");
                    errors += 1;
                    continue;
                }
                ok += 1;
            }
            // Yahoo answered cleanly but has no upcoming date for this
            // stock (uneven coverage). Clear the stored next date so the
            // symbol page falls back to a cadence estimate, and stamp the
            // sync so the next sweep does not re-fetch the same empty.
            Ok(None) => {
                guard.record_success().await?;
                if let Err(e) = store_earnings_next(pool, ticker, None).await {
                    tracing::warn!("[scheduler] earnings_calendar store {ticker}: {e:#}");
                    errors += 1;
                    continue;
                }
                empty += 1;
            }
            Err(e) => {
                guard.record_failure(&e).await?;
                errors += 1;
                tracing::warn!("[scheduler] earnings_calendar {ticker}: {e:#}");
            }
        }
    }

    let dur = t0.elapsed().as_millis() as i64;
    let detail = format!(
        "{ok}/{} stocks ({empty} empty, {errors} errors)",
        stale.len()
    );
    match stopped {
        Some(why) => {
            let full = format!("stopped early ({why}); {detail}");
            tracing::warn!("[scheduler] earnings_calendar: {full}");
            log_fetch(
                pool, "earnings_calendar", "yahoo", "skipped",
                Some(&full), Some(ok), dur, started,
            )
            .await?;
        }
        None => {
            tracing::info!("[scheduler] earnings_calendar: {detail}");
            log_fetch(
                pool, "earnings_calendar", "yahoo", "ok",
                Some(&detail), Some(ok), dur, started,
            )
            .await?;
        }
    }
    mark_ok(pool, "earnings_calendar", Some(next)).await?;
    notify_health(hub);
    Ok(())
}

/// Write one stock's next-earnings date and stamp it as freshly synced.
/// `next` is `None` when Yahoo has no upcoming date (the stored value is
/// cleared so the page falls back to a cadence estimate). `pub(crate)`:
/// the add-symbol backfill reuses it.
pub(crate) async fn store_earnings_next(
    pool: &SqlitePool,
    ticker: &str,
    next: Option<i64>,
) -> sqlx::Result<()> {
    let now = now_ms();
    sqlx::query(
        "UPDATE symbols SET next_earnings_at = ?, earnings_synced_at = ?, \
                            updated_at = ? \
         WHERE ticker = ?",
    )
    .bind(next)
    .bind(now)
    .bind(now)
    .bind(ticker)
    .execute(pool)
    .await?;
    Ok(())
}

// ────────────── stock asset profile sweep (Phase 15) ──────────────────────

/// Sweep stocks whose Yahoo `quoteSummary.assetProfile` snapshot has gone
/// stale (monthly), refreshing each stock's sector and industry. One request
/// per stock through the shared `yahoo` `EndpointGuard`; mirrors the
/// `earnings_calendar` shape — guard-paced, resumable, broadcast-to-/health.
///
/// A stock Yahoo cleanly knows but has no `assetProfile` module for (uneven
/// coverage on small caps) is still stamped synced, so the sweep does not
/// re-fetch the same empty answer every cycle.
async fn run_asset_profile(
    pool: &SqlitePool,
    config: &Config,
    hub: &Hub,
) -> anyhow::Result<()> {
    let started = now_ms();
    let next = started + ASSET_PROFILE_INTERVAL_SECS * 1000;
    let cutoff = started - ASSET_PROFILE_STALE_SECS * 1000;
    let stale: Vec<String> = sqlx::query_scalar(
        "SELECT ticker FROM symbols \
         WHERE kind = 'stock' \
           AND (asset_profile_synced_at IS NULL OR asset_profile_synced_at < ?) \
         ORDER BY ticker",
    )
    .bind(cutoff)
    .fetch_all(pool)
    .await?;

    if stale.is_empty() {
        mark_ok(pool, "asset_profile", Some(next)).await?;
        return Ok(());
    }

    mark_fetching(pool, "asset_profile").await?;
    notify_health(hub);
    tracing::info!(
        "[scheduler] asset_profile: refreshing {} stocks",
        stale.len()
    );

    let yahoo = YahooProvider::new(providers::http::build_client(config));
    let guard = EndpointGuard::with_budget(pool.clone(), "yahoo", YAHOO_BUDGET);
    let t0 = Instant::now();
    let mut ok = 0i64;
    let mut empty = 0i64;
    let mut errors = 0i64;
    let mut stopped: Option<String> = None;

    for ticker in &stale {
        match guard.acquire().await? {
            Permit::Granted => {}
            Permit::Denied(why) => {
                stopped = Some(why);
                break;
            }
        }
        match yahoo.asset_profile(ticker).await {
            Ok(Some(profile)) => {
                guard.record_success().await?;
                if let Err(e) = store_asset_profile(pool, ticker, &profile).await {
                    tracing::warn!("[scheduler] asset_profile store {ticker}: {e:#}");
                    errors += 1;
                    continue;
                }
                ok += 1;
            }
            // Yahoo answered cleanly but had no profile for this stock —
            // stamp it checked so the next sweep does not re-fetch the
            // same empty answer.
            Ok(None) => {
                guard.record_success().await?;
                let _ = mark_asset_profile_synced(pool, ticker).await;
                empty += 1;
            }
            Err(e) => {
                guard.record_failure(&e).await?;
                errors += 1;
                tracing::warn!("[scheduler] asset_profile {ticker}: {e:#}");
            }
        }
    }

    let dur = t0.elapsed().as_millis() as i64;
    let detail = format!(
        "{ok}/{} stocks ({empty} empty, {errors} errors)",
        stale.len()
    );
    match stopped {
        Some(why) => {
            let full = format!("stopped early ({why}); {detail}");
            tracing::warn!("[scheduler] asset_profile: {full}");
            log_fetch(
                pool, "asset_profile", "yahoo", "skipped",
                Some(&full), Some(ok), dur, started,
            ).await?;
        }
        None => {
            tracing::info!("[scheduler] asset_profile: {detail}");
            log_fetch(
                pool, "asset_profile", "yahoo", "ok",
                Some(&detail), Some(ok), dur, started,
            ).await?;
        }
    }
    mark_ok(pool, "asset_profile", Some(next)).await?;
    notify_health(hub);
    Ok(())
}

/// Write one stock's sector / industry classification and stamp it freshly
/// synced. Either field may be `None` when Yahoo's coverage is partial; an
/// empty / whitespace value is dropped at parse time, not stored. `pub(crate)`:
/// the add-symbol backfill reuses it.
pub(crate) async fn store_asset_profile(
    pool: &SqlitePool,
    ticker: &str,
    profile: &crate::providers::AssetProfile,
) -> sqlx::Result<()> {
    let now = now_ms();
    sqlx::query(
        "UPDATE symbols SET sector = ?, industry = ?, \
                            asset_profile_synced_at = ?, updated_at = ? \
         WHERE ticker = ?",
    )
    .bind(&profile.sector)
    .bind(&profile.industry)
    .bind(now)
    .bind(now)
    .bind(ticker)
    .execute(pool)
    .await?;
    Ok(())
}

/// Stamp a stock as freshly asset-profile-synced without overwriting its
/// stored sector / industry (used on a clean-empty Yahoo response so the
/// sweep does not re-fetch).
async fn mark_asset_profile_synced(pool: &SqlitePool, ticker: &str) -> sqlx::Result<()> {
    let now = now_ms();
    sqlx::query(
        "UPDATE symbols SET asset_profile_synced_at = ?, updated_at = ? WHERE ticker = ?",
    )
    .bind(now)
    .bind(now)
    .bind(ticker)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fill in `symbols.cik` for any stock found in the bulk SEC ticker map.
/// Returns how many were newly resolved.
async fn resolve_ciks(pool: &SqlitePool, map: &HashMap<String, String>) -> sqlx::Result<i64> {
    let stocks: Vec<String> =
        sqlx::query_scalar("SELECT ticker FROM symbols WHERE kind = 'stock' AND cik IS NULL")
            .fetch_all(pool)
            .await?;
    let mut resolved = 0;
    for ticker in stocks {
        if let Some(cik) = map.get(&crate::providers::sec::normalize_ticker(&ticker)) {
            let now = now_ms();
            sqlx::query("UPDATE symbols SET cik = ?, updated_at = ? WHERE ticker = ?")
                .bind(cik)
                .bind(now)
                .bind(&ticker)
                .execute(pool)
                .await?;
            resolved += 1;
        }
    }
    Ok(resolved)
}

/// Fill in `symbols.cik` and `symbols.series_id` for any ETF found in the bulk
/// SEC mutual-fund ticker map. Returns how many were newly resolved.
async fn resolve_fund_ciks(
    pool: &SqlitePool,
    map: &HashMap<String, FundId>,
) -> sqlx::Result<i64> {
    let etfs: Vec<String> =
        sqlx::query_scalar("SELECT ticker FROM symbols WHERE kind = 'etf' AND cik IS NULL")
            .fetch_all(pool)
            .await?;
    let mut resolved = 0;
    for ticker in etfs {
        if let Some(id) = map.get(&crate::providers::sec::normalize_ticker(&ticker)) {
            let now = now_ms();
            sqlx::query(
                "UPDATE symbols SET cik = ?, series_id = ?, updated_at = ? WHERE ticker = ?",
            )
            .bind(&id.cik)
            .bind(&id.series_id)
            .bind(now)
            .bind(&ticker)
            .execute(pool)
            .await?;
            resolved += 1;
        }
    }
    Ok(resolved)
}

/// Stamp one of a symbol's SEC sync timestamps to now. `column` is one of a
/// few hardcoded literals (never user input), so interpolating it is safe.
async fn mark_sec_synced(pool: &SqlitePool, ticker: &str, column: &str) -> sqlx::Result<()> {
    let now = now_ms();
    let sql = format!("UPDATE symbols SET {column} = ?, updated_at = ? WHERE ticker = ?");
    sqlx::query(&sql)
        .bind(now)
        .bind(now)
        .bind(ticker)
        .execute(pool)
        .await?;
    Ok(())
}

/// Upsert one company's fundamental facts. Keyed on (ticker, metric, period),
/// so a later filing's restated figure overwrites the prior one.
async fn store_fundamentals(pool: &SqlitePool, ticker: &str, facts: &[Fact]) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    for f in facts {
        sqlx::query(
            "INSERT INTO fundamentals \
               (ticker, metric, period, fiscal_year, fiscal_qtr, period_end, \
                value, unit, form, filed_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(ticker, metric, period) DO UPDATE SET \
               fiscal_year = excluded.fiscal_year, fiscal_qtr = excluded.fiscal_qtr, \
               period_end = excluded.period_end, value = excluded.value, \
               unit = excluded.unit, form = excluded.form, filed_at = excluded.filed_at",
        )
        .bind(ticker)
        .bind(&f.metric)
        .bind(&f.period)
        .bind(f.fiscal_year)
        .bind(f.fiscal_qtr)
        .bind(&f.period_end)
        .bind(f.value)
        .bind(&f.unit)
        .bind(&f.form)
        .bind(&f.filed_at)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Upsert one company's filing history. Keyed on (ticker, accession).
async fn store_filings(
    pool: &SqlitePool,
    ticker: &str,
    filings: &[FilingRecord],
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    for f in filings {
        sqlx::query(
            "INSERT INTO filings \
               (ticker, accession, form, filed_at, period_of_report, \
                primary_doc, url, description, items) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(ticker, accession) DO UPDATE SET \
               form = excluded.form, filed_at = excluded.filed_at, \
               period_of_report = excluded.period_of_report, \
               primary_doc = excluded.primary_doc, url = excluded.url, \
               description = excluded.description, items = excluded.items",
        )
        .bind(ticker)
        .bind(&f.accession)
        .bind(&f.form)
        .bind(&f.filed_at)
        .bind(&f.period_of_report)
        .bind(&f.primary_doc)
        .bind(&f.url)
        .bind(&f.description)
        .bind(&f.items)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Upsert a company's leadership roster from parsed ownership filings (Phase
/// 14). `roster` arrives newest-filing-first, so the first entry seen for a
/// person is their most recent filing and any later duplicate is skipped. The
/// upsert is guarded on `last_seen`, so a stale re-parse never overwrites a
/// person's role with an older filing's; departed insiders simply stop being
/// re-stamped and age out of the symbol page's recency window.
async fn store_leadership(
    pool: &SqlitePool,
    ticker: &str,
    roster: &[(OwnershipPerson, String)],
) -> sqlx::Result<()> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut tx = pool.begin().await?;
    for (person, filed_at) in roster {
        if !seen.insert(person.name.as_str()) {
            continue;
        }
        sqlx::query(
            "INSERT INTO leadership \
               (ticker, name, is_director, is_officer, officer_title, last_seen) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(ticker, name) DO UPDATE SET \
               is_director = excluded.is_director, is_officer = excluded.is_officer, \
               officer_title = excluded.officer_title, last_seen = excluded.last_seen \
             WHERE excluded.last_seen >= leadership.last_seen",
        )
        .bind(ticker)
        .bind(&person.name)
        .bind(person.is_director as i64)
        .bind(person.is_officer as i64)
        .bind(&person.officer_title)
        .bind(filed_at)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Upsert one ETF's N-PORT fund profile and replace its stored holdings. The
/// kept holdings are a small top slice, so they are deleted and re-inserted
/// wholesale on each refresh.
async fn store_fund_portfolio(
    pool: &SqlitePool,
    ticker: &str,
    p: &PortfolioData,
) -> anyhow::Result<()> {
    // Asset / sector / geography mixes are each variable-length, so they ride
    // in JSON columns rather than their own tables:
    // [["Equity", 99.8], ["Cash & equivalents", 0.2], ...].
    let asset_mix = serde_json::to_string(&p.asset_mix)?;
    let sector_mix = serde_json::to_string(&p.sector_mix)?;
    let geography_mix = serde_json::to_string(&p.geography_mix)?;
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO fund_profiles \
           (ticker, kind, net_assets, total_assets, holdings_count, report_date, \
            asset_mix, sector_mix, geography_mix, updated_at) \
         VALUES (?, 'portfolio', ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(ticker) DO UPDATE SET \
           kind = excluded.kind, net_assets = excluded.net_assets, \
           total_assets = excluded.total_assets, holdings_count = excluded.holdings_count, \
           report_date = excluded.report_date, asset_mix = excluded.asset_mix, \
           sector_mix = excluded.sector_mix, geography_mix = excluded.geography_mix, \
           updated_at = excluded.updated_at",
    )
    .bind(ticker)
    .bind(p.net_assets)
    .bind(p.total_assets)
    .bind(p.holdings_count)
    .bind(&p.report_date)
    .bind(&asset_mix)
    .bind(&sector_mix)
    .bind(&geography_mix)
    .bind(now_ms())
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM fund_holdings WHERE ticker = ?")
        .bind(ticker)
        .execute(&mut *tx)
        .await?;
    for (i, h) in p.top_holdings.iter().enumerate() {
        sqlx::query(
            "INSERT INTO fund_holdings (ticker, rank, name, pct, value_usd) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(ticker)
        .bind(i as i64 + 1)
        .bind(&h.name)
        .bind(h.pct)
        .bind(h.value_usd)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Record a physical-commodity grantor trust's profile: just the AUM read from
/// its 10-K. It holds bullion, not a securities portfolio, so there are no
/// holdings and no asset mix.
async fn store_fund_commodity(
    pool: &SqlitePool,
    ticker: &str,
    aum: Option<f64>,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO fund_profiles \
           (ticker, kind, net_assets, total_assets, holdings_count, report_date, \
            asset_mix, updated_at) \
         VALUES (?, 'commodity_trust', ?, NULL, NULL, NULL, NULL, ?) \
         ON CONFLICT(ticker) DO UPDATE SET \
           kind = excluded.kind, net_assets = excluded.net_assets, \
           updated_at = excluded.updated_at",
    )
    .bind(ticker)
    .bind(aum)
    .bind(now_ms())
    .execute(pool)
    .await?;
    Ok(())
}

/// Stamp an ETF's fund profile as freshly synced.
async fn mark_fund_synced(pool: &SqlitePool, ticker: &str) -> sqlx::Result<()> {
    let now = now_ms();
    sqlx::query("UPDATE symbols SET fund_synced_at = ?, updated_at = ? WHERE ticker = ?")
        .bind(now)
        .bind(now)
        .bind(ticker)
        .execute(pool)
        .await?;
    Ok(())
}

/// Upsert one symbol's live quote into `quotes` and refresh the denormalized
/// snapshot columns on `symbols` that the dashboard and SSE seeding read.
/// `pub(crate)`: the add-symbol route stores the quote its Yahoo lookup
/// already returned, rather than spending a second request.
pub(crate) async fn store_quote(pool: &SqlitePool, ticker: &str, q: &Quote) -> sqlx::Result<()> {
    let now = now_ms();
    sqlx::query(
        "INSERT INTO quotes \
           (ticker, price, prev_close, open, day_high, day_low, volume, \
            market_state, source, source_time, fetched_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'yahoo', ?, ?) \
         ON CONFLICT(ticker) DO UPDATE SET \
           price = excluded.price, prev_close = excluded.prev_close, \
           open = excluded.open, day_high = excluded.day_high, \
           day_low = excluded.day_low, volume = excluded.volume, \
           market_state = excluded.market_state, source = excluded.source, \
           source_time = excluded.source_time, fetched_at = excluded.fetched_at",
    )
    .bind(ticker)
    .bind(q.price)
    .bind(q.prev_close)
    .bind(q.open)
    .bind(q.day_high)
    .bind(q.day_low)
    .bind(q.volume)
    .bind(&q.market_state)
    .bind(q.source_time)
    .bind(now)
    .execute(pool)
    .await?;

    sqlx::query(
        "UPDATE symbols SET last_price = ?, prev_close = ?, last_quote_at = ?, \
         updated_at = ? WHERE ticker = ?",
    )
    .bind(q.price)
    .bind(q.prev_close)
    .bind(now)
    .bind(now)
    .bind(ticker)
    .execute(pool)
    .await?;
    Ok(())
}

/// Upsert one symbol's intraday bars in a single transaction. The prune job
/// trims `intraday_bars` to a rolling ~14-day window, so nothing here grows
/// without bound. `pub(crate)`: also called by the add-symbol route.
pub(crate) async fn store_intraday(
    pool: &SqlitePool,
    ticker: &str,
    bars: &[IntradayBar],
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    for b in bars {
        sqlx::query(
            "INSERT INTO intraday_bars (ticker, ts, open, high, low, close, volume) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(ticker, ts) DO UPDATE SET \
               open = excluded.open, high = excluded.high, low = excluded.low, \
               close = excluded.close, volume = excluded.volume",
        )
        .bind(ticker)
        .bind(b.ts)
        .bind(b.open)
        .bind(b.high)
        .bind(b.low)
        .bind(b.close)
        .bind(b.volume)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

// ── synchronous backfill for a freshly-added symbol (Phase 21) ─────────────

/// Run one guarded outbound call: acquire a permit, await `call`, and feed the
/// outcome back to the guard. `None` when the guard denied the request (the
/// breaker is open or the hourly budget is spent) or the permit could not be
/// acquired; `Some` carries whatever the call itself returned.
async fn guarded<T>(
    guard: &EndpointGuard,
    call: impl std::future::Future<Output = anyhow::Result<T>>,
) -> Option<anyhow::Result<T>> {
    match guard.acquire().await {
        Ok(Permit::Granted) => {
            let result = call.await;
            match &result {
                Ok(_) => {
                    let _ = guard.record_success().await;
                }
                Err(e) => {
                    let _ = guard.record_failure(e).await;
                }
            }
            Some(result)
        }
        Ok(Permit::Denied(why)) => {
            tracing::info!("[backfill] guard denied: {why}");
            None
        }
        Err(e) => {
            tracing::warn!("[backfill] guard error: {e:#}");
            None
        }
    }
}

/// Synchronously backfill everything for one just-added symbol: its deep daily
/// history from Stooq and, for a stock or ETF, its SEC data. The add-symbol
/// route (`routes::symbols`) calls this so a user-added symbol's page is
/// complete the moment the add returns, rather than filling in over later
/// scheduler cycles (PLAN.md Phase 21).
///
/// Best-effort and guard-routed: every outbound call passes through the same
/// `EndpointGuard` the background jobs use, and a guard denial or upstream
/// error for any one piece is logged and skipped. The symbol is already added;
/// the normal scheduler sweeps pick up whatever this run missed.
pub(crate) async fn backfill_symbol(pool: &SqlitePool, config: &Config, ticker: &str, kind: &str) {
    backfill_history(pool, config, ticker, kind).await;
    // Phase 26 + Phase 28: dividends covers stock dividends and ETF
    // distributions (same Yahoo event series). Rides the Yahoo guard, so it
    // runs independently of the SEC contact-email gate below.
    if kind == "stock" || kind == "etf" {
        backfill_dividends(pool, config, ticker).await;
    }
    // Phase 28: ETFs get their Yahoo fund_metadata pulled too — expense
    // ratio, yield, NAV, inception, category, family, strategy summary.
    if kind == "etf" {
        backfill_fund_metadata(pool, config, ticker).await;
    }
    // Phase 25: stocks get their Yahoo earnings calendar pulled too — so a
    // user-added stock's symbol page carries the next-expected earnings
    // date the moment the add returns, rather than waiting on the next
    // scheduler cycle.
    if kind == "stock" {
        backfill_earnings_calendar(pool, config, ticker).await;
    }
    // Phase 15: stocks get their Yahoo assetProfile pulled too — so a
    // user-added stock immediately shows up under its sector and industry
    // on /industries and carries the symbol-page header tag, rather than
    // waiting on the next monthly scheduler sweep.
    if kind == "stock" {
        backfill_asset_profile(pool, config, ticker).await;
    }

    // SEC data covers stocks and ETFs; indexes and futures do not file. The
    // whole SEC step is skipped with no contact email configured, as `run_sec`
    // skips itself.
    if config.sec_contact_email.is_empty() {
        return;
    }
    let sec = SecProvider::new(providers::http::build_sec_client(config));
    let guard = EndpointGuard::with_budget(pool.clone(), sec.name(), SEC_BUDGET);
    match kind {
        "stock" => backfill_stock_sec(pool, &sec, &guard, ticker).await,
        "etf" => backfill_etf_sec(pool, &sec, &guard, ticker).await,
        _ => {}
    }
}

/// Pull and store a freshly-added ETF's Yahoo `quoteSummary` metadata (Phase
/// 28). Mirrors `backfill_dividends`: same `yahoo` guard, best-effort, no
/// failure propagated to the add-symbol response.
async fn backfill_fund_metadata(pool: &SqlitePool, config: &Config, ticker: &str) {
    let yahoo = YahooProvider::new(providers::http::build_client(config));
    let guard = EndpointGuard::with_budget(pool.clone(), "yahoo", YAHOO_BUDGET);
    match guarded(&guard, yahoo.fund_metadata(ticker)).await {
        Some(Ok(Some(meta))) => match store_fund_metadata(pool, ticker, &meta).await {
            Ok(()) => {
                let _ = mark_fund_metadata_synced(pool, ticker).await;
                tracing::info!("[backfill] {ticker} <- fund_metadata");
            }
            Err(e) => tracing::warn!("[backfill] store fund_metadata {ticker}: {e:#}"),
        },
        // Yahoo answered cleanly but had no fund modules for this ETF — stamp
        // it checked so the next sweep does not re-fetch the same empty.
        Some(Ok(None)) => {
            let _ = mark_fund_metadata_synced(pool, ticker).await;
        }
        Some(Err(e)) => tracing::warn!("[backfill] fund_metadata {ticker}: {e:#}"),
        None => {}
    }
}

/// Pull and store a freshly-added stock's sector / industry classification
/// (Phase 15). Mirrors `backfill_earnings_calendar`: same `yahoo` guard,
/// best-effort, no failure propagated to the add-symbol response.
async fn backfill_asset_profile(pool: &SqlitePool, config: &Config, ticker: &str) {
    let yahoo = YahooProvider::new(providers::http::build_client(config));
    let guard = EndpointGuard::with_budget(pool.clone(), "yahoo", YAHOO_BUDGET);
    match guarded(&guard, yahoo.asset_profile(ticker)).await {
        Some(Ok(Some(profile))) => match store_asset_profile(pool, ticker, &profile).await {
            Ok(()) => {
                tracing::info!(
                    "[backfill] {ticker} <- asset_profile ({} / {})",
                    profile.sector.as_deref().unwrap_or("—"),
                    profile.industry.as_deref().unwrap_or("—"),
                );
            }
            Err(e) => tracing::warn!("[backfill] store asset_profile {ticker}: {e:#}"),
        },
        // Yahoo answered cleanly but had no profile for this stock — stamp
        // it so the next sweep does not re-fetch the same empty.
        Some(Ok(None)) => {
            let _ = mark_asset_profile_synced(pool, ticker).await;
        }
        Some(Err(e)) => tracing::warn!("[backfill] asset_profile {ticker}: {e:#}"),
        None => {}
    }
}

/// Pull and store a freshly-added stock's next-expected earnings date
/// (Phase 25). Mirrors `backfill_dividends`: same `yahoo` guard,
/// best-effort, no failure propagated to the add-symbol response.
async fn backfill_earnings_calendar(pool: &SqlitePool, config: &Config, ticker: &str) {
    let yahoo = YahooProvider::new(providers::http::build_client(config));
    let guard = EndpointGuard::with_budget(pool.clone(), "yahoo", YAHOO_BUDGET);
    match guarded(&guard, yahoo.earnings_calendar(ticker)).await {
        Some(Ok(next)) => match store_earnings_next(pool, ticker, next).await {
            Ok(()) => {
                tracing::info!(
                    "[backfill] {ticker} <- earnings_calendar ({})",
                    next.map(|_| "next set").unwrap_or("no upcoming date"),
                );
            }
            Err(e) => tracing::warn!("[backfill] store earnings {ticker}: {e:#}"),
        },
        Some(Err(e)) => tracing::warn!("[backfill] earnings_calendar {ticker}: {e:#}"),
        None => {}
    }
}

/// Pull and store a freshly-added symbol's dividend / distribution history
/// (Phase 26 + Phase 28: stocks and ETFs both use this path). Routed through
/// the same `yahoo` guard the dividends sweep uses. Best-effort: a guard
/// denial or upstream error leaves the symbol for the next normal sweep.
async fn backfill_dividends(pool: &SqlitePool, config: &Config, ticker: &str) {
    let yahoo = YahooProvider::new(providers::http::build_client(config));
    let guard = EndpointGuard::with_budget(pool.clone(), "yahoo", YAHOO_BUDGET);
    match guarded(&guard, yahoo.dividends(ticker)).await {
        Some(Ok(events)) => match store_dividends(pool, ticker, &events).await {
            Ok(()) => {
                let _ = mark_dividends_synced(pool, ticker).await;
                tracing::info!("[backfill] {ticker} <- {} dividends", events.len());
            }
            Err(e) => tracing::warn!("[backfill] store dividends {ticker}: {e:#}"),
        },
        Some(Err(e)) => tracing::warn!("[backfill] dividends {ticker}: {e:#}"),
        None => {}
    }
}

/// Pull and store one symbol's deep daily history from Yahoo (one
/// `interval=1d&range=max` call). Used by the add-symbol backfill; all kinds
/// are eligible since Yahoo serves `=F` futures history too.
async fn backfill_history(pool: &SqlitePool, config: &Config, ticker: &str, _kind: &str) {
    let yahoo = YahooProvider::new(providers::http::build_client(config));
    let guard = EndpointGuard::with_budget(pool.clone(), "yahoo", YAHOO_BUDGET);
    match guarded(&guard, yahoo.daily(ticker, None)).await {
        Some(Ok(bars)) if !bars.is_empty() => match seed::store_daily(pool, ticker, &bars).await {
            Ok(()) => tracing::info!("[backfill] {ticker} <- {} daily bars", bars.len()),
            Err(e) => tracing::warn!("[backfill] store history {ticker}: {e:#}"),
        },
        // A valid but empty response (a historyless symbol): stamp it checked
        // so the history job does not immediately re-fetch it.
        Some(Ok(_)) => {
            let _ = mark_history_checked(pool, ticker).await;
        }
        _ => {}
    }
}

/// Backfill a stock's SEC data: resolve its CIK, then pull fundamentals,
/// filings, and the officer/board roster.
async fn backfill_stock_sec(
    pool: &SqlitePool,
    sec: &SecProvider,
    guard: &EndpointGuard,
    ticker: &str,
) {
    let Some(cik) = resolve_one_cik(pool, sec, guard, ticker, false).await else {
        tracing::info!("[backfill] {ticker}: no SEC CIK, leaving it for the sec job");
        return;
    };
    if let Some(Ok(facts)) = guarded(guard, sec.facts(&cik)).await {
        match store_fundamentals(pool, ticker, &facts).await {
            Ok(()) => {
                let _ = mark_sec_synced(pool, ticker, "fundamentals_synced_at").await;
            }
            Err(e) => tracing::warn!("[backfill] store facts {ticker}: {e:#}"),
        }
    }
    if let Some(Ok(filings)) = guarded(guard, sec.filings(&cik)).await {
        match store_filings(pool, ticker, &filings).await {
            Ok(()) => {
                let _ = mark_sec_synced(pool, ticker, "filings_synced_at").await;
            }
            Err(e) => tracing::warn!("[backfill] store filings {ticker}: {e:#}"),
        }
    }
    backfill_leadership(pool, sec, guard, ticker, &cik).await;
}

/// Backfill a stock's officer/board roster from a window of its most recent
/// Form 3/4/5 ownership filings, mirroring the leadership sweep in `run_sec`.
async fn backfill_leadership(
    pool: &SqlitePool,
    sec: &SecProvider,
    guard: &EndpointGuard,
    ticker: &str,
    cik: &str,
) {
    let Some(Ok(index)) = guarded(guard, sec.ownership_index(cik)).await else {
        return;
    };
    let to_parse: Vec<_> = index.into_iter().take(LEADERSHIP_MAX_FILINGS).collect();

    let mut roster: Vec<(OwnershipPerson, String)> = Vec::new();
    let mut complete = true;
    for f in &to_parse {
        match guarded(guard, sec.ownership_doc(cik, &f.accession, &f.primary_doc)).await {
            Some(Ok(people)) => {
                for p in people {
                    if p.is_director || p.is_officer {
                        roster.push((p, f.filed_at.clone()));
                    }
                }
            }
            // A parse or network error for one filing: skip it and build the
            // roster from the rest, exactly as `run_sec` does.
            Some(Err(e)) => tracing::warn!("[backfill] ownership_doc {ticker}: {e:#}"),
            // A guard denial leaves the roster only partial: leave it unsynced
            // so the next `sec` cycle finishes it.
            None => complete = false,
        }
    }
    let _ = store_leadership(pool, ticker, &roster).await;
    if complete {
        let _ = mark_sec_synced(pool, ticker, "leadership_synced_at").await;
    }
}

/// Backfill an ETF's fund profile: resolve its fund CIK, pull the filing list,
/// then either the N-PORT portfolio or a commodity trust's AUM.
async fn backfill_etf_sec(pool: &SqlitePool, sec: &SecProvider, guard: &EndpointGuard, ticker: &str) {
    let Some(cik) = resolve_one_cik(pool, sec, guard, ticker, true).await else {
        tracing::info!("[backfill] {ticker}: no SEC fund CIK, leaving it for the sec job");
        return;
    };
    // `resolve_fund_ciks` stored the series id alongside the CIK.
    let series_id: Option<String> =
        sqlx::query_scalar("SELECT series_id FROM symbols WHERE ticker = ?")
            .bind(ticker)
            .fetch_one(pool)
            .await
            .ok()
            .flatten();
    let id = FundId {
        cik: cik.clone(),
        series_id,
    };

    let Some(Ok(ff)) = guarded(guard, sec.fund_filings(&id)).await else {
        return;
    };
    let _ = store_filings(pool, ticker, &ff.filings).await;
    match ff.shape {
        FundShape::Portfolio { nport_href } => {
            if let Some(Ok(portfolio)) = guarded(guard, sec.fund_portfolio(&nport_href)).await {
                if store_fund_portfolio(pool, ticker, &portfolio).await.is_ok() {
                    let _ = mark_fund_synced(pool, ticker).await;
                }
            }
        }
        FundShape::CommodityTrust => {
            if let Some(Ok(aum)) = guarded(guard, sec.fund_aum(&cik)).await {
                if store_fund_commodity(pool, ticker, aum).await.is_ok() {
                    let _ = mark_fund_synced(pool, ticker).await;
                }
            }
        }
        FundShape::Unknown => {
            let _ = mark_fund_synced(pool, ticker).await;
        }
    }
}

/// Resolve and store a freshly-added symbol's SEC CIK from the bulk ticker map.
/// `fund` selects the mutual-fund map (ETFs) over the operating-company map
/// (stocks). Returns the stored CIK on success.
async fn resolve_one_cik(
    pool: &SqlitePool,
    sec: &SecProvider,
    guard: &EndpointGuard,
    ticker: &str,
    fund: bool,
) -> Option<String> {
    if fund {
        if let Some(Ok(map)) = guarded(guard, sec.fund_ticker_map()).await {
            let _ = resolve_fund_ciks(pool, &map).await;
        }
    } else if let Some(Ok(map)) = guarded(guard, sec.cik_map()).await {
        let _ = resolve_ciks(pool, &map).await;
    }
    sqlx::query_scalar::<_, Option<String>>("SELECT cik FROM symbols WHERE ticker = ?")
        .bind(ticker)
        .fetch_one(pool)
        .await
        .ok()
        .flatten()
}

/// Prune aged rows once per `PRUNE_INTERVAL_SECS`. `intraday_bars` keeps a
/// rolling ~14-day window; `fetch_log` keeps ~30 days. `daily_prices` is
/// permanent and never touched here.
async fn run_prune_if_due(
    pool: &SqlitePool,
    last: &mut Option<i64>,
    hub: &Hub,
) -> anyhow::Result<()> {
    let now = now_ms();
    if let Some(t) = *last {
        if (now - t) / 1000 < PRUNE_INTERVAL_SECS {
            return Ok(());
        }
    }

    let t0 = Instant::now();
    let intraday_cutoff = now - INTRADAY_RETENTION_DAYS * 86_400 * 1000;
    let log_cutoff = now - FETCH_LOG_RETENTION_DAYS * 86_400 * 1000;

    let bars = sqlx::query("DELETE FROM intraday_bars WHERE ts < ?")
        .bind(intraday_cutoff)
        .execute(pool)
        .await?
        .rows_affected();
    let logs = sqlx::query("DELETE FROM fetch_log WHERE started_at < ?")
        .bind(log_cutoff)
        .execute(pool)
        .await?
        .rows_affected();

    let dur = t0.elapsed().as_millis() as i64;
    let detail = format!("{bars} intraday bars, {logs} fetch_log rows");
    tracing::info!("[scheduler] prune: removed {detail}");
    log_fetch(pool, "prune", "-", "ok", Some(&detail), Some((bars + logs) as i64), dur, now).await?;

    *last = Some(now);
    notify_health(hub);
    Ok(())
}

// ── data_status / fetch_log helpers ───────────────────────────────────────

/// Nudge any connected `/health` page to pull a fresh snapshot. Sent whenever a
/// job changes state or appends a `fetch_log` row, so the data-health page
/// tracks the worker in near real time. Carries no payload (see `StreamEvent`).
fn notify_health(hub: &Hub) {
    hub.publish(StreamEvent::Health);
}

/// Move a job's `data_status` row to the `fetching` state.
async fn mark_fetching(pool: &SqlitePool, job: &str) -> sqlx::Result<()> {
    let now = now_ms();
    sqlx::query(
        "INSERT INTO data_status (job, state, updated_at) VALUES (?, 'fetching', ?) \
         ON CONFLICT(job) DO UPDATE SET state = 'fetching', updated_at = excluded.updated_at",
    )
    .bind(job)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark a job finished-OK, recording when it next falls due (`None` for
/// one-shot jobs like the seed).
async fn mark_ok(pool: &SqlitePool, job: &str, next_run_at: Option<i64>) -> sqlx::Result<()> {
    let now = now_ms();
    sqlx::query(
        "INSERT INTO data_status (job, state, last_ok_at, next_run_at, updated_at) \
         VALUES (?, 'ok', ?, ?, ?) \
         ON CONFLICT(job) DO UPDATE SET \
           state = 'ok', last_ok_at = excluded.last_ok_at, \
           next_run_at = excluded.next_run_at, updated_at = excluded.updated_at",
    )
    .bind(job)
    .bind(now)
    .bind(next_run_at)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark a job failed, recording the error and when it should be retried.
async fn mark_error(
    pool: &SqlitePool,
    job: &str,
    msg: &str,
    next_run_at: Option<i64>,
) -> sqlx::Result<()> {
    let now = now_ms();
    sqlx::query(
        "INSERT INTO data_status (job, state, last_error, last_error_at, next_run_at, updated_at) \
         VALUES (?, 'error', ?, ?, ?, ?) \
         ON CONFLICT(job) DO UPDATE SET \
           state = 'error', last_error = excluded.last_error, \
           last_error_at = excluded.last_error_at, next_run_at = excluded.next_run_at, \
           updated_at = excluded.updated_at",
    )
    .bind(job)
    .bind(msg)
    .bind(now)
    .bind(next_run_at)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Set only a job's `next_run_at` (creating an `idle` row if absent), leaving
/// any existing state untouched. `pub(crate)`: the add-symbol route uses it to
/// bring the history job forward so a newly added symbol is backfilled within
/// a tick instead of waiting out the ~6h interval.
pub(crate) async fn schedule_next(
    pool: &SqlitePool,
    job: &str,
    next_run_at: i64,
) -> sqlx::Result<()> {
    let now = now_ms();
    sqlx::query(
        "INSERT INTO data_status (job, state, next_run_at, updated_at) VALUES (?, 'idle', ?, ?) \
         ON CONFLICT(job) DO UPDATE SET \
           next_run_at = excluded.next_run_at, updated_at = excluded.updated_at",
    )
    .bind(job)
    .bind(next_run_at)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Whether `job` is due: no row yet, a null `next_run_at`, or one in the past.
async fn is_due(pool: &SqlitePool, job: &str, now: i64) -> sqlx::Result<bool> {
    let next: Option<Option<i64>> =
        sqlx::query_scalar("SELECT next_run_at FROM data_status WHERE job = ?")
            .bind(job)
            .fetch_optional(pool)
            .await?;
    Ok(match next {
        None | Some(None) => true,
        Some(Some(t)) => t <= now,
    })
}

/// Stamp a symbol as history-checked without storing bars: used when the
/// upstream returned a valid response that simply held nothing new.
async fn mark_history_checked(pool: &SqlitePool, ticker: &str) -> sqlx::Result<()> {
    let now = now_ms();
    sqlx::query("UPDATE symbols SET history_synced_at = ?, updated_at = ? WHERE ticker = ?")
        .bind(now)
        .bind(now)
        .bind(ticker)
        .execute(pool)
        .await?;
    Ok(())
}

/// Append one `fetch_log` row. `ticker` is left NULL: these are bulk jobs, so
/// a run logs once rather than once per symbol.
#[allow(clippy::too_many_arguments)]
async fn log_fetch(
    pool: &SqlitePool,
    job: &str,
    provider: &str,
    status: &str,
    detail: Option<&str>,
    rows: Option<i64>,
    duration_ms: i64,
    started_at: i64,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO fetch_log \
           (job, provider, ticker, status, detail, rows, duration_ms, started_at, finished_at) \
         VALUES (?, ?, NULL, ?, ?, ?, ?, ?, ?)",
    )
    .bind(job)
    .bind(provider)
    .bind(status)
    .bind(detail)
    .bind(rows)
    .bind(duration_ms)
    .bind(started_at)
    .bind(now_ms())
    .execute(pool)
    .await?;
    Ok(())
}
