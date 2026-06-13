//! Live quotes and intraday bars from Yahoo Finance's v8 chart endpoint.
//!
//! `https://query1.finance.yahoo.com/v8/finance/chart/<symbol>` needs no API
//! key — just an ordinary browser User-Agent, which the shared client already
//! sends. One call with `interval=15m&range=1d` returns the day's 15-minute
//! bars *and* a `meta` block carrying the live quote, so a single request
//! feeds both the `quotes` snapshot and `intraday_bars`.
//!
//! The same endpoint also identifies a symbol — its name, instrument type,
//! exchange and currency all sit in `meta` — so [`YahooProvider::lookup`]
//! reuses it to validate and describe a symbol the add-symbol flow (Phase 9)
//! is about to register.

use std::sync::Mutex;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::{header::RETRY_AFTER, StatusCode};
use serde::Deserialize;

use crate::providers::{
    AssetProfile, DailyBar, DividendEvent, FundMetadata, HistoryProvider, IntradayBar, Quote,
    QuoteData, QuoteProvider, RateLimited,
};

/// Near-real-time quotes from Yahoo Finance.
pub struct YahooProvider {
    client: reqwest::Client,
    /// Cached crumb token for Yahoo's v10 `quoteSummary` endpoint, which is
    /// crumb-gated and refuses (401) without it. Lazy: only the first
    /// `quoteSummary` call pays the round-trip, then every subsequent call
    /// in the process replays the cached token until Yahoo invalidates it.
    crumb: Mutex<Option<String>>,
}

impl YahooProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            crumb: Mutex::new(None),
        }
    }

    /// Return a Yahoo `crumb` token, fetching one if the cache is empty.
    /// The dance is two requests: a primer to `fc.yahoo.com` that drops a
    /// session cookie (handled by reqwest's cookie jar — see `http.rs`),
    /// then a GET to `/v1/test/getcrumb` that replays the cookie and
    /// returns the crumb as a plain-text body. A failure here propagates
    /// as a regular `RateLimited` / transport error so the caller can feed
    /// the endpoint guard the same way as a v10 call would.
    async fn ensure_crumb(&self) -> Result<String> {
        if let Some(c) = self.crumb.lock().expect("crumb cache").clone() {
            return Ok(c);
        }
        // Primer: any 2xx body is fine, we only want the Set-Cookie.
        let _ = self
            .client
            .get("https://fc.yahoo.com/")
            .send()
            .await?
            .error_for_status();
        // The crumb endpoint returns the token as its raw body.
        let resp = self
            .client
            .get("https://query1.finance.yahoo.com/v1/test/getcrumb")
            .send()
            .await?;
        let status = resp.status();
        if matches!(
            status,
            StatusCode::TOO_MANY_REQUESTS
                | StatusCode::SERVICE_UNAVAILABLE
                | StatusCode::UNAUTHORIZED
                | StatusCode::FORBIDDEN
        ) {
            let retry_after_secs = resp
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<i64>().ok());
            return Err(anyhow::Error::new(RateLimited {
                status: status.as_u16(),
                retry_after_secs,
            }));
        }
        let crumb = resp.error_for_status()?.text().await?.trim().to_string();
        if crumb.is_empty() {
            return Err(anyhow!("yahoo returned an empty crumb"));
        }
        *self.crumb.lock().expect("crumb cache") = Some(crumb.clone());
        Ok(crumb)
    }

    /// Forget the cached crumb. Called on a 401 from a v10 call so the next
    /// attempt fetches a fresh one (Yahoo crumbs rotate occasionally).
    fn invalidate_crumb(&self) {
        *self.crumb.lock().expect("crumb cache") = None;
    }
}

/// Map a canonical ticker to Yahoo's symbol scheme.
/// - most indexes already match Yahoo (`^DJI`, `^NDX`, `^RUT`, `^VIX`)
/// - two differ: our Stooq-style `^SPX` / `^NDQ` are `^GSPC` / `^IXIC` on Yahoo
/// - stocks and ETFs are the plain ticker with `.` rewritten to `-`
///   (`BRK.B` -> `BRK-B`)
fn yahoo_symbol(ticker: &str) -> String {
    match ticker {
        "^SPX" => "^GSPC".to_string(),
        "^NDQ" => "^IXIC".to_string(),
        t if t.starts_with('^') => t.to_string(),
        t => t.replace('.', "-"),
    }
}

// ── identity of a looked-up symbol ─────────────────────────────────────────

/// Identifying metadata for a symbol, derived from Yahoo's chart `meta`. The
/// add-symbol flow uses it to register a new symbol with a real name and kind
/// rather than a bare ticker.
#[derive(Debug, Clone)]
pub struct SymbolInfo {
    pub name: String,
    /// One of `stock` | `etf` | `index` | `future`.
    pub kind: String,
    pub exchange: Option<String>,
    pub currency: String,
}

/// The outcome of [`YahooProvider::lookup`]. An `Err` from `lookup` is a
/// genuine transport / rate-limit failure (and should feed the endpoint
/// guard); these three variants are all successful answers from Yahoo.
#[derive(Debug)]
pub enum SymbolLookup {
    /// Yahoo knows this symbol: its identity, plus the quote and intraday bars
    /// the same request returned.
    Found { info: SymbolInfo, data: QuoteData },
    /// Yahoo has no such symbol.
    Unknown,
    /// Yahoo knows the symbol but it is an instrument type this app does not
    /// model yet (a currency pair, a cryptocurrency, ...). Carries the raw type.
    Unsupported(String),
}

