//! In-process pub/sub hub for the live data stream.
//!
//! One [`Hub`] lives in `AppState`. The scheduler publishes quote and
//! market-session events into it; each `/stream` SSE connection subscribes and
//! forwards them to a browser.
//!
//! The hub also carries the **interest registry** — a count, per ticker, of
//! how many connected clients are currently displaying it. This is what makes
//! intraday polling demand-driven: the scheduler asks [`Hub::viewed`] for the
//! tickers with at least one live viewer and polls only those. With nobody
//! watching, `viewed()` is empty and the intraday job does no network work at
//! all (see `scheduler::run_intraday`).
//!
//! For a Python reader: this is a tiny in-memory pub/sub plus a `Counter` of
//! who's looking at what — no external broker, just a `tokio` broadcast
//! channel and a `Mutex<HashMap>`.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;
use tokio::sync::broadcast;

/// Broadcast channel depth. One full universe sweep (~144 quote events) sits
/// well under this; a subscriber that still lags is handled by skipping the
/// gap (see `routes::stream`), never by stalling a publisher.
const CHANNEL_CAPACITY: usize = 1024;

/// One message pushed to every connected `/stream` client. Serialized as the
/// SSE event payload; the `kind` tag is unused on the wire (the SSE `event:`
/// field carries the type) but keeps the JSON self-describing.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamEvent {
    Quote(QuoteUpdate),
    Market { session: String },
    /// A background-data state change — a job started or finished, a
    /// `fetch_log` row landed. Carries no payload: it is a nudge telling an
    /// open `/health` page to pull a fresh snapshot from `/api/health`.
    /// Published by the scheduler; see `routes::health`.
    Health,
}

/// A live quote, shaped for the browser to patch `data-field` nodes in place.
#[derive(Debug, Clone, Serialize)]
pub struct QuoteUpdate {
    pub ticker: String,
    pub price: f64,
    pub prev_close: Option<f64>,
    pub change_abs: Option<f64>,
    pub change_pct: Option<f64>,
    pub market_state: Option<String>,
}

impl QuoteUpdate {
    /// Build an update, deriving the day change from `price` and `prev_close`.
    pub fn new(
        ticker: String,
        price: f64,
        prev_close: Option<f64>,
        market_state: Option<String>,
    ) -> Self {
        let (change_abs, change_pct) = match prev_close {
            Some(p) if p != 0.0 => (Some(price - p), Some((price - p) / p * 100.0)),
            Some(p) => (Some(price - p), None),
            None => (None, None),
        };
        Self {
            ticker,
            price,
            prev_close,
            change_abs,
            change_pct,
            market_state,
        }
    }
}

/// The pub/sub hub plus the per-ticker viewer-interest registry.
pub struct Hub {
    tx: broadcast::Sender<StreamEvent>,
    /// ticker -> number of connected clients currently viewing it.
    interest: Mutex<HashMap<String, u32>>,
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}

impl Hub {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            tx,
            interest: Mutex::new(HashMap::new()),
        }
    }

    /// Subscribe a new client. The returned receiver yields every event
    /// published from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<StreamEvent> {
        self.tx.subscribe()
    }

    /// Publish an event to all subscribers. A send with no subscribers is not
    /// an error — it simply goes nowhere.
    pub fn publish(&self, event: StreamEvent) {
        let _ = self.tx.send(event);
    }

    /// Register that a client has begun viewing `tickers`.
    pub fn add_interest(&self, tickers: &[String]) {
        let mut map = self.interest.lock().unwrap();
        for t in tickers {
            *map.entry(t.clone()).or_insert(0) += 1;
        }
    }

    /// Drop a client's interest in `tickers` (called when its stream ends). A
    /// ticker's entry is removed once its viewer count falls back to zero, so
    /// `viewed()` stays a tight set.
    pub fn remove_interest(&self, tickers: &[String]) {
        let mut map = self.interest.lock().unwrap();
        for t in tickers {
            if let Some(n) = map.get_mut(t) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    map.remove(t);
                }
            }
        }
    }

    /// The tickers with at least one live viewer right now.
    pub fn viewed(&self) -> Vec<String> {
        self.interest.lock().unwrap().keys().cloned().collect()
    }
}
