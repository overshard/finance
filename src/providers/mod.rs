//! Data-source abstraction.
//!
//! Each upstream sits behind a trait so a source can be swapped without
//! touching callers. `QuoteProvider` + `HistoryProvider` (both Yahoo) cover
//! live quotes/intraday and deep daily history; `FundamentalsProvider` (SEC
//! EDGAR) covers stock fundamentals, filings, leadership, and ETF profiles.
//! (Stooq was the original history source; it was dropped 2026-05-30 — see
//! PLAN.md's data-source policy.)

pub mod http;
pub mod sec;
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

/// Deep daily OHLCV history. Implemented by `YahooProvider` (one
/// `interval=1d` chart call returns a symbol's whole history, or an
/// incremental window when `since` is given).
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
    /// For an 8-K, the reported item codes, comma-separated as EDGAR lists them
    /// (e.g. `5.02,9.01`); `None` for other forms. Item 5.02 is the
    /// officer/director change the leadership-changes feed keys on (Phase 14).
    pub items: Option<String>,
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

// ── company leadership (Phase 14) ──────────────────────────────────────────
//
// A company's officers and board come from SEC Form 3/4/5 ownership filings:
// every director and Section-16 officer must file these, and each carries a
// structured `reportingOwnerRelationship`. Like the N-PORT fund methods, the
// leadership methods are inherent to `SecProvider` (this is wholly EDGAR
// territory), but their data types sit here beside `FilingRecord`.

/// One Form 3/4/5 ownership filing in a company's submission history — the
/// pointer the scheduler needs to fetch and parse the ownership XML itself.
#[derive(Debug, Clone)]
pub struct OwnershipFiling {
    pub accession: String,
    /// Filing date, `YYYY-MM-DD`.
    pub filed_at: String,
    /// The ownership XML's file name within the filing's Archives directory.
    pub primary_doc: String,
}

/// One insider parsed from an ownership XML's `reportingOwnerRelationship`.
#[derive(Debug, Clone)]
pub struct OwnershipPerson {
    /// Name as filed: last-name-first and upper-case (`COOK TIMOTHY D`).
    pub name: String,
    pub is_director: bool,
    pub is_officer: bool,
    /// The officer title, present when `is_officer` and the filer gave one.
    pub officer_title: Option<String>,
}

// ── ETF fund profiles (Phase 18) ───────────────────────────────────────────
//
// ETFs file as registered funds: their portfolio comes from quarterly N-PORT
// filings, not the XBRL companyfacts behind the stock fundamentals above. The
// fund methods live on `SecProvider` as inherent methods (N-PORT is wholly
// SEC-specific, with no second source to abstract over), but their data types
// sit here next to `Fact` / `FilingRecord` for the scheduler and routes.

/// Identifies one ETF to the SEC's fund endpoints.
#[derive(Debug, Clone)]
pub struct FundId {
    /// 10-digit zero-padded registrant CIK.
    pub cik: String,
    /// SEC series id (e.g. `S000002839`), present when the registrant hosts
    /// more than one fund — then it, not the CIK, pins a lookup to this ETF.
    pub series_id: Option<String>,
}

/// One portfolio holding parsed from an N-PORT filing.
#[derive(Debug, Clone)]
pub struct FundHolding {
    /// Issuer / security name as the fund reported it.
    pub name: String,
    /// Percent of the fund's net assets, e.g. `8.4`.
    pub pct: Option<f64>,
    /// Market value of the position, USD.
    pub value_usd: Option<f64>,
    /// N-PORT asset-category code (`EC` equity, `DBT` debt, ...), for the mix.
    pub asset_cat: Option<String>,
}

/// What a fund's filing history reveals about how to read its portfolio.
#[derive(Debug, Clone)]
pub enum FundShape {
    /// A fund that files N-PORT: fetch this filing for its holdings. The value
    /// is the filing's EDGAR index-page URL, whose directory also holds the
    /// N-PORT XML — and which carries the registrant CIK even when a filing
    /// agent (not the fund) is the named filer on the accession number.
    Portfolio { nport_href: String },
    /// A physical-commodity grantor trust (GLD, SLV): no N-PORT — it holds
    /// bullion, not a securities portfolio — so AUM comes from its 10-K.
    CommodityTrust,
    /// Neither pattern matched; the page can still show the filing list.
    Unknown,
}

/// The filing list for an ETF plus what it implies about the fund's shape.
#[derive(Debug, Clone)]
pub struct FundFilings {
    pub filings: Vec<FilingRecord>,
    pub shape: FundShape,
}

