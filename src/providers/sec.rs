//! Company fundamentals and filing history from SEC EDGAR.
//!
//! Three endpoints, no API key (SEC asks only for an identifying User-Agent,
//! which `build_sec_client` sets):
//!
//!  - `https://www.sec.gov/files/company_tickers.json`: the whole-market
//!    ticker -> CIK map, fetched once to fill in `symbols.cik`.
//!  - `https://data.sec.gov/api/xbrl/companyfacts/CIK##########.json`: every
//!    XBRL fact a company has reported.
//!  - `https://data.sec.gov/submissions/CIK##########.json`: its filing
//!    history.
//!
//! The XBRL `companyfacts` payload is the awkward one. A company reports the
//! same metric under different us-gaap *concepts* across accounting eras (e.g.
//! revenue moved to `RevenueFromContractWithCustomerExcludingAssessedTax`), and
//! each value carries year-to-date *and* discrete-period durations. `facts`
//! normalises that: it merges the candidate concepts for each of our metrics
//! and keeps only the clean full-year and discrete-quarter figures (see
//! `classify`).

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{Datelike, NaiveDate};
use reqwest::{header::RETRY_AFTER, StatusCode};
use serde::Deserialize;

use crate::providers::{Fact, FilingRecord, FundamentalsProvider, RateLimited};

/// Fundamentals and filings from SEC EDGAR.
pub struct SecProvider {
    /// A client whose User-Agent carries our contact email (see
    /// `http::build_sec_client`).
    client: reqwest::Client,
}

impl SecProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    /// Send one GET. `Ok(None)` means HTTP 404: a valid "this company has no
    /// such resource" answer (some symbols simply have no XBRL facts), not a
    /// failure, so it must not feed the circuit breaker. A 429/503 surfaces as
    /// the typed `RateLimited` the guard trips on at once.
    async fn get(&self, url: &str) -> Result<Option<reqwest::Response>> {
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
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Ok(Some(resp.error_for_status()?))
    }
}

/// Normalise a ticker to bare uppercase alphanumerics so our universe's
/// `BRK.B` matches EDGAR's `BRK-B` and Stooq-style symbols line up too.
pub fn normalize_ticker(ticker: &str) -> String {
    ticker
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_uppercase()
}

// ── company_tickers.json ───────────────────────────────────────────────────

#[derive(Deserialize)]
struct TickerEntry {
    cik_str: i64,
    ticker: String,
}

// ── companyfacts ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CompanyFacts {
    #[serde(default)]
    facts: FactNamespaces,
}

#[derive(Default, Deserialize)]
struct FactNamespaces {
    /// The us-gaap taxonomy carries every metric we read. `dei` (entity
    /// identifiers) is ignored.
    #[serde(rename = "us-gaap", default)]
    us_gaap: HashMap<String, Concept>,
}

#[derive(Deserialize)]
struct Concept {
    /// Keyed by XBRL unit (`USD`, `USD/shares`, `shares`, ...).
    #[serde(default)]
    units: HashMap<String, Vec<UnitEntry>>,
}

/// One reported value of a concept. `start` is present only for *duration*
/// facts (income-statement items); *instantaneous* facts (balance-sheet items)
/// carry only `end`.
///
/// The `fy` (fiscal year) field is deliberately not read: companyfacts tags a
/// fact with the fiscal year of the *filing* it was drawn from, so a prior
/// year shown as a comparative in a later 10-K carries that later filing's
/// `fy`. The fiscal year is taken from the `end` date instead (see `classify`).
#[derive(Deserialize)]
struct UnitEntry {
    start: Option<String>,
    end: String,
    val: f64,
    /// Fiscal period: `FY`, `Q1`, `Q2`, `Q3` (`Q4` appears rarely).
    fp: Option<String>,
    form: Option<String>,
    filed: Option<String>,
}

