//! `GET /health` — the data-health page — and `GET /api/health`, its JSON feed.
//!
//! The page lays the background-data machinery open: each endpoint guard's
//! circuit-breaker state and how much of its per-hour request budget is spent,
//! every scheduler job's state / last success / next run, and a tail of the
//! `fetch_log`.
//!
//! The page route renders an initial snapshot embedded in the HTML so it draws
//! without a round trip. From there the page stays live off the Phase 5 SSE
//! hub: the scheduler publishes a `health` event whenever a job changes state
//! or logs a row, and the page answers each one by re-pulling `/api/health`.
//! Both routes build the same [`Health`] snapshot via [`snapshot`].

use axum::{extract::State, response::Response, routing::get, Json, Router};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::db::now_ms;
use crate::render::render;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(page))
        .route("/api/health", get(api))
}

/// The whole data-health picture in one payload: every endpoint guard, every
/// scheduler job, and a tail of the fetch log.
#[derive(Serialize)]
struct Health {
    /// When this snapshot was built, UTC epoch-ms.
    generated_at: i64,
    /// True while any job sits in the `fetching` state — drives the page's
    /// live "fetching now" banner.
    fetching: bool,
    endpoints: Vec<Endpoint>,
    jobs: Vec<Job>,
    log: Vec<LogRow>,
}

/// One upstream's request guard, shaped for the page.
#[derive(Serialize)]
struct Endpoint {
    endpoint: String,
    label: String,
    /// `closed` | `open` | `half_open`.
    state: String,
    fail_streak: i64,
    trip_count: i64,
    opened_at: Option<i64>,
    retry_at: Option<i64>,
    /// Requests let through in the current rolling budget hour.
    hour_count: i64,
    /// Start of that hour, UTC epoch-ms (the budget resets an hour later).
    hour_start: Option<i64>,
    hourly_budget: i64,
    /// `hour_count` as a 0..100 share of the budget, for the meter fill.
    budget_pct: f64,
    last_ok_at: Option<i64>,
    last_error: Option<String>,
    last_error_at: Option<i64>,
}

/// One scheduler job's `data_status` row, with a human label and description.
#[derive(Serialize)]
struct Job {
    job: String,
    label: String,
    description: String,
    /// `idle` | `fetching` | `ok` | `stale` | `error`.
    state: String,
    last_ok_at: Option<i64>,
    last_error: Option<String>,
    last_error_at: Option<i64>,
    next_run_at: Option<i64>,
    updated_at: i64,
}

/// One `fetch_log` row — a passthrough of the table, newest first.
#[derive(Serialize, sqlx::FromRow)]
struct LogRow {
    job: String,
    provider: String,
    ticker: Option<String>,
    /// `ok` | `error` | `skipped`.
    status: String,
    detail: Option<String>,
    rows: Option<i64>,
    duration_ms: Option<i64>,
    started_at: i64,
}

/// The `endpoint_guard` columns the page needs.
#[derive(sqlx::FromRow)]
struct GuardRow {
    endpoint: String,
    state: String,
    fail_streak: i64,
    trip_count: i64,
    opened_at: Option<i64>,
    retry_at: Option<i64>,
    hour_start: Option<i64>,
    hour_count: i64,
    hourly_budget: i64,
    last_ok_at: Option<i64>,
    last_error: Option<String>,
    last_error_at: Option<i64>,
}

/// The `data_status` columns the page needs.
#[derive(sqlx::FromRow)]
struct StatusRow {
    job: String,
    state: String,
    last_ok_at: Option<i64>,
    last_error: Option<String>,
    last_error_at: Option<i64>,
    next_run_at: Option<i64>,
    updated_at: i64,
}