// ── the slice of the v8 chart JSON we read ─────────────────────────────────

#[derive(Deserialize)]
struct ChartEnvelope {
    chart: Chart,
}

#[derive(Deserialize)]
struct Chart {
    result: Option<Vec<ChartResult>>,
    /// Non-null on a logical failure (e.g. an unknown symbol).
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ChartResult {
    meta: Meta,
    /// Bar-start times, Unix seconds. Absent when the day has no bars yet.
    timestamp: Option<Vec<i64>>,
    indicators: Indicators,
    /// `events.dividends` carries declared payouts when the request asked for
    /// `events=div` (Phase 26). Absent on a routine quote fetch.
    events: Option<ChartEvents>,
}

/// The events block of a Yahoo chart payload. Each value of `dividends` is
/// keyed by the event's Unix-second timestamp (a JSON string, which is why
/// the outer type is a map).
#[derive(Default, Deserialize)]
struct ChartEvents {
    #[serde(default)]
    dividends: std::collections::HashMap<String, ChartDividend>,
}

#[derive(Deserialize)]
struct ChartDividend {
    /// Per-share amount.
    amount: f64,
    /// Ex-dividend date as a Unix second. Yahoo also echoes the timestamp as
    /// the map key, but the inner field is the canonical one to read off.
    date: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Meta {
    regular_market_price: Option<f64>,
    previous_close: Option<f64>,
    chart_previous_close: Option<f64>,
    regular_market_open: Option<f64>,
    regular_market_day_high: Option<f64>,
    regular_market_day_low: Option<f64>,
    regular_market_volume: Option<i64>,
    /// The source's own timestamp for the quote, Unix seconds.
    regular_market_time: Option<i64>,
    /// Seconds east of UTC for the exchange's timezone (e.g. -14400 for ET in
    /// summer). Daily-bar timestamps are bucketed by the exchange's local day,
    /// so the daily-history parser adds this offset before taking the date.
    gmtoffset: Option<i64>,
    market_state: Option<String>,
    /// Identity fields — read only by `lookup`. `EQUITY` | `ETF` | `INDEX` |
    /// `MUTUALFUND` | `FUTURE` | `CURRENCY` | `CRYPTOCURRENCY` | ...
    instrument_type: Option<String>,
    short_name: Option<String>,
    long_name: Option<String>,
    currency: Option<String>,
    exchange_name: Option<String>,
    full_exchange_name: Option<String>,
}

#[derive(Deserialize)]
struct Indicators {
    quote: Vec<OhlcvArrays>,
}

/// Column-oriented OHLCV: one parallel array per field, indexed by bar. A cell
/// can be null when a bar has no print, so every element is optional.
#[derive(Default, Deserialize)]
struct OhlcvArrays {
    #[serde(default)]
    open: Vec<Option<f64>>,
    #[serde(default)]
    high: Vec<Option<f64>>,
    #[serde(default)]
    low: Vec<Option<f64>>,
    #[serde(default)]
    close: Vec<Option<f64>>,
    #[serde(default)]
    volume: Vec<Option<i64>>,
}

/// One row in a market-movers list (top gainers / losers / most active), from the
/// predefined-screener endpoint. A plain snapshot for the dashboard, cached as
/// JSON in `meta` (hence `Deserialize` too), not a stored table row.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Mover {
    pub symbol: String,
    pub name: String,
    pub price: Option<f64>,
    /// Day % move, already a percent (Yahoo returns e.g. 20.08, not 0.2008).
    pub change_pct: Option<f64>,
    pub volume: Option<i64>,
}

#[derive(Deserialize)]
struct ScreenerEnvelope {
    finance: ScreenerFinance,
}
#[derive(Deserialize)]
struct ScreenerFinance {
    #[serde(default)]
    result: Vec<ScreenerResult>,
}
#[derive(Deserialize)]
struct ScreenerResult {
    #[serde(default)]
    quotes: Vec<ScreenerQuote>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScreenerQuote {
    symbol: String,
    short_name: Option<String>,
    long_name: Option<String>,
    display_name: Option<String>,
    regular_market_price: Option<f64>,
    regular_market_change_percent: Option<f64>,
    regular_market_volume: Option<i64>,
}

impl YahooProvider {
    /// Fetch a predefined market-movers screener (`day_gainers` / `day_losers` /
    /// `most_actives`), `count` rows. One unauthenticated GET (the predefined
    /// screeners are not crumb-gated). A 429/503/401/403 surfaces as the typed
    /// [`RateLimited`] so the endpoint guard trips its breaker at once, exactly
    /// like the chart calls; anything else parses to the rows (empty on a shape
    /// we don't recognise).
    pub async fn fetch_movers(&self, scr_id: &str, count: u32) -> Result<Vec<Mover>> {
        let url = format!(
            "https://query1.finance.yahoo.com/v1/finance/screener/predefined/saved\
             ?count={count}&scrIds={scr_id}"
        );
        let resp = self.client.get(&url).send().await?;
        let status = resp.status();
        if matches!(
            status,
            StatusCode::TOO_MANY_REQUESTS
                | StatusCode::SERVICE_UNAVAILABLE
                | StatusCode::UNAUTHORIZED
                | StatusCode::FORBIDDEN
        ) {
            let retry_after_secs = resp
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<i64>().ok());
            return Err(anyhow::Error::new(RateLimited {
                status: status.as_u16(),
                retry_after_secs,
            }));
        }
        let resp = resp.error_for_status()?;
        let env: ScreenerEnvelope = resp.json().await?;
        let quotes = env
            .finance
            .result
            .into_iter()
            .next()
            .map(|r| r.quotes)
            .unwrap_or_default();
        Ok(quotes
            .into_iter()
            .map(|q| {
                let name = q
                    .short_name
                    .or(q.display_name)
                    .or(q.long_name)
                    .unwrap_or_else(|| q.symbol.clone());
                Mover {
                    symbol: q.symbol,
                    name,
                    price: q.regular_market_price,
                    change_pct: q.regular_market_change_percent,
                    volume: q.regular_market_volume,
                }
            })
            .collect())
    }

