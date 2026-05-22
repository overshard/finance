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

use std::collections::HashMap;
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
    self, stooq::StooqProvider, Fact, FilingRecord, FundamentalsProvider, HistoryProvider,
    IntradayBar, Quote, QuoteProvider,
};
use crate::stream::{Hub, QuoteUpdate, StreamEvent};
use crate::{seed, Config};

/// How often the loop wakes to check whether a job is due. The jobs themselves
/// run hours apart; a one-minute tick is plenty responsive and nearly free
/// (two small SELECTs per wake).
const TICK: Duration = Duration::from_secs(60);

/// Incremental daily-history refresh cadence.
const HISTORY_INTERVAL_SECS: i64 = 6 * 3600;

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

        let history_enabled = !config.stooq_apikey.is_empty();
        if !history_enabled {
            tracing::warn!(
                "[scheduler] STOOQ_APIKEY unset: seed and history refresh disabled, prune still runs"
            );
        } else if let Err(e) = run_boot_seed(&pool, &config, &hub).await {
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

            if history_enabled {
                match is_due(&pool, "history", now_ms()).await {
                    Ok(true) => {
                        if let Err(e) = run_history(&pool, &config, &hub).await {
                            tracing::warn!("[scheduler] history: {e:#}");
                        }
                    }
                    Ok(false) => {}
                    Err(e) => tracing::warn!("[scheduler] history due-check: {e}"),
                }
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

            // Intraday quotes: demand-driven (only symbols with a live viewer)
            // and gated to trading hours. Does no network work otherwise.
            if session.is_open() {
                if let Err(e) = run_intraday(&pool, &config, &hub).await {
                    tracing::warn!("[scheduler] intraday: {e:#}");
                }
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
    EndpointGuard::new(pool.clone(), "stooq")
        .ensure_registered()
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
        return Ok(());
    }
    tracing::info!("[scheduler] seed_completed unset: running first-run seed");

    let started = now_ms();
    mark_fetching(pool, "seed").await?;
    notify_health(hub);
    let stooq = StooqProvider::new(
        providers::http::build_client(config),
        config.stooq_apikey.clone(),
    );
    let t0 = Instant::now();
    let result = seed::run(pool, config, &stooq).await;
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
            log_fetch(pool, "seed", "stooq", "ok", Some(&detail), Some(with_history), dur, started)
                .await?;
            mark_ok(pool, "seed", None).await?;
        }
        Err(e) => {
            let msg = format!("{e:#}");
            log_fetch(pool, "seed", "stooq", "error", Some(&msg), None, dur, started).await?;
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
/// history has gone stale, asking Stooq for the window since each symbol's
/// last stored bar. Paced and circuit-broken like the seed.
async fn run_history(pool: &SqlitePool, config: &Config, hub: &Hub) -> anyhow::Result<()> {
    let started = now_ms();
    let next = started + HISTORY_INTERVAL_SECS * 1000;
    mark_fetching(pool, "history").await?;
    notify_health(hub);

    // Every symbol we track, curated or user-added, is eligible: a symbol
    // added through the Phase 9 add-symbol flow needs its history backfilled
    // exactly as a seeded one does. (The seed itself stays curated-list-only;
    // it is the only job that keys on `is_seeded`.) Futures (kind = 'future')
    // are excluded: Stooq has no `=F` history, so they are live-quotes only,
    // carried by the daily-close snapshot alone (see PLAN.md Phase 10).
    let cutoff = started - HISTORY_STALE_SECS * 1000;
    let stale: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT ticker, history_last_date FROM symbols \
         WHERE kind != 'future' \
           AND (history_synced_at IS NULL OR history_synced_at < ?) \
         ORDER BY ticker",
    )
    .bind(cutoff)
    .fetch_all(pool)
    .await?;

    if stale.is_empty() {
        log_fetch(pool, "history", "stooq", "ok", Some("no stale symbols"), Some(0), 0, started)
            .await?;
        mark_ok(pool, "history", Some(next)).await?;
        notify_health(hub);
        return Ok(());
    }
    tracing::info!("[scheduler] history: refreshing {} stale symbols", stale.len());

    let stooq = StooqProvider::new(
        providers::http::build_client(config),
        config.stooq_apikey.clone(),
    );
    let t0 = Instant::now();

    // Route every request through the persistent endpoint guard: it paces the
    // loop and refuses requests once the breaker opens or the hourly budget is
    // spent, so the job stops cleanly instead of hammering a guarded endpoint.
    let guard = EndpointGuard::new(pool.clone(), stooq.name());
    let mut ok = 0usize;
    let mut total_bars = 0i64;
    let mut stopped: Option<String> = None;

    for (ticker, last_date) in &stale {
        match guard.acquire().await? {
            Permit::Granted => {}
            Permit::Denied(why) => {
                stopped = Some(why);
                break;
            }
        }
        match stooq.daily(ticker, last_date.as_deref()).await {
            Ok(bars) if !bars.is_empty() => {
                guard.record_success().await?;
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
            log_fetch(pool, "history", "stooq", "skipped", Some(&detail), Some(total_bars), dur, started)
                .await?;
            mark_ok(pool, "history", Some(next)).await?;
        }
        None => {
            let detail = format!("{ok}/{} symbols refreshed, {total_bars} bars", stale.len());
            tracing::info!("[scheduler] history: {detail}");
            log_fetch(pool, "history", "stooq", "ok", Some(&detail), Some(total_bars), dur, started)
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
/// this returns at once having done no network work — that is the user's hard
/// rule: poll only what is on screen. Each tick re-sweeps the viewed set, so
/// one open symbol page refreshes about every minute, while an open dashboard
/// (~144 symbols) refreshes more slowly as the guard's 1.5s pacing carries it.
///
/// A clean run is recorded only in `data_status` (plus each `quotes.fetched_at`
/// row); a `fetch_log` row is written only for a notable run — an error or a
/// guard stop — so the minute-cadence job does not bury the log.
async fn run_intraday(pool: &SqlitePool, config: &Config, hub: &Hub) -> anyhow::Result<()> {
    let viewed = hub.viewed();
    if viewed.is_empty() {
        return Ok(());
    }
    let started = now_ms();
    mark_fetching(pool, "intraday").await?;
    notify_health(hub);

    let yahoo = YahooProvider::new(providers::http::build_client(config));
    let guard = EndpointGuard::with_budget(pool.clone(), yahoo.name(), YAHOO_BUDGET);
    let t0 = Instant::now();

    let mut ok = 0i64;
    let mut errors = 0i64;
    let mut stopped: Option<String> = None;

    for ticker in &viewed {
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
    let guard = EndpointGuard::with_budget(pool.clone(), yahoo.name(), YAHOO_BUDGET);
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

/// SEC fundamentals & filings sweep.
///
/// On the first run (and whenever new symbols appear) one bulk
/// `company_tickers.json` fetch fills in each stock's CIK. Then every stock
/// whose SEC data has gone stale is refreshed: its XBRL `companyfacts` into
/// `fundamentals` and its submission history into `filings`. ETFs and indexes
/// are skipped; they do not file with the SEC.
///
/// Resumable like the history job: each company's two timestamps
/// (`fundamentals_synced_at`, `filings_synced_at`) are stamped only on a
/// successful fetch, so a guard stop simply leaves the rest for the next cycle.
async fn run_sec(pool: &SqlitePool, config: &Config, hub: &Hub) -> anyhow::Result<()> {
    let started = now_ms();
    let next = started + SEC_INTERVAL_SECS * 1000;
    mark_fetching(pool, "sec").await?;
    notify_health(hub);

    let sec = SecProvider::new(providers::http::build_sec_client(config));
    let guard = EndpointGuard::with_budget(pool.clone(), sec.name(), SEC_BUDGET);
    let t0 = Instant::now();

    // 1. CIK resolution. One bulk call maps the whole market; only needed
    //    while some stock still lacks a CIK.
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

    // 2. Stale sweep. A company is due when either of its SEC timestamps is
    //    unset or older than the staleness window.
    let cutoff = started - SEC_STALE_SECS * 1000;
    let stale: Vec<(String, String, Option<i64>, Option<i64>)> = sqlx::query_as(
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

    if stale.is_empty() {
        log_fetch(pool, "sec", "sec", "ok", Some("no stale companies"), Some(0), 0, started)
            .await?;
        mark_ok(pool, "sec", Some(next)).await?;
        notify_health(hub);
        return Ok(());
    }
    tracing::info!("[scheduler] sec: refreshing {} companies", stale.len());

    // A company's metric is due when its timestamp is unset or past the cutoff.
    let due = |at: Option<i64>| at.map_or(true, |t| t < cutoff);
    let mut funds_ok = 0i64;
    let mut filings_ok = 0i64;
    let mut errors = 0i64;
    let mut stopped: Option<String> = None;

    'sweep: for (ticker, cik, f_at, fl_at) in &stale {
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
                    break 'sweep;
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
                    break 'sweep;
                }
            }
        }
    }

    let dur = t0.elapsed().as_millis() as i64;
    let counts = format!("{funds_ok} fundamentals, {filings_ok} filings, {errors} errors");
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

/// Stamp one of a symbol's SEC sync timestamps to now. `column` is one of two
/// hardcoded literals (never user input), so interpolating it is safe.
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
                primary_doc, url, description) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(ticker, accession) DO UPDATE SET \
               form = excluded.form, filed_at = excluded.filed_at, \
               period_of_report = excluded.period_of_report, \
               primary_doc = excluded.primary_doc, url = excluded.url, \
               description = excluded.description",
        )
        .bind(ticker)
        .bind(&f.accession)
        .bind(&f.form)
        .bind(&f.filed_at)
        .bind(&f.period_of_report)
        .bind(&f.primary_doc)
        .bind(&f.url)
        .bind(&f.description)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
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
