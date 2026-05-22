use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::{header::RETRY_AFTER, StatusCode};

use crate::providers::{DailyBar, HistoryProvider, RateLimited};

/// Deep daily history from Stooq's per-ticker CSV endpoint.
///
/// Stooq now gates this endpoint behind an apikey (obtained once via a captcha
/// on stooq.com). The key is read from `STOOQ_APIKEY` and kept out of the repo.
pub struct StooqProvider {
    client: reqwest::Client,
    apikey: String,
}

impl StooqProvider {
    pub fn new(client: reqwest::Client, apikey: String) -> Self {
        Self { client, apikey }
    }
}

/// Map a canonical ticker to Stooq's symbol scheme.
/// - indexes keep their `^` prefix, lowercased (`^SPX` -> `^spx`)
/// - everything else is a US listing: lowercased with `.us`, and any `.`
///   in the ticker becomes `-` (`BRK.B` -> `brk-b.us`)
fn stooq_symbol(ticker: &str) -> String {
    if let Some(rest) = ticker.strip_prefix('^') {
        format!("^{}", rest.to_lowercase())
    } else {
        format!("{}.us", ticker.replace('.', "-").to_lowercase())
    }
}

/// Parse a Stooq daily CSV body. Stooq returns a plain-text message (not CSV)
/// on errors such as a missing apikey or a hit-rate limit, so a missing header
/// is surfaced as an error.
fn parse_csv(text: &str) -> Result<Vec<DailyBar>> {
    let trimmed = text.trim_start();
    if !trimmed.starts_with("Date") {
        let first = trimmed.lines().next().unwrap_or("").trim();
        // Stooq answers a symbol it genuinely has no history for (e.g. ^RUT,
        // ^VIX) with the plain body "No data". That is a successful, definitive
        // response — the symbol simply has nothing — not an endpoint failure,
        // so it must not feed the circuit breaker. Surface it as empty.
        if first.eq_ignore_ascii_case("No data") {
            return Ok(Vec::new());
        }
        return Err(anyhow!(
            "stooq returned no CSV: {}",
            first.chars().take(120).collect::<String>()
        ));
    }
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(trimmed.as_bytes());
    let mut out = Vec::new();
    for rec in rdr.records() {
        let rec = rec?;
        // Date,Open,High,Low,Close,Volume
        let cell = |i: usize| rec.get(i).unwrap_or("").trim();
        let d = cell(0);
        if d.is_empty() {
            continue;
        }
        let num = |s: &str| s.parse::<f64>().ok();
        let (Some(open), Some(high), Some(low), Some(close)) =
            (num(cell(1)), num(cell(2)), num(cell(3)), num(cell(4)))
        else {
            // Rows with "N/D" (no data, common for indexes) are skipped.
            continue;
        };
        let volume = cell(5).parse::<f64>().ok().map(|v| v as i64).unwrap_or(0);
        out.push(DailyBar {
            d: d.to_string(),
            open,
            high,
            low,
            close,
            volume,
        });
    }
    Ok(out)
}

#[async_trait]
impl HistoryProvider for StooqProvider {
    fn name(&self) -> &'static str {
        "stooq"
    }

    async fn daily(&self, ticker: &str, since: Option<&str>) -> Result<Vec<DailyBar>> {
        let sym = stooq_symbol(ticker);
        let mut url = format!("https://stooq.com/q/d/l/?s={sym}&i=d");
        if let Some(since) = since {
            // Stooq accepts an inclusive d1..d2 window in YYYYMMDD form.
            let d1 = since.replace('-', "");
            let d2 = chrono::Utc::now().format("%Y%m%d").to_string();
            url.push_str(&format!("&d1={d1}&d2={d2}"));
        }
        url.push_str(&format!("&apikey={}", self.apikey));

        let resp = self.client.get(&url).send().await?;

        // Surface an explicit rate-limit signal as a typed `RateLimited` error
        // so the EndpointGuard trips the breaker at once. Stooq's other way of
        // saying "slow down" — a 200 with a plain-text quota message — is not a
        // CSV body, so `parse_csv` rejects it as an ordinary error, and a short
        // streak of those trips the breaker too.
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

        let resp = resp.error_for_status()?;
        let text = resp.text().await?;
        parse_csv(&text)
    }
}