    /// Fetch and parse the v8 chart payload for `ticker`.
    ///
    /// `Ok(Some(_))` is a real chart result; `Ok(None)` means Yahoo answered
    /// cleanly that it has no such symbol (a 404, or a `chart.error` body) —
    /// a definitive "unknown", not a failure. `Err` is a transport error or an
    /// explicit rate-limit signal, surfaced as the typed [`RateLimited`] so the
    /// endpoint guard trips its breaker at once.
    async fn fetch_chart(&self, ticker: &str) -> Result<Option<ChartResult>> {
        self.fetch_chart_range(ticker, "1d").await
    }

    /// Fetch the v8 chart payload for `ticker` over an explicit intraday `range`
    /// (e.g. `1d` for the routine quote, `5d` to backfill the whole trading week
    /// for the end-of-week dashboard view). `interval=15m` is held constant, so
    /// `5d` returns five trading days of 15-minute bars in one request.
    async fn fetch_chart_range(&self, ticker: &str, range: &str) -> Result<Option<ChartResult>> {
        // `^` is not a bare path character; percent-encode the symbol.
        let sym = urlencoding::encode(&yahoo_symbol(ticker)).into_owned();
        let url = format!(
            "https://query1.finance.yahoo.com/v8/finance/chart/{sym}\
             ?interval=15m&range={range}&includePrePost=true"
        );
        self.request_chart(&url).await
    }

    /// Fetch a symbol's deep daily OHLCV history from the same v8 chart
    /// endpoint, one bar per trading day. `since` (a `YYYY-MM-DD` date) selects
    /// an incremental window via Yahoo's `period1`/`period2` epoch params;
    /// `None` asks for the full `range=max` history. A single call returns the
    /// entire deep history (or the window), which is why Yahoo replaced the
    /// per-symbol Stooq fetch the app used through Phase 1.
    ///
    /// Error / "unknown symbol" semantics match [`Self::fetch_chart`].
    async fn fetch_daily(&self, ticker: &str, since: Option<&str>) -> Result<Option<ChartResult>> {
        let sym = urlencoding::encode(&yahoo_symbol(ticker)).into_owned();
        let chart_url = |window: &str| {
            format!("https://query1.finance.yahoo.com/v8/finance/chart/{sym}?interval=1d&{window}")
        };
        match since {
            Some(d) => {
                // Re-fetch from the day before `since` to "now + 1 day" (epoch
                // seconds). The day-before back-step and the day-ahead end
                // cover same-day re-runs and exchange-timezone edges; the
                // daily upsert makes any overlap free.
                let p1 = chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
                    .ok()
                    .and_then(|dt| dt.pred_opt())
                    .and_then(|dt| dt.and_hms_opt(0, 0, 0))
                    .map(|dt| dt.and_utc().timestamp())
                    .unwrap_or(0);
                let p2 = chrono::Utc::now().timestamp() + 86_400;
                self.request_chart(&chart_url(&format!("period1={p1}&period2={p2}")))
                    .await
            }
            None => {
                // Deep backfill: ask for the full history. Yahoo honours
                // `interval=1d` at `range=max` for most symbols (full daily
                // history, e.g. ^SPX back to the 1700s), but silently
                // downsamples it to monthly / quarterly bars for some index and
                // futures symbols (^RUT, ^VIX, the `=F` futures) even though a
                // daily interval was requested. When that happens, refetch a
                // bounded 10-year window, which Yahoo *does* serve at daily
                // granularity — far better for a daily chart than coarse
                // max-range bars. (Costs one extra request for those few
                // symbols, once per deep backfill.)
                let res = self.request_chart(&chart_url("range=max")).await?;
                match &res {
                    Some(r) if is_downsampled(r) => {
                        self.request_chart(&chart_url("range=10y")).await
                    }
                    _ => Ok(res),
                }
            }
        }
    }