/// Build the full health snapshot. Three small reads — `endpoint_guard` holds
/// a handful of rows, `data_status` one per job, and the log tail is capped.
async fn snapshot(pool: &SqlitePool) -> Health {
    let guards: Vec<GuardRow> = sqlx::query_as(
        "SELECT endpoint, state, fail_streak, trip_count, opened_at, retry_at, \
                hour_start, hour_count, hourly_budget, last_ok_at, last_error, last_error_at \
         FROM endpoint_guard ORDER BY endpoint",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let statuses: Vec<StatusRow> = sqlx::query_as(
        "SELECT job, state, last_ok_at, last_error, last_error_at, next_run_at, updated_at \
         FROM data_status",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let log: Vec<LogRow> = sqlx::query_as(
        "SELECT job, provider, ticker, status, detail, rows, duration_ms, started_at \
         FROM fetch_log ORDER BY started_at DESC LIMIT 50",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let endpoints: Vec<Endpoint> = guards.into_iter().map(to_endpoint).collect();

    let mut jobs: Vec<Job> = statuses.into_iter().map(to_job).collect();
    jobs.sort_by_key(|j| job_rank(&j.job));
    let fetching = jobs.iter().any(|j| j.state == "fetching");

    Health {
        generated_at: now_ms(),
        fetching,
        endpoints,
        jobs,
        log,
    }
}

fn to_endpoint(g: GuardRow) -> Endpoint {
    // Rounded to two places: this only drives a meter's CSS width, and a tidy
    // figure keeps the JSON payload readable.
    let budget_pct = if g.hourly_budget > 0 {
        let raw = g.hour_count as f64 / g.hourly_budget as f64 * 100.0;
        (raw.clamp(0.0, 100.0) * 100.0).round() / 100.0
    } else {
        0.0
    };
    Endpoint {
        label: endpoint_label(&g.endpoint).to_string(),
        endpoint: g.endpoint,
        state: g.state,
        fail_streak: g.fail_streak,
        trip_count: g.trip_count,
        opened_at: g.opened_at,
        retry_at: g.retry_at,
        hour_start: g.hour_start,
        hour_count: g.hour_count,
        hourly_budget: g.hourly_budget,
        budget_pct,
        last_ok_at: g.last_ok_at,
        last_error: g.last_error,
        last_error_at: g.last_error_at,
    }
}

fn to_job(s: StatusRow) -> Job {
    let (label, description) = job_meta(&s.job);
    Job {
        label: label.to_string(),
        description: description.to_string(),
        job: s.job,
        state: s.state,
        last_ok_at: s.last_ok_at,
        last_error: s.last_error,
        last_error_at: s.last_error_at,
        next_run_at: s.next_run_at,
        updated_at: s.updated_at,
    }
}

/// A human label for a known upstream id.
fn endpoint_label(endpoint: &str) -> &str {
    match endpoint {
        "yahoo" => "Yahoo Finance · quotes, intraday & daily history",
        "sec" => "SEC EDGAR · fundamentals & filings",
        other => other,
    }
}

/// Human label and one-line description per scheduler job. An unknown job (a
/// future one) falls back to its raw id and no description.
fn job_meta(job: &str) -> (&str, &str) {
    match job {
        "seed" => (
            "Universe seed",
            "First-run import of the curated symbol list and its deep daily history.",
        ),
        "history" => (
            "Daily history",
            "Backstop incremental daily-bar refresh from Yahoo, roughly every 6 hours.",
        ),
        "intraday" => (
            "Intraday quotes",
            "Live quotes from Yahoo for the symbols on screen, during market hours.",
        ),
        "daily_close" => (
            "Daily close",
            "Once-a-day closing snapshot of the whole universe, just after the bell.",
        ),
        "sec" => (
            "Fundamentals & filings",
            "SEC EDGAR company facts and filing history for each stock, refreshed weekly.",
        ),
        "dividends" => (
            "Dividend payouts",
            "Per-payout dividend / distribution history from Yahoo for each \
             stock and ETF, refreshed weekly.",
        ),
        "fund_metadata" => (
            "ETF fund metadata",
            "Yahoo quoteSummary snapshot for each ETF — expense ratio, yield, \
             NAV, inception, category, fund family, strategy. Refreshed monthly.",
        ),
        "fund_nav" => (
            "ETF NAV",
            "Yahoo quoteSummary NAV for each ETF, refreshed daily — keeps the \
             price-vs-NAV premium behind the ETF quality read's tracking factor \
             current (the monthly metadata sweep's NAV is too stale for that).",
        ),
        "earnings_calendar" => (
            "Earnings calendar",
            "Yahoo quoteSummary `calendarEvents` for each stock — the next \
             expected earnings date. Refreshed monthly and whenever the \
             stored date passes.",
        ),
        "asset_profile" => (
            "Stock sector & industry",
            "Yahoo quoteSummary `assetProfile` for each stock — sector and \
             industry classification. Refreshed monthly.",
        ),
        other => (other, ""),
    }
}

/// Display order for the jobs list: the data pipeline's own order.
fn job_rank(job: &str) -> u8 {
    match job {
        "seed" => 0,
        "history" => 1,
        "sec" => 2,
        "fund_metadata" => 3,
        "fund_nav" => 4,
        "asset_profile" => 5,
        "earnings_calendar" => 6,
        "dividends" => 7,
        "intraday" => 8,
        "daily_close" => 9,
        _ => 10,
    }
}

/// `GET /health` — the page, with the current snapshot embedded so it renders
/// without a round trip; the page's script keeps it live from there.
async fn page(State(state): State<AppState>) -> Response {
    let snap = snapshot(&state.pool).await;
    let json = serde_json::to_string(&snap).unwrap_or_else(|_| "null".to_string());
    let extra = minijinja::context! {
        title => "Data health",
        health_json => embed_json(&json),
    };
    render(&state, "pages/health.html", "/health", extra)
}

/// `GET /api/health` — the same snapshot as JSON, polled by the page whenever
/// the SSE hub signals a data change.
async fn api(State(state): State<AppState>) -> Json<Health> {
    Json(snapshot(&state.pool).await)
}

/// Escape a JSON string for safe embedding inside a `<script>` element. Only
/// `<`, `>` and `&` matter (they could otherwise close the tag or open a
/// comment); replaced with their `\uXXXX` forms, which a JSON parser reads back
/// identically. Structural JSON never contains these characters, so a blanket
/// replace touches only string contents.
fn embed_json(json: &str) -> String {
    json.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}
