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

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::{header::RETRY_AFTER, StatusCode};
use serde::Deserialize;

use crate::providers::{DividendEvent, IntradayBar, Quote, QuoteData, QuoteProvider, RateLimited};

/// Near-real-time quotes from Yahoo Finance.
pub struct YahooProvider {
    client: reqwest::Client,
}

impl YahooProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
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

impl YahooProvider {
    /// Fetch and parse the v8 chart payload for `ticker`.
    ///
    /// `Ok(Some(_))` is a real chart result; `Ok(None)` means Yahoo answered
    /// cleanly that it has no such symbol (a 404, or a `chart.error` body) —
    /// a definitive "unknown", not a failure. `Err` is a transport error or an
    /// explicit rate-limit signal, surfaced as the typed [`RateLimited`] so the
    /// endpoint guard trips its breaker at once.
    async fn fetch_chart(&self, ticker: &str) -> Result<Option<ChartResult>> {
        // `^` is not a bare path character; percent-encode the symbol.
        let sym = urlencoding::encode(&yahoo_symbol(ticker)).into_owned();
        let url = format!(
            "https://query1.finance.yahoo.com/v8/finance/chart/{sym}\
             ?interval=15m&range=1d&includePrePost=true"
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

#[async_trait]
impl QuoteProvider for YahooProvider {
    fn name(&self) -> &'static str {
        "yahoo"
    }

    async fn quote(&self, ticker: &str) -> Result<QuoteData> {
        match self.fetch_chart(ticker).await? {
            Some(result) => chart_to_quote_data(ticker, result),
            None => Err(anyhow!("yahoo returned no chart result for {ticker}")),
        }
    }
}
