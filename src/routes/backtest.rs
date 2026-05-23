//! `/backtest` — stress-test the picker (Phase 30).
//!
//! Replays the current pick rankers over historical `daily_prices` and shows
//! "what would $X have done if you'd followed today's algo over the past
//! few years?". Page is a small chart shell; the heavy lifting lives in
//! `picks::run_backtest`, served as JSON at
//! `GET /api/backtest?horizon=<key>&capital=<usd>`.
//!
//! The fundamental signals are today's values (we do not store a per-period
//! history of Phase 7 ratios), which is acknowledged look-ahead bias —
//! surfaced explicitly in the page's disclaimer. The user's own framing of
//! the feature is "for fun and testing", so this is the right trade-off for
//! v1; a future phase can layer in point-in-time fundamentals.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;

use crate::picks::{self, HORIZONS};
use crate::render::render;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/backtest", get(backtest_page))
        .route("/api/backtest", get(backtest_api))
}

async fn backtest_page(State(state): State<AppState>) -> Response {
    let extra = minijinja::context! {
        title => "Backtest",
        horizons => HORIZONS,
    };
    render(&state, "pages/backtest.html", "/backtest", extra)
}

#[derive(Debug, Deserialize)]
struct BacktestQuery {
    /// One of `day | week | month | year`. Defaults to `month`, the
    /// medium-cadence read that has both enough rebalances to be informative
    /// and few enough to be quick to glance at.
    horizon: Option<String>,
    /// Starting capital in USD; defaults to $10,000 (the same anchor as the
    /// Phase 28 growth-of-$10k chart).
    capital: Option<f64>,
}

/// Run the requested horizon's backtest and return its full result as JSON.
/// One heavy DB scan per request (the curated stocks' full close history);
/// not cached, since the data turns over once a day and the page is
/// operator-facing.
async fn backtest_api(
    State(state): State<AppState>,
    Query(q): Query<BacktestQuery>,
) -> Response {
    let key = q.horizon.unwrap_or_else(|| "month".to_string());
    // Match by key against the static HORIZONS list so the JSON carries the
    // canonical horizon metadata (label + description) instead of just an
    // echoed query string.
    let Some(horizon) = HORIZONS.iter().copied().find(|h| h.key == key) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "unknown horizon"})),
        )
            .into_response();
    };
    // Clamp the capital below at $1: a zero or negative starting capital
    // makes the equity curve meaningless and the CAGR explode.
    let capital = q.capital.filter(|c| *c >= 1.0).unwrap_or(10_000.0);

    let (bundles, bench) = match picks::load_hist_bundles(&state.pool).await {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!("backtest load: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "load failed"})),
            )
                .into_response();
        }
    };
    let result = picks::run_backtest(&bundles, &bench, horizon, capital);
    Json(result).into_response()
}