/// Our canonical metric -> the us-gaap concepts that can carry it. All listed
/// concepts are merged, so a company that changed concepts across eras still
/// gets a continuous series; a later filing's restated value wins ties (see
/// the `filed` comparison in `facts`).
const METRIC_CONCEPTS: &[(&str, &[&str])] = &[
    (
        "revenue",
        &[
            "RevenueFromContractWithCustomerExcludingAssessedTax",
            "Revenues",
            "RevenueFromContractWithCustomerIncludingAssessedTax",
            "SalesRevenueNet",
        ],
    ),
    ("net_income", &["NetIncomeLoss", "ProfitLoss"]),
    (
        "eps_diluted",
        &["EarningsPerShareDiluted", "EarningsPerShareBasicAndDiluted"],
    ),
    (
        "shares_diluted",
        &["WeightedAverageNumberOfDilutedSharesOutstanding"],
    ),
    (
        "dividends_per_share",
        &[
            "CommonStockDividendsPerShareDeclared",
            "CommonStockDividendsPerShareCashPaid",
        ],
    ),
    ("assets", &["Assets"]),
    ("liabilities", &["Liabilities"]),
    (
        "equity",
        &[
            "StockholdersEquity",
            "StockholdersEquityIncludingPortionAttributableToNoncontrollingInterest",
        ],
    ),
    ("assets_current", &["AssetsCurrent"]),
    ("liabilities_current", &["LiabilitiesCurrent"]),
];

/// How many fiscal years of annual and quarterly history to keep. Older
/// figures are dropped at parse time so `fundamentals` stays small.
const ANNUAL_YEARS: i64 = 6;
const QUARTERLY_YEARS: i64 = 3;

/// `Q3` -> `3`. `None` for anything that is not a `Q1`..`Q4` label.
fn quarter_num(fp: &str) -> Option<i64> {
    match fp {
        "Q1" => Some(1),
        "Q2" => Some(2),
        "Q3" => Some(3),
        "Q4" => Some(4),
        _ => None,
    }
}

/// The fiscal year a period ending on `end` belongs to, for a company whose
/// fiscal year ends in month `fye_month`.
///
/// A period that ends *after* the fiscal-year-end month falls in the next
/// fiscal year: e.g. an October-to-December quarter of a company with a
/// September fiscal-year end is Q1 of the *following* fiscal year, even though
/// it ends in the same calendar year as that year-end.
fn fiscal_year_of(end: NaiveDate, fye_month: u32) -> i64 {
    let y = end.year() as i64;
    if end.month() > fye_month {
        y + 1
    } else {
        y
    }
}

/// The calendar month a company's fiscal year ends, taken as the most common
/// end month across its annual (full-year duration) facts. Defaults to 12 (a
/// calendar fiscal year) when none can be determined.
fn fiscal_year_end_month(body: &CompanyFacts) -> u32 {
    let mut counts = [0u32; 13]; // indices 1..=12
    for concept in body.facts.us_gaap.values() {
        for entries in concept.units.values() {
            for e in entries {
                if e.fp.as_deref() != Some("FY") {
                    continue;
                }
                let (Some(start), Ok(end)) = (
                    e.start.as_deref(),
                    NaiveDate::parse_from_str(&e.end, "%Y-%m-%d"),
                ) else {
                    continue;
                };
                let Ok(start) = NaiveDate::parse_from_str(start, "%Y-%m-%d") else {
                    continue;
                };
                if (330..=400).contains(&(end - start).num_days()) {
                    counts[end.month() as usize] += 1;
                }
            }
        }
    }
    counts
        .iter()
        .enumerate()
        .skip(1)
        .max_by_key(|(_, &c)| c)
        .filter(|(_, &c)| c > 0)
        .map(|(m, _)| m as u32)
        .unwrap_or(12)
}