/// A fund's portfolio snapshot, parsed from one N-PORT filing.
#[derive(Debug, Clone, Default)]
pub struct PortfolioData {
    /// Total net assets (AUM), USD.
    pub net_assets: Option<f64>,
    /// Gross assets, USD.
    pub total_assets: Option<f64>,
    /// The date the holdings are reported as of, `YYYY-MM-DD`.
    pub report_date: Option<String>,
    /// Positions in the full portfolio (not just the top slice kept below).
    pub holdings_count: i64,
    /// The largest holdings by weight, largest first.
    pub top_holdings: Vec<FundHolding>,
    /// Asset-class mix as `(bucket, percent)` pairs, largest bucket first.
    pub asset_mix: Vec<(String, f64)>,
    /// Sector mix derived from each holding's N-PORT `industryCode` (or
    /// `assetCat` fallback for non-equity buckets), aggregated as
    /// `(label, percent)` pairs largest first. Phase 28; empty on a
    /// commodity-trust fund (no N-PORT).
    pub sector_mix: Vec<(String, f64)>,
    /// Geography mix derived from each holding's issuer country, same shape
    /// as `sector_mix`. Phase 28; empty on a commodity trust.
    pub geography_mix: Vec<(String, f64)>,
}

// ── dividend events (Phase 26) ─────────────────────────────────────────────
//
// Per-payout dividend history comes from Yahoo's chart endpoint, which carries
// an `events.dividends` series alongside the price bars when asked for
// `events=div`. SEC XBRL's `DividendsPerShare` (already in `fundamentals`) is
// per fiscal period, not per payout date, so it does not stand in. The fetch
// lives on `YahooProvider` as an inherent method (one source); the type sits
// here next to `Quote`/`IntradayBar` for the scheduler and routes.

/// One declared dividend payment, as carried by Yahoo's chart event series.
#[derive(Debug, Clone)]
pub struct DividendEvent {
    /// Ex-dividend date, `YYYY-MM-DD`. The first trading day a new buyer does
    /// NOT receive the upcoming payment — Yahoo timestamps each event by it.
    pub ex_date: String,
    /// Per-share amount, in the symbol's reporting currency.
    pub amount: f64,
}

// ── ETF fund metadata (Phase 28) ───────────────────────────────────────────
//
// The slow-moving figures the prospectus carries that N-PORT does not — expense
// ratio, distribution yield, inception, category, fund family, the issuer's
// strategy paragraph — plus the intraday NAV used for the premium / discount
// read. Yahoo's `v10/finance/quoteSummary` endpoint serves all of them in one
// request behind the `fundProfile + defaultKeyStatistics + summaryDetail +
// price + assetProfile` modules. The fetch lives on `YahooProvider` as an
// inherent method (one source); the type sits here next to `Quote` /
// `DividendEvent` for the scheduler and routes.

/// One ETF's Yahoo `quoteSummary` snapshot. Every field is optional: Yahoo's
/// coverage is uneven and a small fund may carry only a subset, but a partial
/// snapshot is still useful, so the parser keeps what it has rather than
/// rejecting the row.
#[derive(Debug, Clone, Default)]
pub struct FundMetadata {
    /// Annual expense ratio as a decimal, e.g. `0.0003` = 0.03%. From
    /// `fundProfile.feesExpensesInvestment.annualReportExpenseRatio`.
    pub expense_ratio: Option<f64>,
    /// Forward / trailing distribution yield as a decimal. From
    /// `summaryDetail.yield` (preferred) or `defaultKeyStatistics.yield`.
    pub yield_pct: Option<f64>,
    /// Trailing-twelve-month distribution yield as a decimal. From
    /// `summaryDetail.trailingAnnualDividendYield`.
    pub trailing_yield_pct: Option<f64>,
    /// Latest NAV from `price.navPrice` or `summaryDetail.navPrice`. USD.
    pub nav_price: Option<f64>,
    /// Inception / first trade date as `YYYY-MM-DD`. From
    /// `defaultKeyStatistics.fundInceptionDate` or `price.firstTradeDateEpochUtc`.
    pub inception_date: Option<String>,
    /// Morningstar-style fund category, e.g. "Large Blend". From
    /// `fundProfile.categoryName`.
    pub category: Option<String>,
    /// Sponsor family, e.g. "Vanguard". From `fundProfile.family`.
    pub fund_family: Option<String>,
    /// The fund's strategy paragraph as the issuer writes it. From
    /// `assetProfile.longBusinessSummary` (preferred) or
    /// `summaryProfile.longBusinessSummary`.
    pub strategy_summary: Option<String>,
}

/// One stock's Yahoo `assetProfile` classification (Phase 15). Both fields
/// are optional: small-cap and foreign tickers occasionally carry only one,
/// and a request that returned an `assetProfile` module Yahoo populated only
/// partially leaves the other side `None` rather than rejecting the row.
#[derive(Debug, Clone, Default)]
pub struct AssetProfile {
    /// GICS-style sector ("Technology"). From `assetProfile.sector`.
    pub sector: Option<String>,
    /// GICS-style industry ("Consumer Electronics"). From
    /// `assetProfile.industry`.
    pub industry: Option<String>,
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
