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

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use crate::providers::{
    Fact, FilingRecord, FundFilings, FundHolding, FundId, FundShape, FundamentalsProvider,
    PortfolioData, RateLimited,
};

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

// ── ETF fund profiles: N-PORT holdings, AUM, filing history (Phase 18) ─────
//
// An ETF files as a registered fund, so its portfolio is not in the XBRL
// `companyfacts` above — it is in quarterly N-PORT filings, one large XML per
// fund. These methods are inherent to `SecProvider` rather than behind a
// trait: N-PORT is wholly SEC-specific, with no second source to abstract
// over. Each method makes exactly one HTTP request so the scheduler can keep
// wrapping every call in the endpoint guard, as it does for `facts`/`filings`.

/// Fund trusts that file with the SEC but are absent from
/// `company_tickers_mf.json` (which is keyed on the series/class structure of
/// open-end funds): the unit investment trusts (SPY, DIA) and the physical-
/// commodity grantor trusts (GLD, SLV). Mapped straight to their registrant
/// CIK, with no series id since each trust is a single fund.
const FUND_FALLBACK: &[(&str, i64)] = &[
    ("SPY", 884394),
    ("DIA", 1041130),
    ("GLD", 1222333),
    ("SLV", 1330568),
];

/// How many of a fund's holdings to keep — the largest by weight. A bond
/// aggregate fund holds thousands of positions; the page shows only the top.
const TOP_HOLDINGS: usize = 25;

/// How many of a fund's filings to keep for the page's filing list.
const MAX_FUND_FILINGS: usize = 40;

/// `company_tickers_mf.json`: a `fields` header plus row tuples of
/// `(cik, seriesId, classId, symbol)`.
#[derive(Deserialize)]
struct MfFile {
    data: Vec<(i64, String, String, String)>,
}

/// One filing parsed from a browse-edgar Atom feed.
#[derive(Default)]
struct AtomEntry {
    form: String,
    accession: String,
    filed: String,
    /// EDGAR filing-index page URL.
    href: String,
}

/// One holding accumulated while streaming through an `<invstOrSec>` block.
#[derive(Default)]
struct HoldingAcc {
    name: String,
    title: String,
    pct: Option<f64>,
    value: Option<f64>,
    asset_cat: Option<String>,
}

impl HoldingAcc {
    fn into_holding(self) -> FundHolding {
        // Prefer `title` (the issue title): it is clean mixed-case, where the
        // issuer `name` often arrives truncated and all-caps. Fall back to
        // `name` for the rare holding that carried no title.
        let name = if self.title.is_empty() {
            self.name
        } else {
            self.title
        };
        FundHolding {
            name,
            pct: self.pct,
            value_usd: self.value,
            asset_cat: self.asset_cat,
        }
    }
}

impl SecProvider {
    /// Ticker -> fund identity, from the SEC mutual-fund ticker file plus the
    /// hardcoded fallback for fund trusts absent from it. Keys are normalised
    /// like `cik_map`'s. One bulk request, fetched while some ETF lacks a CIK.
    pub async fn fund_ticker_map(&self) -> Result<HashMap<String, FundId>> {
        let url = "https://www.sec.gov/files/company_tickers_mf.json";
        let resp = self
            .get(url)
            .await?
            .ok_or_else(|| anyhow!("sec company_tickers_mf.json not found"))?;
        let body: MfFile = resp.json().await?;
        let mut map = HashMap::with_capacity(body.data.len() + FUND_FALLBACK.len());
        for (cik, series_id, _class_id, symbol) in body.data {
            map.entry(normalize_ticker(&symbol)).or_insert(FundId {
                cik: format!("{cik:010}"),
                series_id: Some(series_id),
            });
        }
        for (ticker, cik) in FUND_FALLBACK {
            map.entry(normalize_ticker(ticker)).or_insert(FundId {
                cik: format!("{cik:010}"),
                series_id: None,
            });
        }
        Ok(map)
    }

    /// A fund's filing list, plus what the filing history says about its shape
    /// (whether to read an N-PORT for holdings, or treat it as a commodity
    /// trust). One browse-edgar request, keyed on the series id when the
    /// registrant hosts several funds so a sibling fund's filings never leak in.
    pub async fn fund_filings(&self, id: &FundId) -> Result<FundFilings> {
        let key = id.series_id.as_deref().unwrap_or(&id.cik);
        let url = format!(
            "https://www.sec.gov/cgi-bin/browse-edgar?action=getcompany&CIK={key}\
             &type=&dateb=&owner=include&count=100&output=atom"
        );
        let resp = self
            .get(&url)
            .await?
            .ok_or_else(|| anyhow!("edgar filing index for {key} not found"))?;
        let bytes = resp.bytes().await?;
        let entries = parse_edgar_atom(&bytes)?;

        // Entries arrive newest-first. Read the fund's shape from the whole set
        // before trimming the list to the material forms shown on the page.
        let mut nport: Option<String> = None;
        let mut has_ncen = false;
        let mut has_10k = false;
        for e in &entries {
            if nport.is_none() && e.form.starts_with("NPORT-P") && !e.href.is_empty() {
                nport = Some(e.href.clone());
            }
            has_ncen |= e.form.starts_with("N-CEN");
            has_10k |= e.form.starts_with("10-K");
        }
        let shape = if let Some(nport_href) = nport {
            FundShape::Portfolio { nport_href }
        } else if has_10k && !has_ncen {
            // Files 10-Ks and no fund-census report: a grantor trust holding a
            // physical commodity rather than a securities portfolio.
            FundShape::CommodityTrust
        } else {
            FundShape::Unknown
        };

        let filings = entries
            .into_iter()
            .filter(|e| is_material_fund_form(&e.form))
            .take(MAX_FUND_FILINGS)
            .map(|e| FilingRecord {
                accession: e.accession,
                form: e.form,
                filed_at: e.filed,
                period_of_report: None,
                primary_doc: None,
                url: e.href,
                description: None,
            })
            .collect();

        Ok(FundFilings { filings, shape })
    }