/// Classify one XBRL value into the fiscal period it cleanly represents, or
/// `None` to drop it. Returns `(period_label, fiscal_year, fiscal_qtr)`.
///
/// The fiscal year comes from the `end` date and the company's fiscal-year-end
/// month (see `fiscal_year_of`), never the `fy` field (see `UnitEntry`). XBRL
/// is otherwise noisy: a concept carries discrete-quarter values, full-year
/// values, *and* year-to-date roll-ups (6- and 9-month spans). This keeps only:
///  - **full years**: a duration fact spanning ~a year with `fp == FY`;
///  - **discrete quarters**: a duration fact spanning ~a quarter;
///  - **year-end balances**: an instantaneous fact with `fp == FY`.
///
/// Quarterly *balance-sheet* (instantaneous) figures are deliberately not
/// collected: a 10-Q tags its prior-year-end comparative with the filing's own
/// quarter, which would mislabel a year-end snapshot as a quarter.
fn classify(e: &UnitEntry, fye_month: u32) -> Option<(String, i64, Option<i64>)> {
    let end = NaiveDate::parse_from_str(&e.end, "%Y-%m-%d").ok()?;
    let fiscal_year = fiscal_year_of(end, fye_month);
    let fp = e.fp.as_deref().unwrap_or("");

    if let Some(start) = e.start.as_deref() {
        // Duration fact (an income-statement flow). The span length tells a
        // discrete quarter from a full year and from a year-to-date roll-up.
        let start = NaiveDate::parse_from_str(start, "%Y-%m-%d").ok()?;
        let days = (end - start).num_days();
        if (330..=400).contains(&days) && fp == "FY" {
            Some((format!("FY{fiscal_year}"), fiscal_year, None))
        } else if (80..=100).contains(&days) {
            let q = quarter_num(fp)?;
            Some((format!("Q{q}-{fiscal_year}"), fiscal_year, Some(q)))
        } else {
            None
        }
    } else {
        // Instantaneous fact (a balance-sheet snapshot): keep only the fiscal
        // year-end, sourced from an annual report.
        let form = e.form.as_deref().unwrap_or("");
        let annual_form = form.is_empty()
            || form.starts_with("10-K")
            || form.starts_with("20-F")
            || form.starts_with("40-F");
        if fp == "FY" && annual_form {
            Some((format!("FY{fiscal_year}"), fiscal_year, None))
        } else {
            None
        }
    }
}

// ── submissions ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Submissions {
    #[serde(default)]
    filings: SubmissionFilings,
}

#[derive(Default, Deserialize)]
struct SubmissionFilings {
    #[serde(default)]
    recent: RecentFilings,
}

/// EDGAR returns the filing history column-oriented: one parallel array per
/// field, indexed by filing.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecentFilings {
    #[serde(default)]
    accession_number: Vec<String>,
    #[serde(default)]
    filing_date: Vec<String>,
    #[serde(default)]
    report_date: Vec<String>,
    #[serde(default)]
    form: Vec<String>,
    #[serde(default)]
    primary_document: Vec<String>,
    #[serde(default)]
    primary_doc_description: Vec<String>,
}

/// The filing forms worth showing a market-watcher: periodic reports (10-K,
/// 10-Q, 8-K and the foreign-filer equivalents) and the annual proxy. The
/// long tail (insider Form 4s, 13G/D ownership stakes, S-8 registrations) is
/// dropped as noise.
fn is_material_form(form: &str) -> bool {
    const PREFIXES: [&str; 7] = ["10-K", "10-Q", "8-K", "20-F", "40-F", "6-K", "DEF 14A"];
    PREFIXES.iter().any(|p| form.starts_with(p))
}

/// How many material filings to keep per company.
const MAX_FILINGS: usize = 40;

