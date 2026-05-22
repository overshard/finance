//! Data-source abstraction.
//!
//! Each upstream sits behind a trait so a source can be swapped without
//! touching callers. Phase 1 ships `HistoryProvider` (Stooq); Phase 5 adds
//! `QuoteProvider` (Yahoo); Phase 7 adds `FundamentalsProvider` (SEC EDGAR).

pub mod http;
pub mod sec;
pub mod stooq;
pub mod yahoo;

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;

/// One day of OHLCV, as delivered by a history source.
#[derive(Debug, Clone)]
pub struct DailyBar {
    /// Trading date, `YYYY-MM-DD`.
    pub d: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: i64,
}

/// Deep daily OHLCV history. Implemented by `StooqProvider`.
#[async_trait]
pub trait HistoryProvider: Send + Sync {
    fn name(&self) -> &'static str;

    /// Daily bars for `ticker`, oldest first. `since` (a `YYYY-MM-DD` date)
    /// trims the response to an incremental window when supplied.
    async fn daily(&self, ticker: &str, since: Option<&str>) -> Result<Vec<DailyBar>>;
}

/// A live quote snapshot from a quote source.
#[derive(Debug, Clone)]
pub struct Quote {
    pub price: f64,
    pub prev_close: Option<f64>,
    pub open: Option<f64>,
    pub day_high: Option<f64>,
    pub day_low: Option<f64>,
    pub volume: Option<i64>,
    /// The source's market-state label (e.g. `REGULAR`, `PRE`, `CLOSED`).
    pub market_state: Option<String>,
    /// The source's own timestamp for this quote, UTC epoch-ms.
    pub source_time: Option<i64>,
}

/// One intraday OHLCV bar — 15-minute granularity from Yahoo.
#[derive(Debug, Clone)]
pub struct IntradayBar {
    /// Bar start, UTC epoch-ms.
    pub ts: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: i64,
}

/// A quote source's full reply: the live quote plus the day's intraday bars,
/// both from one request.
#[derive(Debug, Clone)]
pub struct QuoteData {
    pub quote: Quote,
    pub bars: Vec<IntradayBar>,
}

/// Near-real-time quotes and intraday bars. Implemented by `YahooProvider`.
#[async_trait]
pub trait QuoteProvider: Send + Sync {
    fn name(&self) -> &'static str;

    /// The latest quote and the day's intraday bars for `ticker`.
    async fn quote(&self, ticker: &str) -> Result<QuoteData>;
}

/// One fundamental fact: a single metric for a single fiscal period, parsed
/// from a company's SEC XBRL facts.
#[derive(Debug, Clone)]
pub struct Fact {
    /// Our canonical metric name, e.g. `revenue`, `eps_diluted`.
    pub metric: String,
    /// Fiscal-period label: `FY2024` for a full year, `Q3-2024` for a quarter.
    pub period: String,
    pub fiscal_year: i64,
    /// `None` for a full-year figure.
    pub fiscal_qtr: Option<i64>,
    /// Period end, `YYYY-MM-DD`.
    pub period_end: String,
    pub value: f64,
    /// XBRL unit, e.g. `USD`, `USD/shares`, `shares`.
    pub unit: Option<String>,
    /// The form the figure was reported on, e.g. `10-K`.
    pub form: Option<String>,
    /// Filing date, `YYYY-MM-DD`.
    pub filed_at: Option<String>,
}

/// One SEC filing from a company's submission history.
#[derive(Debug, Clone)]
pub struct FilingRecord {
    pub accession: String,
    pub form: String,
    /// Filing date, `YYYY-MM-DD`.
    pub filed_at: String,
    /// The period the filing reports on, `YYYY-MM-DD`.
    pub period_of_report: Option<String>,
    pub primary_doc: Option<String>,
    /// Full URL to the filing's primary document (or index) on EDGAR.
    pub url: String,
    pub description: Option<String>,
}

/// Company fundamentals and filing history from SEC EDGAR. Implemented by
/// `SecProvider`. Stocks only; ETFs and indexes do not file.
#[async_trait]
pub trait FundamentalsProvider: Send + Sync {
    fn name(&self) -> &'static str;

    /// The whole-market ticker -> CIK map, from one bulk request. Keys are
    /// tickers normalised to bare uppercase alphanumerics (so our `BRK.B`
    /// matches EDGAR's `BRK-B`); values are 10-digit zero-padded CIKs.
    async fn cik_map(&self) -> Result<HashMap<String, String>>;

    /// XBRL fundamental facts for one company, by its 10-digit CIK.
    async fn facts(&self, cik: &str) -> Result<Vec<Fact>>;

    /// Recent filing history for one company, by its 10-digit CIK.
    async fn filings(&self, cik: &str) -> Result<Vec<FilingRecord>>;
}

/// An upstream rejected a request with an explicit rate-limit signal (HTTP 429
/// or 503). A provider returns this as the source of its `anyhow::Error` so the
/// `EndpointGuard` (see `src/guard.rs`) can recognise it by downcast and trip
/// the circuit breaker immediately, rather than waiting for a failure streak.
#[derive(Debug)]
pub struct RateLimited {
    /// The HTTP status that carried the signal.
    pub status: u16,
    /// `Retry-After` from the response, in seconds, when the upstream sent one
    /// in the numeric form. The HTTP-date form is not parsed (the guard's own
    /// exponential backoff covers it), so this is `None` then.
    pub retry_after_secs: Option<i64>,
}

impl std::fmt::Display for RateLimited {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "upstream rate-limited (HTTP {})", self.status)?;
        if let Some(s) = self.retry_after_secs {
            write!(f, ", Retry-After {s}s")?;
        }
        Ok(())
    }
}

impl std::error::Error for RateLimited {}