    /// Parse one N-PORT filing into a portfolio snapshot: net assets, the
    /// holdings (top slice by weight), the holding count, and the asset mix.
    /// `index_href` is the filing's EDGAR index-page URL; the N-PORT XML sits
    /// beside it in the same Archives directory. One request.
    pub async fn fund_portfolio(&self, index_href: &str) -> Result<PortfolioData> {
        // `.../data/{cik}/{nodash}/{accession}-index.htm` -> swap the index
        // page for `primary_doc.xml` in the same directory.
        let dir = index_href
            .rsplit_once('/')
            .map(|(d, _)| d)
            .ok_or_else(|| anyhow!("malformed filing href {index_href}"))?;
        let url = format!("{dir}/primary_doc.xml");
        let resp = self
            .get(&url)
            .await?
            .ok_or_else(|| anyhow!("N-PORT not found at {url}"))?;
        let bytes = resp.bytes().await?;
        parse_nport(&bytes)
    }

    /// The latest total-assets figure a company has reported, USD. Gives a
    /// physical-commodity grantor trust (GLD, SLV) an AUM: those file 10-Ks,
    /// not N-PORT, so `Assets` from their XBRL companyfacts stands in for net
    /// assets. Unlike `facts`, this takes the single most recent value
    /// regardless of fiscal period, so a mid-year 10-Q figure beats a stale
    /// prior year-end. `None` when the company has no `Assets` concept.
    pub async fn fund_aum(&self, cik: &str) -> Result<Option<f64>> {
        let url = format!("https://data.sec.gov/api/xbrl/companyfacts/CIK{cik}.json");
        let Some(resp) = self.get(&url).await? else {
            return Ok(None); // 404: no XBRL facts
        };
        let body: CompanyFacts = resp.json().await?;
        let Some(assets) = body.facts.us_gaap.get("Assets") else {
            return Ok(None);
        };
        // Newest by period-end date, ties broken by the later filing.
        let mut best: Option<&UnitEntry> = None;
        for entries in assets.units.values() {
            for e in entries {
                let newer = best.map_or(true, |b| {
                    (e.end.as_str(), e.filed.as_deref().unwrap_or(""))
                        > (b.end.as_str(), b.filed.as_deref().unwrap_or(""))
                });
                if newer {
                    best = Some(e);
                }
            }
        }
        Ok(best.map(|e| e.val))
    }
}

/// Filing forms worth showing on a fund's page: its portfolio reports
/// (N-PORT), the annual fund census and shareholder reports, the prospectus,
/// and — for a commodity trust — the 10-K family it files instead.
fn is_material_fund_form(form: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "NPORT-P", "NPORT-EX", "N-CEN", "N-CSR", "485BPOS", "485APOS", "10-K", "10-Q", "8-K",
    ];
    PREFIXES.iter().any(|p| form.starts_with(p))
}

/// Map an N-PORT `assetCat` code to a human asset-class bucket for the mix.
/// The codes are from the N-PORT technical schema; the long tail is "Other".
fn asset_bucket(cat: &str) -> &'static str {
    match cat {
        "EC" | "EP" => "Equity",
        "DBT" | "SF" => "Bonds",
        "STIV" | "RA" => "Cash & equivalents",
        "COMM" => "Commodities",
        "RE" => "Real estate",
        "LON" => "Loans",
        c if c.starts_with("ABS") => "Bonds",
        // Every other derivative category code begins `D` (DBT is matched above).
        c if c.starts_with('D') => "Derivatives",
        _ => "Other",
    }
}

/// Read one attribute off a start tag as a `String`.
fn attr_val(e: &BytesStart, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .and_then(|a| String::from_utf8(a.value.into_owned()).ok())
}

/// Set the current entry's form from a `<category term="..."/>` tag.
fn set_form_from_category(e: &BytesStart, cur: &mut Option<AtomEntry>) {
    if let (Some(c), Some(term)) = (cur.as_mut(), attr_val(e, b"term")) {
        c.form = term;
    }
}