#[async_trait]
impl FundamentalsProvider for SecProvider {
    fn name(&self) -> &'static str {
        "sec"
    }

    async fn cik_map(&self) -> Result<HashMap<String, String>> {
        let url = "https://www.sec.gov/files/company_tickers.json";
        let resp = self
            .get(url)
            .await?
            .ok_or_else(|| anyhow!("sec company_tickers.json not found"))?;
        // The file is a JSON object keyed by a row index; only the values matter.
        let entries: HashMap<String, TickerEntry> = resp.json().await?;
        let mut map = HashMap::with_capacity(entries.len());
        for entry in entries.into_values() {
            map.insert(
                normalize_ticker(&entry.ticker),
                format!("{:010}", entry.cik_str),
            );
        }
        Ok(map)
    }

    async fn facts(&self, cik: &str) -> Result<Vec<Fact>> {
        let url = format!("https://data.sec.gov/api/xbrl/companyfacts/CIK{cik}.json");
        let Some(resp) = self.get(&url).await? else {
            return Ok(Vec::new()); // 404: company has no XBRL facts
        };
        let body: CompanyFacts = resp.json().await?;

        let fye_month = fiscal_year_end_month(&body);
        let this_year = chrono::Utc::now().year() as i64;

        // Collapse to one fact per (metric, period): a metric can be reported
        // under several concepts and restated across filings, so the latest
        // `filed` wins.
        let mut chosen: HashMap<(String, String), Fact> = HashMap::new();

        for (metric, concepts) in METRIC_CONCEPTS {
            for concept in *concepts {
                let Some(concept_data) = body.facts.us_gaap.get(*concept) else {
                    continue;
                };
                for (unit, entries) in &concept_data.units {
                    for e in entries {
                        let Some((period, fiscal_year, fiscal_qtr)) = classify(e, fye_month)
                        else {
                            continue;
                        };
                        // Drop anything older than the retention window.
                        let keep_since = if fiscal_qtr.is_some() {
                            this_year - QUARTERLY_YEARS
                        } else {
                            this_year - ANNUAL_YEARS
                        };
                        if fiscal_year < keep_since {
                            continue;
                        }
                        let key = (metric.to_string(), period.clone());
                        let newer = chosen.get(&key).map_or(true, |prev| {
                            e.filed.as_deref().unwrap_or("") > prev.filed_at.as_deref().unwrap_or("")
                        });
                        if newer {
                            chosen.insert(
                                key,
                                Fact {
                                    metric: metric.to_string(),
                                    period,
                                    fiscal_year,
                                    fiscal_qtr,
                                    period_end: e.end.clone(),
                                    value: e.val,
                                    unit: Some(unit.clone()),
                                    form: e.form.clone(),
                                    filed_at: e.filed.clone(),
                                },
                            );
                        }
                    }
                }
            }
        }

        Ok(chosen.into_values().collect())
    }

    async fn filings(&self, cik: &str) -> Result<Vec<FilingRecord>> {
        let url = format!("https://data.sec.gov/submissions/CIK{cik}.json");
        let Some(resp) = self.get(&url).await? else {
            return Ok(Vec::new()); // 404: no submission history
        };
        let body: Submissions = resp.json().await?;
        let r = body.filings.recent;

        // EDGAR pads the CIK to 10 digits; the Archives path uses it unpadded.
        let cik_int = cik.trim_start_matches('0');

        let mut out = Vec::new();
        for i in 0..r.accession_number.len() {
            let form = r.form.get(i).cloned().unwrap_or_default();
            if !is_material_form(&form) {
                continue;
            }
            let accession = r.accession_number[i].clone();
            let filed_at = r.filing_date.get(i).cloned().unwrap_or_default();
            if accession.is_empty() || filed_at.is_empty() {
                continue;
            }
            let nodash = accession.replace('-', "");
            let primary_doc = r
                .primary_document
                .get(i)
                .filter(|s| !s.is_empty())
                .cloned();
            // Link straight to the primary document when EDGAR names one;
            // otherwise to the filing index page.
            let url = match &primary_doc {
                Some(doc) => {
                    format!("https://www.sec.gov/Archives/edgar/data/{cik_int}/{nodash}/{doc}")
                }
                None => format!(
                    "https://www.sec.gov/Archives/edgar/data/{cik_int}/{nodash}/{accession}-index.htm"
                ),
            };
            out.push(FilingRecord {
                accession,
                form,
                filed_at,
                period_of_report: r.report_date.get(i).filter(|s| !s.is_empty()).cloned(),
                primary_doc,
                url,
                description: r
                    .primary_doc_description
                    .get(i)
                    .filter(|s| !s.is_empty())
                    .cloned(),
            });
            if out.len() >= MAX_FILINGS {
                break;
            }
        }
        Ok(out)
    }
}