    /// GET a v8 chart URL and parse it into at most one [`ChartResult`].
    ///
    /// `Ok(Some(_))` is a real result; `Ok(None)` means Yahoo answered cleanly
    /// that it has no such symbol (a 404, or a `chart.error` body) — a
    /// definitive "unknown", not a failure. `Err` is a transport error or an
    /// explicit rate-limit signal, surfaced as the typed [`RateLimited`] so the
    /// endpoint guard trips its breaker at once.
    async fn request_chart(&self, url: &str) -> Result<Option<ChartResult>> {
        let resp = self.client.get(url).send().await?;
        let status = resp.status();

        if status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::SERVICE_UNAVAILABLE {
            let retry_after_secs = resp
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<i64>().ok());
            return Err(anyhow::Error::new(RateLimited {
                status: status.as_u16(),
                retry_after_secs,
            }));
        }
        // Yahoo answers an unknown symbol with 404 and a `chart.error` body —
        // a definitive "no such symbol", not a transport failure.
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let resp = resp.error_for_status()?;
        let env: ChartEnvelope = resp.json().await?;
        if env.chart.error.is_some() {
            return Ok(None);
        }
        Ok(env
            .chart
            .result
            .and_then(|mut r| if r.is_empty() { None } else { Some(r.remove(0)) }))
    }