/// Parse a browse-edgar Atom feed into its filing entries, newest first.
fn parse_edgar_atom(xml: &[u8]) -> Result<Vec<AtomEntry>> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut path: Vec<Vec<u8>> = Vec::new();
    let mut entries = Vec::new();
    let mut cur: Option<AtomEntry> = None;

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => {
                let name = e.local_name().as_ref().to_vec();
                if name == b"entry" {
                    cur = Some(AtomEntry::default());
                }
                if name == b"category" {
                    set_form_from_category(&e, &mut cur);
                }
                path.push(name);
            }
            // The form type usually rides a self-closing `<category term=".."/>`.
            Event::Empty(e) => {
                if e.local_name().as_ref() == b"category" {
                    set_form_from_category(&e, &mut cur);
                }
            }
            Event::End(e) => {
                if e.local_name().as_ref() == b"entry" {
                    if let Some(c) = cur.take() {
                        if !c.accession.is_empty() {
                            entries.push(c);
                        }
                    }
                }
                path.pop();
            }
            Event::Text(t) => {
                let (Some(tag), Some(c)) = (path.last(), cur.as_mut()) else {
                    continue;
                };
                let raw = t.unescape().unwrap_or_default();
                let txt = raw.trim();
                if txt.is_empty() {
                    continue;
                }
                match tag.as_slice() {
                    b"accession-number" if c.accession.is_empty() => c.accession = txt.to_string(),
                    b"filing-date" if c.filed.is_empty() => c.filed = txt.to_string(),
                    b"filing-href" if c.href.is_empty() => c.href = txt.to_string(),
                    b"filing-type" if c.form.is_empty() => c.form = txt.to_string(),
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(entries)
}

/// Stream-parse an N-PORT `primary_doc.xml` into a portfolio snapshot. The file
/// can run to many megabytes for a bond fund's thousands of positions, so this
/// walks events rather than building a DOM, keeping only the running totals
/// and, at the end, the largest holdings.
fn parse_nport(xml: &[u8]) -> Result<PortfolioData> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    // Stack of open element local names, so a leaf is read in context.
    let mut path: Vec<Vec<u8>> = Vec::new();
    let mut out = PortfolioData::default();
    let mut all: Vec<FundHolding> = Vec::new();
    // The holding currently being assembled, set while inside `<invstOrSec>`.
    let mut cur: Option<HoldingAcc> = None;

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => {
                let name = e.local_name().as_ref().to_vec();
                if name == b"invstOrSec" {
                    cur = Some(HoldingAcc::default());
                }
                path.push(name);
            }
            Event::End(e) => {
                if e.local_name().as_ref() == b"invstOrSec" {
                    if let Some(h) = cur.take() {
                        all.push(h.into_holding());
                    }
                }
                path.pop();
            }
            Event::Text(t) => {
                let Some(tag) = path.last() else {
                    continue;
                };
                let raw = t.unescape().unwrap_or_default();
                let txt = raw.trim();
                if txt.is_empty() {
                    continue;
                }
                let tag = tag.as_slice();
                // Holding fields, captured only inside an `<invstOrSec>`.
                // First-wins: the issuer-level value precedes any nested block.
                if let Some(h) = cur.as_mut() {
                    match tag {
                        b"name" if h.name.is_empty() => h.name = txt.to_string(),
                        b"title" if h.title.is_empty() => h.title = txt.to_string(),
                        b"pctVal" if h.pct.is_none() => h.pct = txt.parse().ok(),
                        b"valUSD" if h.value.is_none() => h.value = txt.parse().ok(),
                        b"assetCat" if h.asset_cat.is_none() => {
                            h.asset_cat = Some(txt.to_string())
                        }
                        _ => {}
                    }
                }
                // Fund-level fields, scoped by their parent element.
                let parent = path.iter().rev().nth(1).map(Vec::as_slice);
                match (parent, tag) {
                    (Some(b"fundInfo"), b"netAssets") if out.net_assets.is_none() => {
                        out.net_assets = txt.parse().ok();
                    }
                    (Some(b"fundInfo"), b"totAssets") if out.total_assets.is_none() => {
                        out.total_assets = txt.parse().ok();
                    }
                    (Some(b"genInfo"), b"repPdDate") if out.report_date.is_none() => {
                        out.report_date = Some(txt.to_string());
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    out.holdings_count = all.len() as i64;

    // Asset-class mix: each holding's weight summed into its bucket. Tiny
    // residual buckets (rounding noise) are dropped.
    let mut mix: HashMap<&'static str, f64> = HashMap::new();
    for h in &all {
        let bucket = h.asset_cat.as_deref().map_or("Other", asset_bucket);
        *mix.entry(bucket).or_insert(0.0) += h.pct.unwrap_or(0.0);
    }
    let mut mix: Vec<(String, f64)> = mix
        .into_iter()
        .filter(|(_, p)| *p >= 0.05)
        .map(|(b, p)| (b.to_string(), p))
        .collect();
    mix.sort_by(|a, b| b.1.total_cmp(&a.1));
    out.asset_mix = mix;

    // Largest holdings first; keep only the top slice.
    all.sort_by(|a, b| b.pct.unwrap_or(0.0).total_cmp(&a.pct.unwrap_or(0.0)));
    all.truncate(TOP_HOLDINGS);
    out.top_holdings = all;

    Ok(out)
}
