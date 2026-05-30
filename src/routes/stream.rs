//! `GET /stream` — the Server-Sent Events endpoint.
//!
//! A browser opens one `EventSource` here, passing `?symbols=` with the
//! tickers the page is showing. The connection:
//!  - registers viewer interest in those tickers with the [`Hub`], so the
//!    scheduler's intraday job knows to poll them (and unregisters on drop);
//!  - emits an initial `market` event and a `quote` snapshot so the page is
//!    immediately consistent with stored state;
//!  - then forwards live `quote` events for the registered tickers, and every
//!    `market` and `health` event, as the hub publishes them.

use std::collections::HashSet;
use std::convert::Infallible;
use std::sync::Arc;

use axum::{
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
    Router,
};
use futures_util::Stream;
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;

use crate::market;
use crate::stream::{Hub, QuoteUpdate, StreamEvent};
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/stream", get(stream))
}

#[derive(Deserialize)]
struct StreamQuery {
    /// Comma-separated tickers the client is displaying. Unknown tickers are
    /// dropped (see `validate_tickers`) so a client cannot steer the
    /// demand-driven poller at arbitrary upstream symbols.
    symbols: Option<String>,
}

/// Releases a connection's interest back to the hub when its SSE stream is
/// dropped — the client navigated away, closed the tab, or lost the network.
struct InterestGuard {
    hub: Arc<Hub>,
    tickers: Vec<String>,
}

impl Drop for InterestGuard {
    fn drop(&mut self) {
        self.hub.remove_interest(&self.tickers);
    }
}

async fn stream(
    Query(q): Query<StreamQuery>,
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let requested: Vec<String> = q
        .symbols
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty())
        .collect();
    let tickers = validate_tickers(&state, &requested).await;

    state.hub.add_interest(&tickers);
    let guard = InterestGuard {
        hub: state.hub.clone(),
        tickers: tickers.clone(),
    };
    let want: HashSet<String> = tickers.iter().cloned().collect();

    // Subscribe before snapshotting so no event published in between is lost.
    let rx = state.hub.subscribe();
    let snapshot = quote_snapshot(&state, &tickers).await;
    let session = market::session_at(chrono::Utc::now());

    let body = async_stream::stream! {
        // Holding the guard inside the stream ties interest to the stream's
        // lifetime: when the client disconnects, the stream drops, the guard
        // drops, and the interest is released.
        let _guard = guard;

        yield Ok::<_, Infallible>(sse_market(session.as_str()));
        for qu in &snapshot {
            yield Ok(sse_quote(qu));
        }

        let mut rx = rx;
        loop {
            match rx.recv().await {
                Ok(StreamEvent::Quote(qu)) => {
                    if want.contains(&qu.ticker) {
                        yield Ok(sse_quote(&qu));
                    }
                }
                Ok(StreamEvent::Market { session }) => {
                    yield Ok(sse_market(&session));
                }
                Ok(StreamEvent::Summary(s)) => {
                    yield Ok(sse_summary(&s));
                }
                Ok(StreamEvent::Health) => {
                    yield Ok(sse_health());
                }
                // A slow client fell behind the channel: skip the dropped
                // span and carry on rather than tearing the connection down.
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    };

    Sse::new(body).keep_alive(KeepAlive::default())
}

/// Keep only the requested tickers that are real seeded symbols, sorted and
/// deduped. Filtering here is what bounds the demand-driven poller to the
/// known universe.
async fn validate_tickers(state: &AppState, requested: &[String]) -> Vec<String> {
    if requested.is_empty() {
        return Vec::new();
    }
    let known: HashSet<String> = sqlx::query_scalar("SELECT ticker FROM symbols")
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();
    let mut out: Vec<String> = requested
        .iter()
        .filter(|t| known.contains(*t))
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The latest stored quote for each requested ticker, so a freshly connected
/// page can reconcile immediately without waiting for the next poll. The
/// `quotes` table holds one row per symbol (~144 max), so a full scan is cheap.
async fn quote_snapshot(state: &AppState, tickers: &[String]) -> Vec<QuoteUpdate> {
    if tickers.is_empty() {
        return Vec::new();
    }
    let want: HashSet<&str> = tickers.iter().map(String::as_str).collect();
    let rows: Vec<(String, f64, Option<f64>, Option<String>)> =
        sqlx::query_as("SELECT ticker, price, prev_close, market_state FROM quotes")
            .fetch_all(&state.pool)
            .await
            .unwrap_or_default();
    rows.into_iter()
        .filter(|(t, ..)| want.contains(t.as_str()))
        .map(|(t, price, prev, state)| QuoteUpdate::new(t, price, prev, state))
        .collect()
}

fn sse_quote(qu: &QuoteUpdate) -> Event {
    Event::default()
        .event("quote")
        .data(serde_json::to_string(qu).unwrap_or_default())
}

fn sse_market(session: &str) -> Event {
    Event::default()
        .event("market")
        .data(format!("{{\"session\":\"{session}\"}}"))
}

/// The recomputed dashboard market summary (Phase 7). An open `/` page patches
/// its hero verdict + headline figures + breadth from this payload.
fn sse_summary(s: &crate::summary::MarketSummary) -> Event {
    Event::default()
        .event("summary")
        .data(serde_json::to_string(s).unwrap_or_default())
}

/// A content-free nudge: an open `/health` page answers it by pulling a fresh
/// snapshot from `/api/health`. See `routes::health`.
fn sse_health() -> Event {
    Event::default().event("health").data("{}")
}