    /// Fetch the declared dividend history for `ticker` (Phase 26).
    ///
    /// The same v8 chart endpoint that serves quotes carries an
    /// `events.dividends` series when the request asks for `events=div`. Ask
    /// for a five-year window at daily granularity: that is plenty for the
    /// page's prior-year + YTD totals and a long history list, while keeping
    /// the payload modest (the candle stream itself is discarded here — only
    /// the events block is parsed). Returns the payouts oldest first.
    ///
    /// Error semantics mirror [`Self::quote`]: a 429/503 surfaces as
    /// [`RateLimited`] so the endpoint guard trips at once; an unknown symbol
    /// (404 or `chart.error`) returns an empty vec, not an error, since the
    /// guard should not treat it as a transport failure.
    pub async fn dividends(&self, ticker: &str) -> Result<Vec<DividendEvent>> {
        let sym = urlencoding::encode(&yahoo_symbol(ticker)).into_owned();
        let url = format!(
            "https://query1.finance.yahoo.com/v8/finance/chart/{sym}\
             ?interval=1d&range=5y&events=div"
        );
        let resp = self.client.get(&url).send().await?;
        let status = resp.status();
        if status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::SERVICE_UNAVAILABLE {
            let retry_after_secs = resp
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<i64>().ok());
            return Err(anyhow::Error::new(RateLimited {
                status: status.as_u16(),
                retry_after_secs,
            }));
        }
        if status == StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        let resp = resp.error_for_status()?;
        let env: ChartEnvelope = resp.json().await?;
        if env.chart.error.is_some() {
            return Ok(Vec::new());
        }
        let Some(result) = env
            .chart
            .result
            .and_then(|mut r| if r.is_empty() { None } else { Some(r.remove(0)) })
        else {
            return Ok(Vec::new());
        };
        let mut out: Vec<DividendEvent> = result
            .events
            .unwrap_or_default()
            .dividends
            .into_values()
            .filter_map(|d| {
                // A non-positive amount or a nonsense timestamp is filtered;
                // Yahoo has occasionally emitted a literal 0 placeholder.
                if d.amount <= 0.0 {
                    return None;
                }
                let ex_date = chrono::DateTime::from_timestamp(d.date, 0)?
                    .format("%Y-%m-%d")
                    .to_string();
                Some(DividendEvent {
                    ex_date,
                    amount: d.amount,
                })
            })
            .collect();
        out.sort_by(|a, b| a.ex_date.cmp(&b.ex_date));
        Ok(out)
    }

    /// Fetch the Yahoo `quoteSummary` ETF metadata snapshot for `ticker`
    /// (Phase 28). One request to `v10/finance/quoteSummary` pulls the five
    /// modules that together carry every figure the Phase 28 ETF page needs
    /// beyond what SEC N-PORT already provides — expense ratio, distribution
    /// yield, latest NAV, inception, category, fund family, and the issuer's
    /// strategy paragraph.
    ///
    /// Returns `Ok(None)` when Yahoo answers cleanly that it has no such
    /// symbol (a 404 or a `quoteSummary.error` body) — a definitive empty,
    /// not a guard failure. Yahoo's gating responses (`429`, `503`, and `401
    /// "Invalid Crumb"` which the gate sometimes returns as either) surface
    /// as the typed [`RateLimited`] so the endpoint guard trips at once. The
    /// returned [`FundMetadata`] may carry only a subset of fields populated
    /// — Yahoo's coverage is uneven across small ETFs.
    pub async fn fund_metadata(&self, ticker: &str) -> Result<Option<FundMetadata>> {
        // The five modules that between them carry every Phase 28 field. A
        // module Yahoo does not recognise for this symbol is silently
        // omitted from the response (rather than failing the whole request).
        let Some(result) = self
            .quote_summary(
                ticker,
                "fundProfile,defaultKeyStatistics,summaryDetail,price,assetProfile",
            )
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(parse_fund_metadata(result)))
    }

    /// Fetch just the ETF's latest NAV (net asset value per share) via the v10
    /// `quoteSummary` `summaryDetail` / `price` modules — the two that carry
    /// `navPrice`. The Phase 4 daily NAV refresh calls this so the price-vs-NAV
    /// premium behind the ETF quality read's tracking factor stays current,
    /// without re-pulling the static fields the 30-day `fund_metadata` sweep
    /// owns. `Ok(None)` is a clean empty (unknown symbol or no NAV reported);
    /// gating responses (429 / 503 / 401 / 403) surface as the typed
    /// [`RateLimited`], the same defensive set as `fund_metadata`.
    pub async fn fund_nav(&self, ticker: &str) -> Result<Option<f64>> {
        let Some(result) = self.quote_summary(ticker, "summaryDetail,price").await? else {
            return Ok(None);
        };
        let sd = result.summary_detail.unwrap_or_default();
        let price = result.price.unwrap_or_default();
        Ok(sd.nav_price.or(price.nav_price).map(|v| v.0))
    }

    /// Shared v10 `quoteSummary` fetch: ensures the crumb is cached, builds
    /// the URL with `&crumb=...`, parses gating responses (429 / 503 / 401 /
    /// 403) as the typed [`RateLimited`], and treats Yahoo's "unknown
    /// symbol" replies (404 or a `quoteSummary.error` body) as a clean
    /// `Ok(None)`. A bare 401 on a previously-good crumb is retried once
    /// with a fresh one — Yahoo rotates crumbs and this masks the rotation
    /// from the endpoint guard.
    async fn quote_summary(
        &self,
        ticker: &str,
        modules: &str,
    ) -> Result<Option<QuoteSummaryResult>> {
        let sym = urlencoding::encode(&yahoo_symbol(ticker)).into_owned();
        // First attempt with the cached crumb (may fetch one on the first
        // call of the process).
        match self.quote_summary_once(&sym, modules).await {
            Ok(v) => Ok(v),
            Err(e) => {
                // A 401/403 on a request we sent a crumb with means the
                // crumb expired. Drop it and try one more time so the
                // caller sees a clean answer instead of a guard trip.
                let retry = e.downcast_ref::<RateLimited>().is_some_and(|r| {
                    matches!(r.status, 401 | 403)
                });
                if retry {
                    self.invalidate_crumb();
                    return self.quote_summary_once(&sym, modules).await;
                }
                Err(e)
            }
        }
    }

    async fn quote_summary_once(
        &self,
        sym: &str,
        modules: &str,
    ) -> Result<Option<QuoteSummaryResult>> {
        let crumb = self.ensure_crumb().await?;
        let url = format!(
            "https://query1.finance.yahoo.com/v10/finance/quoteSummary/{sym}\
             ?modules={modules}&crumb={c}",
            c = urlencoding::encode(&crumb),
        );
        let resp = self.client.get(&url).send().await?;
        let status = resp.status();
        if matches!(
            status,
            StatusCode::TOO_MANY_REQUESTS
                | StatusCode::SERVICE_UNAVAILABLE
                | StatusCode::UNAUTHORIZED
                | StatusCode::FORBIDDEN
        ) {
            let retry_after_secs = resp
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<i64>().ok());
            return Err(anyhow::Error::new(RateLimited {
                status: status.as_u16(),
                retry_after_secs,
            }));
        }
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let resp = resp.error_for_status()?;
        let env: QuoteSummaryEnvelope = resp.json().await?;
        if env.quote_summary.error.is_some() {
            return Ok(None);
        }
        Ok(env
            .quote_summary
            .result
            .and_then(|mut r| if r.is_empty() { None } else { Some(r.remove(0)) }))
    }

    /// Fetch the next-expected earnings date for `ticker` from Yahoo's
    /// `quoteSummary.calendarEvents` module (Phase 25). One request to the
    /// same v10 endpoint that already serves `fund_metadata`, asking only for
    /// the calendar module — Yahoo's smallest reply on this endpoint.
    ///
    /// Returns `Ok(Some(epoch_ms))` when Yahoo has an upcoming earnings date,
    /// `Ok(None)` when it knows the symbol but carries no date (Yahoo's
    /// coverage is uneven on small caps, and a closely-watched name with a
    /// just-passed print also briefly reads empty), or `Ok(None)` for an
    /// unknown symbol (404 or `quoteSummary.error`). Gating responses (429 /
    /// 503 / 401 / 403) surface as the typed [`RateLimited`], same defensive
    /// set as `fund_metadata`.
    pub async fn earnings_calendar(&self, ticker: &str) -> Result<Option<i64>> {
        let Some(result) = self.quote_summary(ticker, "calendarEvents").await? else {
            return Ok(None);
        };
        // `earningsDate` is an array; Yahoo populates 1 or 2 entries — the
        // confirmed date, or a confirmed/estimated pair. The earliest one
        // is the upcoming print. Future events only: a date in the past
        // means Yahoo has not yet rolled it forward, so we ignore it.
        let now_secs = chrono::Utc::now().timestamp();
        let next_secs = result
            .calendar_events
            .and_then(|c| c.earnings)
            .and_then(|e| {
                e.earnings_date
                    .into_iter()
                    .filter_map(|d| Some(d.0 as i64))
                    .filter(|s| *s >= now_secs)
                    .min()
            });
        Ok(next_secs.map(|s| s * 1000))
    }

    /// Fetch a stock's sector and industry classification from Yahoo's
    /// `quoteSummary.assetProfile` module (Phase 15). One request to the same
    /// v10 endpoint that serves `fund_metadata` and `earnings_calendar`,
    /// asking only for the `assetProfile` module — Yahoo's smallest reply
    /// for this concern.
    ///
    /// Returns `Ok(Some(profile))` with whichever of `sector` / `industry`
    /// Yahoo carries (a small cap with a partial profile leaves the absent
    /// field `None`); `Ok(None)` when Yahoo cleanly does not know the
    /// symbol (404 or `quoteSummary.error`); gating responses (429 / 503 /
    /// 401 / 403) surface as the typed [`RateLimited`] so the endpoint
    /// guard trips at once.
    pub async fn asset_profile(&self, ticker: &str) -> Result<Option<AssetProfile>> {
        let Some(result) = self.quote_summary(ticker, "assetProfile").await? else {
            return Ok(None);
        };
        let ap = result.asset_profile.unwrap_or_default();
        let trim = |s: Option<String>| s.map(|x| x.trim().to_string()).filter(|x| !x.is_empty());
        Ok(Some(AssetProfile {
            sector: trim(ap.sector),
            industry: trim(ap.industry),
        }))
    }

    /// Identify a symbol: validate it exists on Yahoo and return its name,
    /// kind, exchange and currency, alongside the quote the same request
    /// carried. Used by the Phase 9 add-symbol flow.
    pub async fn lookup(&self, ticker: &str) -> Result<SymbolLookup> {
        let Some(result) = self.fetch_chart(ticker).await? else {
            return Ok(SymbolLookup::Unknown);
        };
        // Derive identity from `meta` before the result is consumed for the
        // quote. An instrument type we do not model is a clean rejection.
        let Some(info) = symbol_info(ticker, &result.meta) else {
            let raw = result.meta.instrument_type.clone().unwrap_or_default();
            return Ok(SymbolLookup::Unsupported(raw));
        };
        let data = chart_to_quote_data(ticker, result)?;
        Ok(SymbolLookup::Found { info, data })
    }
}

// ── v10 quoteSummary response (Phase 28) ───────────────────────────────────
//
// Yahoo wraps most numeric fields as `{"raw": ..., "fmt": "..."}`. A small
// `RawF64` / `RawI64` carrier lets one serde derive handle every field of
// that shape; an absent field, an unparsable one, or one missing the inner
// `raw` is None — Yahoo's coverage is uneven and a partial snapshot is
// still useful, so the parser keeps what it has rather than failing whole.

#[derive(Deserialize)]
struct QuoteSummaryEnvelope {
    #[serde(rename = "quoteSummary")]
    quote_summary: QuoteSummary,
}

#[derive(Deserialize)]
struct QuoteSummary {
    result: Option<Vec<QuoteSummaryResult>>,
    /// Non-null on a logical failure (e.g. an unknown symbol — Yahoo returns
    /// `200 OK` with an `error` body, not a 404).
    error: Option<serde_json::Value>,
}

/// One module bag from the `quoteSummary` response. Every module is optional
/// — Yahoo silently drops a module it does not recognise for this symbol
/// rather than failing the whole request.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct QuoteSummaryResult {
    fund_profile: Option<FundProfileModule>,
    default_key_statistics: Option<DefaultKeyStatisticsModule>,
    summary_detail: Option<SummaryDetailModule>,
    price: Option<PriceModule>,
    asset_profile: Option<AssetProfileModule>,
    /// `calendarEvents` (Phase 25) — the upcoming earnings date and ex-div
    /// date a v10 request can carry alongside the fund modules.
    calendar_events: Option<CalendarEventsModule>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct CalendarEventsModule {
    earnings: Option<CalendarEarnings>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct CalendarEarnings {
    /// Yahoo emits 1 or 2 `RawF64` entries (Unix seconds), the confirmed
    /// upcoming date or a confirmed/estimated pair.
    earnings_date: Vec<RawF64>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct FundProfileModule {
    family: Option<String>,
    category_name: Option<String>,
    fees_expenses_investment: Option<FeesExpensesInvestment>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct FeesExpensesInvestment {
    annual_report_expense_ratio: Option<RawF64>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct DefaultKeyStatisticsModule {
    /// Yahoo timestamps inception in either the unix-seconds `{raw, fmt}`
    /// shape or, on some funds, an `fmt`-only ISO-ish string. Accept both.
    fund_inception_date: Option<RawF64>,
    #[serde(rename = "yield")]
    yield_pct: Option<RawF64>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct SummaryDetailModule {
    #[serde(rename = "yield")]
    yield_pct: Option<RawF64>,
    trailing_annual_dividend_yield: Option<RawF64>,
    nav_price: Option<RawF64>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct PriceModule {
    nav_price: Option<RawF64>,
    first_trade_date_milliseconds: Option<RawF64>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct AssetProfileModule {
    long_business_summary: Option<String>,
    /// GICS-style sector ("Technology"). Stocks only; Yahoo leaves this
    /// blank or omits the module entirely on ETFs / indexes / funds.
    sector: Option<String>,
    /// GICS-style industry ("Consumer Electronics"). See `sector`.
    industry: Option<String>,
}

/// Yahoo's `{ "raw": ..., "fmt": "..." }` numeric carrier. Deserialises from
/// either form: the wrapped object, a bare number, or a string that parses
/// as a number. Missing or unparsable -> `None`.
#[derive(Debug, Clone, Copy)]
struct RawF64(f64);

impl<'de> serde::Deserialize<'de> for RawF64 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wrap {
            raw: Option<f64>,
        }
        // `untagged` lets serde try each variant in order; the first one to
        // deserialise cleanly wins.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Any {
            Wrapped(Wrap),
            Bare(f64),
            Str(String),
        }
        let any = Any::deserialize(d)?;
        let v = match any {
            Any::Wrapped(w) => w.raw,
            Any::Bare(v) => Some(v),
            Any::Str(s) => s.trim().parse().ok(),
        };
        v.map(RawF64).ok_or_else(|| {
            // Yahoo sometimes serves `{}` for a missing field; surface as a
            // deserialiser error so serde's outer Option<RawF64> on each
            // field captures it as None rather than failing the whole parse.
            serde::de::Error::custom("missing raw")
        })
    }
}

/// Build a [`FundMetadata`] from one parsed `quoteSummary` result. Every
/// field is best-effort: a missing module or field just leaves its slot
/// `None` rather than rejecting the row.
fn parse_fund_metadata(r: QuoteSummaryResult) -> FundMetadata {
    let fp = r.fund_profile.unwrap_or_default();
    let dks = r.default_key_statistics.unwrap_or_default();
    let sd = r.summary_detail.unwrap_or_default();
    let price = r.price.unwrap_or_default();
    let ap = r.asset_profile.unwrap_or_default();

    let expense_ratio = fp
        .fees_expenses_investment
        .and_then(|f| f.annual_report_expense_ratio)
        .map(|v| v.0);
    // `summaryDetail.yield` is the live figure; `defaultKeyStatistics.yield`
    // is the same number on most funds but missing on some, so fall back.
    let yield_pct = sd
        .yield_pct
        .or(dks.yield_pct)
        .map(|v| v.0);
    let trailing_yield_pct = sd.trailing_annual_dividend_yield.map(|v| v.0);
    let nav_price = sd.nav_price.or(price.nav_price).map(|v| v.0);
    // Inception comes in two shapes: a unix-seconds carrier (older API),
    // or — on some funds — a unix-ms carrier from `price`. Normalise to a
    // `YYYY-MM-DD` date in UTC; the inception itself is day-precision.
    let inception_date = dks
        .fund_inception_date
        .map(|v| v.0 as i64)
        .or_else(|| {
            price
                .first_trade_date_milliseconds
                // Heuristic: a value > 10^11 is ms, else seconds. ETF
                // inceptions are post-1989 so both shapes are plausible.
                .map(|v| {
                    let n = v.0 as i64;
                    if n.abs() > 100_000_000_000 { n / 1000 } else { n }
                })
        })
        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
        .map(|dt| dt.format("%Y-%m-%d").to_string());

    let non_empty = |s: Option<String>| s.filter(|x| !x.trim().is_empty());
    FundMetadata {
        expense_ratio,
        yield_pct,
        trailing_yield_pct,
        nav_price,
        inception_date,
        category: non_empty(fp.category_name),
        fund_family: non_empty(fp.family),
        strategy_summary: non_empty(ap.long_business_summary),
    }
}

/// Build a `SymbolInfo` from a chart `meta`. `None` for an instrument type
/// this app does not model yet (currencies, crypto, ...).
fn symbol_info(ticker: &str, meta: &Meta) -> Option<SymbolInfo> {
    let kind = match meta
        .instrument_type
        .as_deref()
        .map(str::to_uppercase)
        .as_deref()
    {
        Some("EQUITY") => "stock",
        Some("ETF") | Some("MUTUALFUND") => "etf",
        Some("INDEX") => "index",
        Some("FUTURE") => "future",
        // No instrument type at all: fall back to the ticker's shape — a `^`
        // prefix is an index, a Yahoo `=F` suffix a future, else a stock.
        None => {
            if ticker.starts_with('^') {
                "index"
            } else if ticker.ends_with("=F") {
                "future"
            } else {
                "stock"
            }
        }
        // A type we do not model yet (CURRENCY, CRYPTOCURRENCY, ...).
        Some(_) => return None,
    };
    let non_empty = |s: &String| !s.trim().is_empty();
    let name = meta
        .long_name
        .clone()
        .or_else(|| meta.short_name.clone())
        .filter(non_empty)
        .unwrap_or_else(|| ticker.to_string());
    let exchange = meta
        .full_exchange_name
        .clone()
        .or_else(|| meta.exchange_name.clone())
        .filter(non_empty);
    let currency = meta
        .currency
        .clone()
        .filter(non_empty)
        .unwrap_or_else(|| "USD".to_string());
    Some(SymbolInfo {
        name,
        kind: kind.to_string(),
        exchange,
        currency,
    })
}

/// Turn a parsed chart result into a `QuoteData` (live quote + intraday bars).
fn chart_to_quote_data(ticker: &str, result: ChartResult) -> Result<QuoteData> {
    let meta = result.meta;
    let price = meta
        .regular_market_price
        .ok_or_else(|| anyhow!("yahoo quote for {ticker} carried no price"))?;
    let quote = Quote {
        price,
        prev_close: meta.previous_close.or(meta.chart_previous_close),
        open: meta.regular_market_open,
        day_high: meta.regular_market_day_high,
        day_low: meta.regular_market_day_low,
        volume: meta.regular_market_volume,
        market_state: meta.market_state,
        source_time: meta.regular_market_time.map(|t| t * 1000),
    };

    // The day's intraday bars, zipping the timestamp column against the OHLCV
    // columns. A bar missing any OHLC cell is skipped.
    let mut bars = Vec::new();
    if let (Some(ts), Some(o)) = (result.timestamp, result.indicators.quote.into_iter().next()) {
        for (i, &t) in ts.iter().enumerate() {
            let cell = |v: &[Option<f64>]| v.get(i).copied().flatten();
            let (Some(open), Some(high), Some(low), Some(close)) =
                (cell(&o.open), cell(&o.high), cell(&o.low), cell(&o.close))
            else {
                continue;
            };
            let volume = o.volume.get(i).copied().flatten().unwrap_or(0);
            bars.push(IntradayBar {
                ts: t * 1000,
                open,
                high,
                low,
                close,
                volume,
            });
        }
    }

    Ok(QuoteData { quote, bars })
}

/// True when a supposedly-daily chart response is actually coarser than daily.
///
/// Yahoo silently downsamples `interval=1d&range=max` to monthly / quarterly
/// bars for some index and futures symbols (^RUT, ^VIX, the `=F` futures),
/// ignoring the requested interval. Detect it from the *median* spacing of the
/// returned timestamps: the *median* gap of a genuine daily series is 1 day
/// (consecutive trading days; only weekends and holidays stretch it to 3-4),
/// while a weekly series medians ~7 days and a monthly one ~30. A median over
/// four days therefore means the series is coarser than daily. Fewer than three
/// bars carries no usable spacing, so it is treated as fine.
fn is_downsampled(result: &ChartResult) -> bool {
    let Some(ts) = result.timestamp.as_ref() else {
        return false;
    };
    if ts.len() < 3 {
        return false;
    }
    let mut gaps: Vec<i64> = ts.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_unstable();
    let median = gaps[gaps.len() / 2];
    median > 4 * 86_400
}

/// Turn a parsed `interval=1d` chart result into daily bars, oldest first.
///
/// Yahoo timestamps each daily bar at the start of the trading day in UTC
/// seconds; adding the exchange's `gmtoffset` before formatting yields the
/// local trading date (so a bar that starts 14:30 UTC reads as the right ET
/// day). A bar missing any of open/high/low/close is skipped.
fn chart_to_daily(result: ChartResult) -> Vec<DailyBar> {
    let off = result.meta.gmtoffset.unwrap_or(0);
    let (Some(ts), Some(o)) = (result.timestamp, result.indicators.quote.into_iter().next()) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(ts.len());
    for (i, &t) in ts.iter().enumerate() {
        let cell = |v: &[Option<f64>]| v.get(i).copied().flatten();
        let (Some(open), Some(high), Some(low), Some(close)) =
            (cell(&o.open), cell(&o.high), cell(&o.low), cell(&o.close))
        else {
            continue;
        };
        let Some(d) = chrono::DateTime::from_timestamp(t + off, 0)
            .map(|dt| dt.format("%Y-%m-%d").to_string())
        else {
            continue;
        };
        let volume = o.volume.get(i).copied().flatten().unwrap_or(0);
        out.push(DailyBar {
            d,
            open,
            high,
            low,
            close,
            volume,
        });
    }
    out
}

#[async_trait]
impl QuoteProvider for YahooProvider {
    async fn quote(&self, ticker: &str) -> Result<QuoteData> {
        match self.fetch_chart(ticker).await? {
            Some(result) => chart_to_quote_data(ticker, result),
            None => Err(anyhow!("yahoo returned no chart result for {ticker}")),
        }
    }
}

impl YahooProvider {
    /// Fetch a wider intraday window (e.g. `range=5d`) of 15-minute bars in one
    /// request, used to backfill the whole trading week for the end-of-week
    /// dashboard view. Same shape as [`Self::quote`] (live quote + bars); callers
    /// store only the bars.
    pub async fn intraday_window(&self, ticker: &str, range: &str) -> Result<QuoteData> {
        match self.fetch_chart_range(ticker, range).await? {
            Some(result) => chart_to_quote_data(ticker, result),
            None => Err(anyhow!("yahoo returned no chart result for {ticker}")),
        }
    }
}

#[async_trait]
impl HistoryProvider for YahooProvider {
    fn name(&self) -> &'static str {
        "yahoo"
    }

    async fn daily(&self, ticker: &str, since: Option<&str>) -> Result<Vec<DailyBar>> {
        // An unknown / historyless symbol returns an empty vec (a clean empty,
        // not a guard failure) — same contract the Stooq provider had.
        match self.fetch_daily(ticker, since).await? {
            Some(result) => Ok(chart_to_daily(result)),
            None => Ok(Vec::new()),
        }
    }
}
