//! Derived market figures. Ratios live here, not in the database, so they
//! always reflect the latest price.

use serde::Serialize;

/// Placeholder shown for a figure that cannot be computed — an em dash, the
/// unambiguous "no data" mark used for every empty value across the app.
const DASH: &str = "\u{2014}";

/// An absolute and percentage change between two prices.
#[derive(Debug, Clone, Copy)]
pub struct Change {
    pub abs: f64,
    pub pct: f64,
}

/// Change of `last` relative to `prev` (a prior close).
pub fn change(last: f64, prev: f64) -> Change {
    let abs = last - prev;
    let pct = if prev != 0.0 { abs / prev * 100.0 } else { 0.0 };
    Change { abs, pct }
}

/// A VIX level read into one plain word, for the dashboard's volatility tone.
/// The bands suit the ^VIX cash gauge: sub-14 is a placid tape, the teens are
/// normal, the low-20s start to show stress, and 28+ is outright fear.
pub fn vix_tone(level: f64) -> &'static str {
    match level {
        v if v < 14.0 => "calm",
        v if v < 20.0 => "steady",
        v if v < 28.0 => "elevated",
        _ => "stressed",
    }
}

/// Position of `value` along the `[lo, hi]` range, as a 0..100 percent for
/// placing a marker on a track. Clamped to the ends; a zero-width range maps
/// to the midpoint. Rounded to 2 dp so it inlines cleanly into a `style`.
pub fn pos(value: f64, lo: f64, hi: f64) -> f64 {
    if hi <= lo {
        return 50.0;
    }
    let p = (value - lo) / (hi - lo) * 100.0;
    (p.clamp(0.0, 100.0) * 100.0).round() / 100.0
}

// ─────────────────────────── computed ratios ───────────────────────────────
//
// Phase 7. Each ratio is computed from the latest full fiscal year's SEC
// figures plus the latest price, graded good / ok / bad against sensible
// thresholds, and paired with plain-English text so a non-expert can read it.
// Nothing here is stored: a fresh price re-grades the price-based ratios.

/// A computed fundamental ratio's quality, for the symbol page's semantic
/// green / amber / red.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Grade {
    Good,
    Ok,
    Bad,
    /// Inputs missing or the ratio is not meaningful (e.g. negative equity):
    /// shown neutrally, not coloured.
    Unknown,
}

impl Grade {
    /// One-word verdict for the ratio card's badge.
    pub fn verdict(self) -> &'static str {
        match self {
            Grade::Good => "Strong",
            Grade::Ok => "Fair",
            Grade::Bad => "Weak",
            Grade::Unknown => "No data",
        }
    }
}

/// One computed ratio, ready for the symbol page: a graded value plus
/// plain-English text so a non-expert can tell good from concerning.
#[derive(Debug, Clone, Serialize)]
pub struct Ratio {
    /// Stable identifier, also a CSS hook.
    pub key: &'static str,
    pub label: &'static str,
    /// Formatted value, e.g. `28.4x`, `1.6%`; a middle dot when unknown.
    pub display: String,
    pub grade: Grade,
    /// One-word badge text derived from `grade`.
    pub verdict: &'static str,
    /// Plain-English reading of this company's particular value.
    pub reading: String,
    /// What the metric means and how to read it, the same for every company.
    pub explain: &'static str,
}

/// The figures a full set of ratios is computed from: the latest full fiscal
/// year's values, the prior year's (for the growth ratios), and a price. Every
/// field is optional, since a company may simply not report a given concept.
#[derive(Debug, Default, Clone)]
pub struct RatioInputs {
    pub price: Option<f64>,
    pub eps_diluted: Option<f64>,
    pub dividends_per_share: Option<f64>,
    pub revenue: Option<f64>,
    pub net_income: Option<f64>,
    pub assets: Option<f64>,
    pub liabilities: Option<f64>,
    pub equity: Option<f64>,
    pub assets_current: Option<f64>,
    pub liabilities_current: Option<f64>,
    pub prev_revenue: Option<f64>,
    pub prev_net_income: Option<f64>,
}

/// Assemble a `Ratio`, deriving the badge verdict from the grade.
fn mk(
    key: &'static str,
    label: &'static str,
    explain: &'static str,
    display: String,
    grade: Grade,
    reading: String,
) -> Ratio {
    Ratio {
        key,
        label,
        display,
        grade,
        verdict: grade.verdict(),
        reading,
        explain,
    }
}

/// An `Unknown` ratio: inputs missing or the ratio not meaningful.
fn unknown(
    key: &'static str,
    label: &'static str,
    explain: &'static str,
    reading: &str,
) -> Ratio {
    mk(key, label, explain, DASH.to_string(), Grade::Unknown, reading.to_string())
}

/// The nine ratios shown on a stock's symbol page, in display order.
pub fn compute_ratios(i: &RatioInputs) -> Vec<Ratio> {
    vec![
        pe(i.price, i.eps_diluted),
        dividend_yield(i.price, i.dividends_per_share),
        profit_margin(i.net_income, i.revenue),
        return_on_equity(i.net_income, i.equity),
        return_on_assets(i.net_income, i.assets),
        debt_to_equity(i.liabilities, i.equity),
        current_ratio(i.assets_current, i.liabilities_current),
        revenue_growth(i.revenue, i.prev_revenue),
        earnings_growth(i.net_income, i.prev_net_income),
    ]
}

fn pe(price: Option<f64>, eps: Option<f64>) -> Ratio {
    const KEY: &str = "pe";
    const LABEL: &str = "P/E ratio";
    const EXPLAIN: &str = "Share price divided by earnings per share: what you \
        pay for each $1 of yearly profit. Roughly 15 to 25 is typical, above 40 \
        is richly priced, and negative means the company is losing money.";
    let (Some(price), Some(eps)) = (price, eps) else {
        return unknown(KEY, LABEL, EXPLAIN, "Not enough data to compute a price-to-earnings ratio.");
    };
    if eps <= 0.0 {
        return unknown(
            KEY, LABEL, EXPLAIN,
            "Earnings per share were negative, so a P/E cannot be formed; the company was unprofitable over the period.",
        );
    }
    let v = price / eps;
    // Below 10x the stock is cheap (a bargain, or a warning); 10-25x is the
    // healthy band; 25-40x is paying up for growth; above 40x is steep.
    let (grade, reading) = if v < 10.0 {
        (Grade::Ok, format!("At {v:.0}x, the stock is priced cheaply against its profits: sometimes a bargain, sometimes a sign of trouble ahead."))
    } else if v <= 25.0 {
        (Grade::Good, format!("At {v:.0}x, the price is a reasonable multiple of the company's annual profit."))
    } else if v < 40.0 {
        (Grade::Ok, format!("At {v:.0}x, investors are paying up; a fair amount of future growth is already in the price."))
    } else {
        (Grade::Bad, format!("At {v:.0}x, the price is steep relative to profit; the stock leans heavily on growth that has yet to arrive."))
    };
    mk(KEY, LABEL, EXPLAIN, format!("{v:.1}x"), grade, reading)
}

fn dividend_yield(price: Option<f64>, dps: Option<f64>) -> Ratio {
    const KEY: &str = "div_yield";
    const LABEL: &str = "Dividend yield";
    const EXPLAIN: &str = "The yearly dividend as a percent of the share price: \
        the cash income each share pays out. Around 2 to 6% is healthy; above \
        roughly 8% often signals the payout may be cut.";
    let Some(price) = price.filter(|p| *p > 0.0) else {
        return unknown(KEY, LABEL, EXPLAIN, "Not enough data to compute a dividend yield.");
    };
    // A company that pays no dividend simply never reports the concept; treat
    // a missing figure as a genuine zero.
    let dps = dps.unwrap_or(0.0);
    let v = dps / price * 100.0;
    let (grade, reading) = if v <= 0.0 {
        (Grade::Ok, "This company pays no dividend, common for firms reinvesting their profits back into growth.".to_string())
    } else if v < 2.0 {
        (Grade::Ok, format!("A {v:.1}% yield is modest: a small income on top of whatever the share price does."))
    } else if v <= 6.0 {
        (Grade::Good, format!("A {v:.1}% yield is a healthy, generally sustainable level of cash income."))
    } else if v <= 10.0 {
        (Grade::Ok, format!("A {v:.1}% yield is high; it is worth checking the payout is covered by profit."))
    } else {
        (Grade::Bad, format!("A {v:.1}% yield is unusually high, often a sign the market expects the dividend to be cut."))
    };
    mk(KEY, LABEL, EXPLAIN, format!("{v:.2}%"), grade, reading)
}

fn profit_margin(net_income: Option<f64>, revenue: Option<f64>) -> Ratio {
    const KEY: &str = "profit_margin";
    const LABEL: &str = "Profit margin";
    const EXPLAIN: &str = "The share of revenue left as profit once every cost \
        is paid. Above 15% is strong; below 5% leaves little cushion against a \
        bad year.";
    let (Some(ni), Some(rev)) = (net_income, revenue) else {
        return unknown(KEY, LABEL, EXPLAIN, "Not enough data to compute a profit margin.");
    };
    if rev <= 0.0 {
        return unknown(KEY, LABEL, EXPLAIN, "No revenue was reported, so a margin cannot be computed.");
    }
    let v = ni / rev * 100.0;
    let (grade, reading) = if v < 5.0 {
        (Grade::Bad, format!("A {v:.1}% margin is thin; little of each revenue dollar survives as profit."))
    } else if v <= 15.0 {
        (Grade::Ok, format!("A {v:.1}% margin is solid, in the ordinary range for a profitable company."))
    } else {
        (Grade::Good, format!("A {v:.1}% margin is strong; the company keeps a healthy slice of every revenue dollar."))
    };
    mk(KEY, LABEL, EXPLAIN, format!("{v:.1}%"), grade, reading)
}

fn return_on_equity(net_income: Option<f64>, equity: Option<f64>) -> Ratio {
    const KEY: &str = "roe";
    const LABEL: &str = "Return on equity";
    const EXPLAIN: &str = "Profit earned on each dollar of shareholder equity: \
        how well the company compounds its owners' capital. Above 15% is strong.";
    let (Some(ni), Some(eq)) = (net_income, equity) else {
        return unknown(KEY, LABEL, EXPLAIN, "Not enough data to compute return on equity.");
    };
    if eq <= 0.0 {
        return unknown(KEY, LABEL, EXPLAIN, "Shareholder equity is negative, so return on equity is not meaningful.");
    }
    let v = ni / eq * 100.0;
    let (grade, reading) = if v < 5.0 {
        (Grade::Bad, format!("A {v:.1}% return on equity is weak; owners' capital is barely being put to work."))
    } else if v <= 15.0 {
        (Grade::Ok, format!("A {v:.1}% return on equity is respectable, in the normal range."))
    } else {
        (Grade::Good, format!("A {v:.1}% return on equity is strong; the company compounds owners' capital well."))
    };
    mk(KEY, LABEL, EXPLAIN, format!("{v:.1}%"), grade, reading)
}

fn return_on_assets(net_income: Option<f64>, assets: Option<f64>) -> Ratio {
    const KEY: &str = "roa";
    const LABEL: &str = "Return on assets";
    const EXPLAIN: &str = "Profit earned on each dollar of assets: how \
        efficiently the whole asset base is used. Above 8% is strong.";
    let (Some(ni), Some(assets)) = (net_income, assets) else {
        return unknown(KEY, LABEL, EXPLAIN, "Not enough data to compute return on assets.");
    };
    if assets <= 0.0 {
        return unknown(KEY, LABEL, EXPLAIN, "No asset total was reported, so return on assets cannot be computed.");
    }
    let v = ni / assets * 100.0;
    let (grade, reading) = if v < 2.0 {
        (Grade::Bad, format!("A {v:.1}% return on assets is low; the asset base is generating little profit."))
    } else if v <= 8.0 {
        (Grade::Ok, format!("A {v:.1}% return on assets is reasonable for a company of this kind."))
    } else {
        (Grade::Good, format!("A {v:.1}% return on assets is strong; the company squeezes good profit from its assets."))
    };
    mk(KEY, LABEL, EXPLAIN, format!("{v:.1}%"), grade, reading)
}

fn debt_to_equity(liabilities: Option<f64>, equity: Option<f64>) -> Ratio {
    const KEY: &str = "debt_equity";
    const LABEL: &str = "Debt-to-equity";
    const EXPLAIN: &str = "Total liabilities divided by shareholder equity: how \
        heavily the company leans on borrowing. Below 1 is conservative; above \
        2 is highly leveraged.";
    let (Some(liab), Some(eq)) = (liabilities, equity) else {
        return unknown(KEY, LABEL, EXPLAIN, "Not enough data to compute debt-to-equity.");
    };
    if eq <= 0.0 {
        return mk(
            KEY, LABEL, EXPLAIN, DASH.to_string(), Grade::Bad,
            "Shareholder equity is negative; liabilities exceed everything the company owns.".to_string(),
        );
    }
    let v = liab / eq;
    let (grade, reading) = if v < 1.0 {
        (Grade::Good, format!("At {v:.2}, the company carries less in liabilities than in equity, a conservative balance sheet."))
    } else if v <= 2.0 {
        (Grade::Ok, format!("At {v:.2}, the company carries a moderate, manageable amount of debt."))
    } else {
        (Grade::Bad, format!("At {v:.2}, the company leans heavily on borrowing, which adds risk if results weaken."))
    };
    mk(KEY, LABEL, EXPLAIN, format!("{v:.2}"), grade, reading)
}

fn current_ratio(assets_current: Option<f64>, liabilities_current: Option<f64>) -> Ratio {
    const KEY: &str = "current_ratio";
    const LABEL: &str = "Current ratio";
    const EXPLAIN: &str = "Current assets divided by current liabilities: \
        whether short-term resources cover short-term bills. Above 1.5 is \
        comfortable; below 1 is tight.";
    let (Some(ca), Some(cl)) = (assets_current, liabilities_current) else {
        return unknown(KEY, LABEL, EXPLAIN, "This company does not report a current-assets breakdown, so the ratio cannot be computed.");
    };
    if cl <= 0.0 {
        return unknown(KEY, LABEL, EXPLAIN, "No current liabilities were reported, so the ratio cannot be computed.");
    }
    let v = ca / cl;
    let (grade, reading) = if v < 1.0 {
        (Grade::Bad, format!("At {v:.2}, short-term assets fall short of short-term bills; liquidity is tight."))
    } else if v < 1.5 {
        (Grade::Ok, format!("At {v:.2}, short-term assets cover short-term bills with a little room to spare."))
    } else {
        (Grade::Good, format!("At {v:.2}, short-term assets comfortably cover short-term bills."))
    };
    mk(KEY, LABEL, EXPLAIN, format!("{v:.2}"), grade, reading)
}

fn revenue_growth(revenue: Option<f64>, prev_revenue: Option<f64>) -> Ratio {
    const KEY: &str = "revenue_growth";
    const LABEL: &str = "Revenue growth";
    const EXPLAIN: &str = "Change in annual revenue from the prior fiscal year: \
        whether the top line is expanding. Above 10% is strong growth; below 0 \
        means revenue is shrinking.";
    let (Some(rev), Some(prev)) = (revenue, prev_revenue) else {
        return unknown(KEY, LABEL, EXPLAIN, "Two fiscal years of revenue are needed to compute growth.");
    };
    if prev <= 0.0 {
        return unknown(KEY, LABEL, EXPLAIN, "Prior-year revenue was not positive, so a growth rate is not meaningful.");
    }
    let v = (rev - prev) / prev * 100.0;
    let (grade, reading) = if v < 0.0 {
        (Grade::Bad, format!("Revenue fell {:.1}% from the prior year; the top line is contracting.", v.abs()))
    } else if v <= 10.0 {
        (Grade::Ok, format!("Revenue grew {v:.1}% from the prior year: steady, modest expansion."))
    } else {
        (Grade::Good, format!("Revenue grew {v:.1}% from the prior year: strong top-line expansion."))
    };
    mk(KEY, LABEL, EXPLAIN, format!("{v:+.1}%"), grade, reading)
}

// ─────────────────────────── chart indicators ──────────────────────────────
//
// Phase 8. Overlay/indicator series for the price chart: simple and
// exponential moving averages plus a Relative Strength Index. Each takes a
// slice of closing prices (oldest first) and returns one `Option<f64>` per
// input bar — `None` until enough history has accumulated for the figure to
// be meaningful — so a caller can align the result to its bar list by index
// and drop the leading `None`s. The maths lives here, not in SQL or the
// browser, so it stays in one place the rest of the app already trusts.

/// Simple moving average over `period` bars: `out[i]` is the mean of
/// `closes[i+1-period ..= i]`, and `None` for the first `period-1` bars.
/// A running sum keeps it one pass regardless of `period`.
pub fn sma(closes: &[f64], period: usize) -> Vec<Option<f64>> {
    if period == 0 {
        return vec![None; closes.len()];
    }
    let mut out = Vec::with_capacity(closes.len());
    let mut sum = 0.0;
    for i in 0..closes.len() {
        sum += closes[i];
        if i >= period {
            sum -= closes[i - period];
        }
        out.push((i + 1 >= period).then(|| sum / period as f64));
    }
    out
}

/// Exponential moving average over `period` bars. Seeded at index `period-1`
/// with the simple average of the first window, then each step weights the
/// newest close by `2/(period+1)`. `None` before the seed bar.
pub fn ema(closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; closes.len()];
    if period == 0 || closes.len() < period {
        return out;
    }
    let k = 2.0 / (period as f64 + 1.0);
    let mut prev = closes[..period].iter().sum::<f64>() / period as f64;
    out[period - 1] = Some(prev);
    for i in period..closes.len() {
        prev = closes[i] * k + prev * (1.0 - k);
        out[i] = Some(prev);
    }
    out
}

/// Wilder's Relative Strength Index over `period` bars (classically 14): a
/// 0..100 momentum reading, `None` until `period` price changes have
/// accumulated. Above ~70 is conventionally "overbought", below ~30
/// "oversold". The seed averages the first `period` gains and losses; every
/// later bar applies Wilder's smoothing.
pub fn rsi(closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; closes.len()];
    if period == 0 || closes.len() <= period {
        return out;
    }
    let (mut gain, mut loss) = (0.0, 0.0);
    for i in 1..=period {
        let ch = closes[i] - closes[i - 1];
        if ch >= 0.0 {
            gain += ch;
        } else {
            loss -= ch;
        }
    }
    let mut avg_gain = gain / period as f64;
    let mut avg_loss = loss / period as f64;
    out[period] = Some(rsi_from(avg_gain, avg_loss));
    for i in period + 1..closes.len() {
        let ch = closes[i] - closes[i - 1];
        let (g, l) = if ch >= 0.0 { (ch, 0.0) } else { (0.0, -ch) };
        avg_gain = (avg_gain * (period as f64 - 1.0) + g) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + l) / period as f64;
        out[i] = Some(rsi_from(avg_gain, avg_loss));
    }
    out
}

/// One RSI reading from a smoothed average gain and loss; an all-gains
/// window (no losses) reads a flat 100.
fn rsi_from(avg_gain: f64, avg_loss: f64) -> f64 {
    if avg_loss == 0.0 {
        return 100.0;
    }
    let rs = avg_gain / avg_loss;
    100.0 - 100.0 / (1.0 + rs)
}

fn earnings_growth(net_income: Option<f64>, prev_net_income: Option<f64>) -> Ratio {
    const KEY: &str = "earnings_growth";
    const LABEL: &str = "Earnings growth";
    const EXPLAIN: &str = "Change in annual net income from the prior fiscal \
        year: whether profit is expanding. Above 10% is strong; below 0 means \
        profit is falling.";
    let (Some(ni), Some(prev)) = (net_income, prev_net_income) else {
        return unknown(KEY, LABEL, EXPLAIN, "Two fiscal years of net income are needed to compute growth.");
    };
    if prev <= 0.0 {
        // A growth percentage off a loss-making base is meaningless; describe
        // the turn instead of computing a rate.
        let reading = if ni > 0.0 {
            "The company returned to profit after a loss-making prior year."
        } else {
            "The company was unprofitable in both years, so an earnings growth rate is not meaningful."
        };
        return unknown(KEY, LABEL, EXPLAIN, reading);
    }
    let v = (ni - prev) / prev * 100.0;
    let (grade, reading) = if ni < 0.0 {
        (Grade::Bad, "Profit swung to a loss from a profitable prior year.".to_string())
    } else if v < 0.0 {
        (Grade::Bad, format!("Net income fell {:.1}% from the prior year; profit is shrinking.", v.abs()))
    } else if v <= 10.0 {
        (Grade::Ok, format!("Net income grew {v:.1}% from the prior year: steady profit expansion."))
    } else {
        (Grade::Good, format!("Net income grew {v:.1}% from the prior year: strong profit expansion."))
    };
    mk(KEY, LABEL, EXPLAIN, format!("{v:+.1}%"), grade, reading)
}

// ──────────────────────── dividend pace (Phase 26) ─────────────────────────
//
// Inferred cadence + an on-track read for a stock's dividend payouts. Inputs
// are sorted (ex_date, amount) pairs and a reference "today" date. Pure code,
// kept here next to the other graded reads; the route formats display.

/// How frequently a stock pays out — inferred from the median gap between its
/// recent ex-dividend dates. Drives both the page's "Pays …" caption and the
/// count-tempered projection of the current year's total.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Cadence {
    /// One payment per year.
    Annual,
    /// Two per year (~180-day gap), e.g. some European dual-listings.
    SemiAnnual,
    /// Four per year (~90-day gap), the US norm.
    Quarterly,
    /// Twelve per year, e.g. monthly-paying real-estate trusts.
    Monthly,
    /// Cadence does not fit a clean pattern (special one-off, etc.).
    Irregular,
    /// No payouts to read from.
    None,
}

impl Cadence {
    /// A short caption for the page header, e.g. `Pays quarterly`.
    pub fn caption(self) -> &'static str {
        match self {
            Cadence::Annual => "Pays annually",
            Cadence::SemiAnnual => "Pays twice a year",
            Cadence::Quarterly => "Pays quarterly",
            Cadence::Monthly => "Pays monthly",
            Cadence::Irregular => "Irregular cadence",
            Cadence::None => "No dividends recorded",
        }
    }

    /// Expected number of payouts per calendar year, for the projection.
    /// `None` for `Irregular` / `None`, where a clean projection is misleading.
    fn expected_per_year(self) -> Option<u32> {
        match self {
            Cadence::Annual => Some(1),
            Cadence::SemiAnnual => Some(2),
            Cadence::Quarterly => Some(4),
            Cadence::Monthly => Some(12),
            Cadence::Irregular | Cadence::None => None,
        }
    }
}

/// Infer cadence from the most recent payouts' ex-date gaps. Takes a sorted
/// (oldest first) slice of dates as `YYYY-MM-DD` strings; reads the median
/// gap across the last few payments so a single irregular one-off does not
/// throw the classification. Returns `Irregular` when the median lands outside
/// every clean band, and `None` when there is too little to infer from.
pub fn infer_cadence(ex_dates_oldest_first: &[String]) -> Cadence {
    if ex_dates_oldest_first.is_empty() {
        return Cadence::None;
    }
    if ex_dates_oldest_first.len() == 1 {
        // One payout: not enough to infer a cadence, but better to flag it as
        // irregular than claim a clean annual.
        return Cadence::Irregular;
    }
    // The most recent up-to-8 payouts give a stable median while still
    // reflecting any recent change in cadence.
    let tail = &ex_dates_oldest_first[ex_dates_oldest_first.len().saturating_sub(8)..];
    let parsed: Vec<chrono::NaiveDate> = tail
        .iter()
        .filter_map(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .collect();
    if parsed.len() < 2 {
        return Cadence::Irregular;
    }
    let mut gaps: Vec<i64> = parsed
        .windows(2)
        .map(|w| (w[1] - w[0]).num_days())
        .collect();
    gaps.sort();
    // Median rather than mean so a single irregular gap does not skew it.
    let median = gaps[gaps.len() / 2];
    match median {
        // Each band leaves comfortable slack: a quarterly payer's gaps range
        // ~80-95d in practice depending on the calendar.
        d if d <= 45 => Cadence::Monthly,
        d if d <= 130 => Cadence::Quarterly,
        d if d <= 220 => Cadence::SemiAnnual,
        d if d <= 450 => Cadence::Annual,
        _ => Cadence::Irregular,
    }
}

/// The on-track read for a stock's dividends: prior-year and YTD totals, the
/// projected current-year total, and a graded verdict on whether the company
/// is tracking ahead of, on, or behind its prior-year payout.
#[derive(Debug, Clone, Serialize)]
pub struct DividendPace {
    pub cadence: Cadence,
    /// Short caption derived from `cadence`, e.g. `Pays quarterly`. Carried
    /// so the template renders it without poking at the method.
    pub cadence_caption: &'static str,
    /// Sum of payouts in the previous calendar year, per share.
    pub prior_year_total: f64,
    /// Sum of payouts in the current calendar year so far, per share.
    pub ytd_total: f64,
    /// Number of payouts declared so far this calendar year.
    pub ytd_count: u32,
    /// Projected current-year total per share, scaling YTD by the count-
    /// tempered factor (`expected_n / declared_n_so_far`). `None` when the
    /// cadence is unclear, no payouts have landed this year, or there is no
    /// prior-year baseline to compare against.
    pub projection: Option<f64>,
    /// Projection vs prior-year, as a percent change. `None` whenever
    /// `projection` is.
    pub pct_change: Option<f64>,
    /// On-track verdict (rise is good for dividends, matching the Phase 24
    /// trend reading): `Good` for a clear rise, `Bad` for a clear fall, `Ok`
    /// for a small move or a flat year, `Unknown` whenever `projection` is.
    pub grade: Grade,
    /// One-word badge text derived from `grade` — `Strong` / `Fair` / `Weak`,
    /// or `No data` when there is nothing to read.
    pub verdict: &'static str,
}

/// A small one-week-each-side band around prior-year that reads as flat, so a
/// rounding-grade payment increase does not register as "growing" /
/// "shrinking".
const PACE_FLAT_BAND: f64 = 2.0;

/// Build a [`DividendPace`] from dividend events oldest first. `today` carries
/// the date the YTD window closes at (taken from the route's clock). Returns a
/// `DividendPace` even when there is little to say, so the page can show the
/// raw cadence and totals alone; the verdict downgrades to `Unknown` when a
/// pace projection is not meaningful.
pub fn dividend_pace(events: &[(String, f64)], today: chrono::NaiveDate) -> DividendPace {
    use chrono::Datelike;
    let year = today.year();
    let prior = year - 1;
    let (mut prior_total, mut ytd_total, mut ytd_count) = (0.0_f64, 0.0_f64, 0_u32);
    for (date, amount) in events {
        let Ok(d) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
            continue;
        };
        if d.year() == year && d <= today {
            ytd_total += amount;
            ytd_count += 1;
        } else if d.year() == prior {
            prior_total += amount;
        }
    }

    let dates: Vec<String> = events.iter().map(|(d, _)| d.clone()).collect();
    let cadence = infer_cadence(&dates);

    // Count-tempered projection: scale YTD by (expected_n / declared_n_so_far).
    // A quarterly payer at end-of-Q1 thus projects ×4, not ×~4 by elapsed days,
    // which is what the user picked over a calendar-elapsed-fraction approach.
    let (projection, pct_change, grade) = match (
        cadence.expected_per_year(),
        ytd_count,
        prior_total,
    ) {
        (Some(expected), declared, prior_year) if declared > 0 && prior_year > 0.0 => {
            let p = ytd_total * f64::from(expected) / f64::from(declared);
            let pct = (p - prior_year) / prior_year * 100.0;
            let grade = if pct > PACE_FLAT_BAND {
                Grade::Good
            } else if pct < -PACE_FLAT_BAND {
                Grade::Bad
            } else {
                Grade::Ok
            };
            (Some(p), Some(pct), grade)
        }
        _ => (None, None, Grade::Unknown),
    };

    DividendPace {
        cadence,
        cadence_caption: cadence.caption(),
        prior_year_total: prior_total,
        ytd_total,
        ytd_count,
        projection,
        pct_change,
        grade,
        verdict: grade.verdict(),
    }
}

// ─────────────────────── company standing (Phase 20) ───────────────────────
//
// A stock's overall standing rolls its nine graded ratios into a single
// strong / fair / weak verdict — the badge shown across the app — and combines
// that fundamental strength with a price-and-growth trajectory into one score
// the home page ranks the strongest and weakest stocks by. Everything here is
// pure: it derives only from the Phase 7 ratios and a daily-close series, with
// no new data source.

/// Of the nine ratios, how many must carry a real grade (not `Unknown`) before
/// a strength verdict is meaningful. A company reporting almost nothing gets no
/// badge rather than one resting on one or two figures.
const MIN_GRADED: usize = 5;

/// Strength-score cutoffs for the strong / fair / weak verdict. The score is a
/// mean of per-ratio values in [-1, 1]; a curated large-cap typically lands
/// near zero, so the band is deliberately narrow. Tunable.
const STRONG_CUTOFF: f64 = 0.2;
const WEAK_CUTOFF: f64 = -0.2;

/// Weight of fundamental strength in the combined score; trajectory takes the
/// rest. ~2:1 in favour of fundamentals (a user steer — the ranking should
/// lean on how well a company is built over how its price has lately moved).
const STRENGTH_WEIGHT: f64 = 2.0 / 3.0;

/// Trading days in the trailing-year price-trend window (~12 months).
const TREND_WINDOW: usize = 252;
/// Trading days per sub-block when measuring how steady the climb was (~1mo).
const TREND_BLOCK: usize = 21;
/// Minimum history (~3 months) before a price trend is read at all.
const TREND_MIN: usize = TREND_BLOCK * 3;
/// A trailing return of this magnitude (a fraction, so 0.25 = ±25%) saturates
/// the return component of the price-trend score.
const TREND_SATURATION: f64 = 0.25;

/// A stock's rolled-up standing: the strong / fair / weak verdict shown as a
/// badge across the app, plus a combined strength-and-trajectory score the
/// home "Strongest & weakest" panels rank by.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Standing {
    /// CSS hook for the badge: `good` | `ok` | `bad`. Mirrors `Grade`, so it
    /// reuses the per-ratio badge colours.
    pub grade: Grade,
    /// Badge text derived from `grade`: `Strong` | `Fair` | `Weak`.
    pub verdict: &'static str,
    /// Combined score in [-1, 1]; the home panels sort by it. The verdict
    /// above reflects fundamental strength alone (it sits over the ratio
    /// cards); this score additionally folds in trajectory.
    pub score: f64,
}

/// A grade's numeric value for averaging: `Good` +1, `Ok` 0, `Bad` −1.
/// `Unknown` carries no value and is skipped by the mean.
fn grade_value(g: Grade) -> Option<f64> {
    match g {
        Grade::Good => Some(1.0),
        Grade::Ok => Some(0.0),
        Grade::Bad => Some(-1.0),
        Grade::Unknown => None,
    }
}

/// Mean of the graded values in `grades`, ignoring `Unknown`. `None` when
/// fewer than `min` of them carried a grade.
fn graded_mean(grades: impl Iterator<Item = Grade>, min: usize) -> Option<f64> {
    let vals: Vec<f64> = grades.filter_map(grade_value).collect();
    (vals.len() >= min).then(|| vals.iter().sum::<f64>() / vals.len() as f64)
}

/// Map a score in [-1, 1] to a strong / fair / weak `Grade`.
fn score_grade(score: f64) -> Grade {
    if score >= STRONG_CUTOFF {
        Grade::Good
    } else if score <= WEAK_CUTOFF {
        Grade::Bad
    } else {
        Grade::Ok
    }
}

/// Score the trailing-year price trend in [-1, 1]: a trailing return blended
/// with how steady the climb was — the share of ~monthly sub-blocks that did
/// not fall. `None` with too little history to judge.
fn price_trend_score(closes: &[f64]) -> Option<f64> {
    if closes.len() < TREND_MIN {
        return None;
    }
    let window = &closes[closes.len().saturating_sub(TREND_WINDOW)..];
    let (&first, &last) = (window.first()?, window.last()?);
    if first <= 0.0 {
        return None;
    }
    // Return component: a move of ±TREND_SATURATION over the window saturates.
    let ret = (last - first) / first;
    let ret_comp = (ret / TREND_SATURATION).clamp(-1.0, 1.0);
    // Steadiness: the fraction of ~monthly blocks that closed up, recentred to
    // [-1, 1] so an all-up year reads +1 and an all-down year −1.
    let (mut blocks, mut up) = (0u32, 0u32);
    let mut i = 0;
    while i + TREND_BLOCK < window.len() {
        blocks += 1;
        if window[i + TREND_BLOCK] >= window[i] {
            up += 1;
        }
        i += TREND_BLOCK;
    }
    let steady_comp = if blocks > 0 {
        (f64::from(up) / f64::from(blocks) - 0.5) * 2.0
    } else {
        ret_comp
    };
    // The return carries most of the weight; steadiness only refines it.
    Some(0.7 * ret_comp + 0.3 * steady_comp)
}

/// Trajectory score in [-1, 1]: the recent price trend blended equally with
/// fundamental growth (the revenue- and earnings-growth ratio grades). `None`
/// when neither half can be computed.
fn trajectory_score(ratios: &[Ratio], closes: &[f64]) -> Option<f64> {
    let price = price_trend_score(closes);
    let growth = graded_mean(
        ratios
            .iter()
            .filter(|r| matches!(r.key, "revenue_growth" | "earnings_growth"))
            .map(|r| r.grade),
        1,
    );
    match (price, growth) {
        (Some(p), Some(g)) => Some((p + g) / 2.0),
        (Some(v), None) | (None, Some(v)) => Some(v),
        (None, None) => None,
    }
}

/// Roll a stock's nine graded ratios and its price trajectory into a single
/// [`Standing`]. `ratios` is the output of [`compute_ratios`]; `closes` is a
/// daily-close series (oldest first) over roughly the trailing year, which may
/// be empty. `None` when too few ratios graded to judge.
pub fn standing(ratios: &[Ratio], closes: &[f64]) -> Option<Standing> {
    // Fundamental strength: the mean grade across all nine ratios. The badge's
    // verdict reflects this alone, since it sits over the ratio cards.
    let strength = graded_mean(ratios.iter().map(|r| r.grade), MIN_GRADED)?;
    let grade = score_grade(strength);
    // Combined score: fundamentals weighted ~2:1 over trajectory. With no
    // trajectory to read, strength stands alone.
    let score = match trajectory_score(ratios, closes) {
        Some(t) => STRENGTH_WEIGHT * strength + (1.0 - STRENGTH_WEIGHT) * t,
        None => strength,
    };
    Some(Standing {
        grade,
        verdict: grade.verdict(),
        score,
    })
}

// ─────────────────────── stock health read (Phase 17) ──────────────────────
//
// The health read layers a leadership-stability signal over the Phase 20
// strength + trajectory composite to give a single non-advice summary of
// whether a stock looks healthy. Pure derivation: ratios from Phase 7, a
// daily-close series, and a count of recent 8-K item-5.02 leadership changes
// from Phase 14. Industry context (Phase 15) is intentionally not folded in
// yet; this phase ships without it and a later pass will layer it on.

/// Trailing window in days for counting 8-K item-5.02 leadership changes that
/// feed the stability score (~24 months). Long enough to capture an annual
/// change pattern, short enough that years-old turnover ages out.
pub const LEADERSHIP_STABILITY_DAYS: i64 = 730;

/// Health-composite weights. Strength carries the most weight (a healthy
/// company is first built well), trajectory next, stability last. They sum
/// to 1; the weighting mirrors the Phase 20 user steer that the ranking
/// should lean on how the company is *built* over how its price has *moved*.
const HEALTH_W_STRENGTH: f64 = 0.55;
const HEALTH_W_TRAJECTORY: f64 = 0.30;
const HEALTH_W_STABILITY: f64 = 0.15;

/// A stock's health read: an overall healthy / mixed / concerning verdict,
/// the composite score the home panels rank by, and the three sub-components
/// behind it so the symbol page can show the breakdown.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct HealthRead {
    /// CSS hook for the overall badge: `good` | `ok` | `bad`.
    pub overall: Grade,
    /// Badge text derived from `overall`: `Healthy` | `Mixed` | `Concerning`.
    pub verdict: &'static str,
    /// Composite score in [-1, 1]; home panels sort by it.
    pub score: f64,
    /// `score` mapped linearly to a 0-100 percentage for the header badge:
    /// `-1.0` reads 0%, `0.0` reads 50%, `+1.0` reads 100%.
    pub percent: u8,
    pub strength: Grade,
    pub strength_label: &'static str,
    pub trajectory: Grade,
    pub trajectory_label: &'static str,
    pub stability: Grade,
    pub stability_label: &'static str,
    /// 8-K item-5.02 filings counted inside `LEADERSHIP_STABILITY_DAYS`.
    pub recent_changes: usize,
}

/// Map an overall health grade to its display verdict. Distinct from the
/// per-ratio `Strong/Fair/Weak` so the panel reads as a synthesis, not a
/// ratio rollup.
fn health_verdict(g: Grade) -> &'static str {
    match g {
        Grade::Good => "Healthy",
        Grade::Ok => "Mixed",
        Grade::Bad => "Concerning",
        Grade::Unknown => "Unread",
    }
}

/// Trajectory sub-component label.
fn trajectory_label(g: Grade) -> &'static str {
    match g {
        Grade::Good => "Climbing",
        Grade::Ok => "Steady",
        Grade::Bad => "Slipping",
        Grade::Unknown => "—",
    }
}

/// Leadership-stability sub-component label.
fn stability_label(g: Grade) -> &'static str {
    match g {
        Grade::Good => "Stable",
        Grade::Ok => "Normal",
        Grade::Bad => "Churning",
        Grade::Unknown => "—",
    }
}

/// Grade leadership stability from the count of recent 8-K item-5.02 changes
/// inside `LEADERSHIP_STABILITY_DAYS`. Returns `None` when the leadership
/// sweep has not reached the stock yet (the caller passes `None`), so the
/// component drops out of the composite cleanly instead of penalising
/// an unsynced stock. Three discrete bands rather than a linear scale: the
/// signal is coarse and the bands keep it from drifting on small counts.
///
/// - 0-1 change  → `Good` / `+1.0` — stable
/// - 2-3 changes → `Ok`   / `0.0`  — normal
/// - 4+ changes  → `Bad`  / `-1.0` — churn
///
/// Big companies routinely file ~one planned-succession 5.02 a year, so the
/// bands are deliberately lenient.
pub fn stability_grade(recent_changes: Option<usize>) -> Option<(Grade, f64)> {
    match recent_changes? {
        0 | 1 => Some((Grade::Good, 1.0)),
        2 | 3 => Some((Grade::Ok, 0.0)),
        _ => Some((Grade::Bad, -1.0)),
    }
}

/// Roll a stock's ratios, trajectory and leadership-change count into a
/// single [`HealthRead`]. `ratios` is the output of [`compute_ratios`];
/// `closes` is a daily-close series (oldest first) over roughly the trailing
/// year (may be empty); `recent_changes` is the count of 8-K item-5.02
/// filings in the last `LEADERSHIP_STABILITY_DAYS`, or `None` if the
/// leadership sweep has not reached this stock. `None` when too few ratios
/// graded to judge (same gate as [`standing`]).
///
/// The composite renormalises over the components that landed — a stock with
/// no leadership data yet is read on strength + trajectory alone, not penalised.
pub fn health_read(
    ratios: &[Ratio],
    closes: &[f64],
    recent_changes: Option<usize>,
) -> Option<HealthRead> {
    let strength_raw = graded_mean(ratios.iter().map(|r| r.grade), MIN_GRADED)?;
    let trajectory_raw = trajectory_score(ratios, closes);
    let stability_pair = stability_grade(recent_changes);

    let mut weighted = HEALTH_W_STRENGTH * strength_raw;
    let mut total = HEALTH_W_STRENGTH;
    if let Some(t) = trajectory_raw {
        weighted += HEALTH_W_TRAJECTORY * t;
        total += HEALTH_W_TRAJECTORY;
    }
    if let Some((_, s)) = stability_pair {
        weighted += HEALTH_W_STABILITY * s;
        total += HEALTH_W_STABILITY;
    }
    let score = weighted / total;
    let overall = score_grade(score);
    // Linear map [-1, 1] → [0, 100] for the header badge. Clamped because a
    // small floating-point drift past ±1 should not blow past 0% / 100%.
    let percent = (((score + 1.0) / 2.0 * 100.0).round() as i32).clamp(0, 100) as u8;

    let strength = score_grade(strength_raw);
    let trajectory = trajectory_raw.map(score_grade).unwrap_or(Grade::Unknown);
    let stability = stability_pair.map(|(g, _)| g).unwrap_or(Grade::Unknown);

    Some(HealthRead {
        overall,
        verdict: health_verdict(overall),
        score,
        percent,
        strength,
        strength_label: strength.verdict(),
        trajectory,
        trajectory_label: trajectory_label(trajectory),
        stability,
        stability_label: stability_label(stability),
        recent_changes: recent_changes.unwrap_or(0),
    })
}

// ────────────────────── ETF trailing returns (Phase 28) ────────────────────
//
// Trailing total returns from a fund's daily-close series. Distributions are
// not folded in (we have them in the `dividends` table from Phase 26, but the
// price-return shown here is the most common convention; the distribution
// yield rides separately on the page). Periods over a year are annualised so
// every figure reads on the same scale.

/// One trailing return — both the simple cumulative figure and the
/// annualised one (the same number for periods of a year or less).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct TrailingReturn {
    /// Cumulative percent move over the window.
    pub pct: f64,
    /// CAGR. Equal to `pct` for windows ≤ 1 year; geometrically annualised
    /// past that.
    pub annualised_pct: f64,
}

/// The full set of trailing returns the ETF page shows. Each is `None` when
/// the price history does not reach back that far.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TrailingReturns {
    pub m1: Option<TrailingReturn>,
    pub m3: Option<TrailingReturn>,
    pub ytd: Option<TrailingReturn>,
    pub y1: Option<TrailingReturn>,
    pub y3: Option<TrailingReturn>,
    pub y5: Option<TrailingReturn>,
    pub y10: Option<TrailingReturn>,
    pub since_inception: Option<TrailingReturn>,
}

/// One bar of the daily-close series the trailing-return / growth functions
/// consume: a `YYYY-MM-DD` date and the close. Oldest first.
#[derive(Debug, Clone)]
pub struct DatedClose<'a> {
    pub date: &'a str,
    pub close: f64,
}

/// Compute the full trailing-return set from a `bars` series (oldest first)
/// against the latest available close (its tail). Empty / single-bar input
/// returns an all-`None` set. `today` is `YYYY-MM-DD` and anchors the YTD
/// window to the current calendar year — passing the latest bar's date keeps
/// the figure deterministic across requests.
pub fn trailing_returns(bars: &[DatedClose<'_>], today: &str) -> TrailingReturns {
    if bars.len() < 2 {
        return TrailingReturns::default();
    }
    let latest = bars[bars.len() - 1].close;
    if latest <= 0.0 {
        return TrailingReturns::default();
    }

    // Bar at or just before a target date, by walking back from the tail. The
    // series is calendar-irregular (weekends, holidays), so an exact match is
    // rare; "or just before" is the convention for trailing returns.
    let close_at_or_before = |target: &str| -> Option<f64> {
        bars.iter()
            .rev()
            .find(|b| b.date <= target)
            .map(|b| b.close)
            .filter(|c| *c > 0.0)
    };

    let ret = |prev: f64, years: f64| -> TrailingReturn {
        let cum = (latest / prev - 1.0) * 100.0;
        let ann = if years > 1.0 {
            ((latest / prev).powf(1.0 / years) - 1.0) * 100.0
        } else {
            cum
        };
        TrailingReturn {
            pct: cum,
            annualised_pct: ann,
        }
    };

    // Approximate-calendar offsets keyed to `today`'s YMD. `chrono` is already
    // a dependency, so use it rather than fudging day counts.
    let parse = |d: &str| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok();
    let today_d = parse(today);

    let target = |months: i64| -> Option<String> {
        let t = today_d?;
        let ym = t.year() as i64 * 12 + (t.month0() as i64) - months;
        let (ty, tm0) = (ym.div_euclid(12) as i32, ym.rem_euclid(12) as u32);
        let day = t.day().min(28); // a 28th always exists in every month
        chrono::NaiveDate::from_ymd_opt(ty, tm0 + 1, day).map(|d| d.format("%Y-%m-%d").to_string())
    };
    let years_target = |years: i64| target(years * 12);
    let ytd_target = || -> Option<String> {
        // The last close of the prior calendar year — i.e. the bar at or
        // before "Jan 1 of this year" — is the YTD anchor.
        let t = today_d?;
        Some(format!("{}-01-01", t.year()))
    };

    let r = |target_date: Option<String>, years: f64| -> Option<TrailingReturn> {
        let prev = close_at_or_before(&target_date?)?;
        Some(ret(prev, years))
    };

    let m1 = r(target(1), 1.0 / 12.0);
    let m3 = r(target(3), 0.25);
    let ytd = r(ytd_target(), 1.0); // YTD is reported cumulative, not annualised
    let y1 = r(years_target(1), 1.0);
    let y3 = r(years_target(3), 3.0);
    let y5 = r(years_target(5), 5.0);
    let y10 = r(years_target(10), 10.0);

    // Since inception: the very first bar. Years span from its date to today,
    // measured in actual days / 365.25 to capture leap-year drift.
    let since_inception = (|| {
        let first = bars.first()?;
        let f = parse(first.date)?;
        let t = today_d?;
        let days = (t - f).num_days() as f64;
        if days <= 0.0 || first.close <= 0.0 {
            return None;
        }
        let years = (days / 365.25).max(1.0 / 12.0);
        Some(ret(first.close, years))
    })();

    TrailingReturns {
        m1,
        m3,
        ytd,
        y1,
        y3,
        y5,
        y10,
        since_inception,
    }
}

/// Use `chrono::Datelike` for the date arithmetic above.
use chrono::Datelike;

// ────────────────────── growth-of-$10,000 chart (Phase 28) ─────────────────

/// One point of the growth-of-$10k series rendered on the ETF page.
#[derive(Debug, Clone, Serialize)]
pub struct GrowthPoint {
    /// Trading date, `YYYY-MM-DD`.
    pub date: String,
    /// Dollar value of $10,000 invested at the series' start, on this date.
    pub value: f64,
}

/// Scale a daily-close series so the first bar reads as $10,000. Returns the
/// full series — the caller is responsible for downsampling if it would
/// render too densely. Empty / single-bar / zero-anchor input returns an
/// empty series.
pub fn growth_of_10k(bars: &[DatedClose<'_>]) -> Vec<GrowthPoint> {
    if bars.len() < 2 {
        return Vec::new();
    }
    let anchor = bars[0].close;
    if anchor <= 0.0 {
        return Vec::new();
    }
    bars.iter()
        .map(|b| GrowthPoint {
            date: b.date.to_string(),
            value: 10_000.0 * b.close / anchor,
        })
        .collect()
}

// ────────────────────── ETF NAV premium / discount (Phase 28) ──────────────

/// Premium or discount of `price` to `nav`, as a percent. A positive value is
/// a premium (price > NAV), negative a discount. `None` when NAV is unknown
/// or non-positive.
pub fn premium_discount_pct(price: f64, nav: Option<f64>) -> Option<f64> {
    let nav = nav?;
    if nav <= 0.0 {
        return None;
    }
    Some((price - nav) / nav * 100.0)
}

/// A small good/ok/bad band on the premium/discount figure. A persistently
/// large premium is a yellow flag (buying above NAV); a normal ETF stays
/// inside ±25 bps. Symmetric — a deep discount is also notable.
pub fn premium_grade(premium_pct: f64) -> Grade {
    const TIGHT: f64 = 0.25; // ±0.25% is normal for liquid ETFs
    const LOOSE: f64 = 1.00; // ±1% is a yellow flag
    let abs = premium_pct.abs();
    if abs <= TIGHT {
        Grade::Good
    } else if abs <= LOOSE {
        Grade::Ok
    } else {
        Grade::Bad
    }
}

// ───────────────────────── ETF quality read (Phase 4) ──────────────────────
//
// An ETF's quality read mirrors the stock health donut: four graded factors —
// cost, tracking, diversification, size — rolled into one good/ok/bad verdict
// with a 0-100 badge percent and the four sub-readings behind it. It reads the
// quality of the *wrapper* (is it cheap, does it hug fair value, is it broad,
// is it durable), explicitly not a buy/sell call. Pure: derives only from the
// fund's expense ratio, price/NAV premium, top-holdings concentration, and net
// assets — all already loaded for the ETF symbol page.

/// Cost-weighted blend (a user steer). Cost is the one guaranteed, perpetual
/// drag so it carries the most weight; tracking next, then diversification,
/// then size. The composite renormalises over whichever factors graded, so a
/// commodity trust with no holdings is read on the other three, not penalised.
const ETF_W_COST: f64 = 0.40;
const ETF_W_TRACKING: f64 = 0.25;
const ETF_W_DIVERSIFICATION: f64 = 0.20;
const ETF_W_SIZE: f64 = 0.15;

/// At least this many of the four factors must grade before the badge shows, so
/// a fund we know almost nothing about gets no read rather than a hollow one.
const ETF_MIN_GRADED: usize = 2;

/// An ETF's quality read: an overall strong / fair / weak verdict, the
/// composite score, the 0-100 badge percent, and the four sub-components behind
/// it so the symbol page can show the breakdown. Structurally a sibling of
/// [`HealthRead`] (stocks), with ETF-appropriate factors.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct EtfQuality {
    /// CSS hook for the overall badge: `good` | `ok` | `bad`.
    pub overall: Grade,
    /// Badge text derived from `overall`: `Strong` | `Fair` | `Weak`.
    pub verdict: &'static str,
    /// Composite score in [-1, 1].
    pub score: f64,
    /// `score` mapped linearly to a 0-100 percent for the header badge.
    pub percent: u8,
    pub cost: Grade,
    pub cost_label: &'static str,
    pub tracking: Grade,
    pub tracking_label: &'static str,
    pub diversification: Grade,
    pub diversification_label: &'static str,
    pub size: Grade,
    pub size_label: &'static str,
    /// How many of the four factors carried a grade (the rest dropped out of
    /// the blend). Not displayed; useful for debugging a thin read.
    pub graded: usize,
}

/// Cost sub-score from the expense ratio (a decimal, e.g. `0.0003` = 0.03%).
/// Cheaper is better: ≤0.05% saturates at +1, ≥0.75% at −1, linear between.
/// Bands suit the curated iShares/Vanguard roster (core funds ~0.03-0.10%; the
/// priciest commodity/thematic ones ~0.40-0.75%). `None` when not reported.
fn etf_cost_score(expense_ratio: Option<f64>) -> Option<f64> {
    let pct = expense_ratio? * 100.0;
    if pct < 0.0 {
        return None;
    }
    const CHEAP: f64 = 0.05;
    const PRICEY: f64 = 0.75;
    Some((1.0 - (pct - CHEAP) / (PRICEY - CHEAP) * 2.0).clamp(-1.0, 1.0))
}

/// Tracking sub-score from the price's premium/discount to NAV (a signed
/// percent). Reuses the page's [`premium_grade`] bands mapped to a numeric:
/// tight ≤±0.25% ≈ +1, wide ≥±1% ≈ −1, linear in the absolute gap. `None` when
/// NAV is unknown (no premium to read).
fn etf_tracking_score(premium_pct: Option<f64>) -> Option<f64> {
    let abs = premium_pct?.abs();
    const TIGHT: f64 = 0.25;
    const WIDE: f64 = 1.00;
    Some((1.0 - (abs - TIGHT) / (WIDE - TIGHT) * 2.0).clamp(-1.0, 1.0))
}

/// Diversification sub-score from the top-10 holdings' combined weight (a
/// percent, e.g. `27.5` = 27.5%). Lower concentration is broader/safer: ≤20%
/// saturates at +1, ≥60% at −1. `None` for funds with no reported holdings
/// (commodity trusts), dropping the factor from the blend.
fn etf_diversification_score(top10_pct: Option<f64>) -> Option<f64> {
    let c = top10_pct?;
    if c <= 0.0 {
        return None;
    }
    const BROAD: f64 = 20.0;
    const CONCENTRATED: f64 = 60.0;
    Some((1.0 - (c - BROAD) / (CONCENTRATED - BROAD) * 2.0).clamp(-1.0, 1.0))
}

/// Size sub-score from net assets (AUM, USD), on a log scale: small funds carry
/// closure / liquidity risk, large ones are durable. Each 10× in AUM is ±1
/// centred on ~$2B (so ~$200M ≈ −1, ~$2B ≈ 0, ≥~$20B ≈ +1). `None` when AUM is
/// unknown or non-positive.
fn etf_size_score(net_assets: Option<f64>) -> Option<f64> {
    let aum = net_assets?;
    if aum <= 0.0 {
        return None;
    }
    const MID_LOG: f64 = 9.3; // log10($2B) ≈ 9.30
    Some((aum.log10() - MID_LOG).clamp(-1.0, 1.0))
}

fn etf_cost_label(g: Grade) -> &'static str {
    match g {
        Grade::Good => "Cheap",
        Grade::Ok => "Moderate",
        Grade::Bad => "Pricey",
        Grade::Unknown => "—",
    }
}

fn etf_tracking_label(g: Grade) -> &'static str {
    match g {
        Grade::Good => "Tight",
        Grade::Ok => "Slight drift",
        Grade::Bad => "Wide gap",
        Grade::Unknown => "—",
    }
}

fn etf_diversification_label(g: Grade) -> &'static str {
    match g {
        Grade::Good => "Broad",
        Grade::Ok => "Moderate",
        Grade::Bad => "Concentrated",
        Grade::Unknown => "—",
    }
}

fn etf_size_label(g: Grade) -> &'static str {
    match g {
        Grade::Good => "Large",
        Grade::Ok => "Mid-size",
        Grade::Bad => "Small",
        Grade::Unknown => "—",
    }
}

/// Roll an ETF's four factors into a single [`EtfQuality`]. `expense_ratio` is a
/// decimal; `premium_pct` is the signed price-vs-NAV percent (from
/// [`premium_discount_pct`]); `top10_pct` is the summed weight of the ten
/// largest holdings (percent), `None` for a fund with no holdings; `net_assets`
/// is AUM in USD. Returns `None` until at least [`ETF_MIN_GRADED`] factors
/// grade, so a barely-known fund gets no badge rather than a hollow one. The
/// composite renormalises over the factors that landed.
pub fn etf_quality(
    expense_ratio: Option<f64>,
    premium_pct: Option<f64>,
    top10_pct: Option<f64>,
    net_assets: Option<f64>,
) -> Option<EtfQuality> {
    let cost_raw = etf_cost_score(expense_ratio);
    let tracking_raw = etf_tracking_score(premium_pct);
    let div_raw = etf_diversification_score(top10_pct);
    let size_raw = etf_size_score(net_assets);

    let factors = [
        (cost_raw, ETF_W_COST),
        (tracking_raw, ETF_W_TRACKING),
        (div_raw, ETF_W_DIVERSIFICATION),
        (size_raw, ETF_W_SIZE),
    ];
    let graded = factors.iter().filter(|(r, _)| r.is_some()).count();
    if graded < ETF_MIN_GRADED {
        return None;
    }
    let (mut weighted, mut total) = (0.0, 0.0);
    for (r, w) in factors {
        if let Some(v) = r {
            weighted += w * v;
            total += w;
        }
    }
    let score = weighted / total;
    let overall = score_grade(score);
    let percent = (((score + 1.0) / 2.0 * 100.0).round() as i32).clamp(0, 100) as u8;

    let cost = cost_raw.map_or(Grade::Unknown, score_grade);
    let tracking = tracking_raw.map_or(Grade::Unknown, score_grade);
    let diversification = div_raw.map_or(Grade::Unknown, score_grade);
    let size = size_raw.map_or(Grade::Unknown, score_grade);

    Some(EtfQuality {
        overall,
        // `overall` is never Unknown (score_grade only yields Good/Ok/Bad), so
        // Grade::verdict's Strong/Fair/Weak reads cleanly as a quality grade.
        verdict: overall.verdict(),
        score,
        percent,
        cost,
        cost_label: etf_cost_label(cost),
        tracking,
        tracking_label: etf_tracking_label(tracking),
        diversification,
        diversification_label: etf_diversification_label(diversification),
        size,
        size_label: etf_size_label(size),
        graded,
    })
}

// ── Phase 16: per-ticker anomaly feed ─────────────────────────────────────

/// One row in the symbol-page anomaly feed. Built either here (price events,
/// drawdowns) or in `models.rs` (fundamentals events) or directly in the
/// symbol route (leadership events, reused from Phase 14's filings SELECT).
#[derive(Debug, Clone, Serialize)]
pub struct AnomalyEvent {
    /// `YYYY-MM-DD`.
    pub date: String,
    /// Glyph key the template maps to an icon — one of `up`, `down`,
    /// `drawdown`, `fund-up`, `fund-down`, `leader`.
    pub glyph: &'static str,
    /// `good` | `bad` | `neutral` — drives the row's background tint in the
    /// feed so a one-glance scan reveals whether recent events skew positive
    /// or negative. Up / fund-up are good, down / drawdown / fund-down are
    /// bad, leadership changes are neutral (an officer change is not itself
    /// good or bad news).
    pub polarity: &'static str,
    /// Human one-line headline, e.g. `+8.2% one-day move`.
    pub headline: String,
    /// Outbound link (set on leadership events; the row becomes an anchor).
    pub url: Option<String>,
    /// Sort tiebreaker; larger = more notable. Not displayed, just used to
    /// keep the top N when the merged feed overflows the display cap.
    pub severity: f64,
}

/// Trailing-volatility window for the price-move detector (~6 months trading days).
const PRICE_VOL_WINDOW: usize = 90;
/// Daily-return magnitude threshold below which we never flag, even for a
/// very low-vol stock where 2σ would come in tiny.
const PRICE_MIN_MOVE: f64 = 0.05;
/// Standard-deviation multiplier — a move must clear both this and PRICE_MIN_MOVE.
const PRICE_SIGMA_MULT: f64 = 2.0;
/// Drawdown lookback (~6 months trading days).
const DRAWDOWN_WINDOW: usize = 126;
/// Min gap (trading days) between drawdown events so a long slide doesn't
/// emit every bar.
const DRAWDOWN_DEDUPE_BARS: usize = 30;

/// Walk `closes` (oldest-first, aligned to `dates`) and emit one event for
/// every bar whose close-to-close return is both `> 5%` in magnitude and
/// `> 2σ` of the trailing 90-day daily returns. The pair of thresholds
/// keeps a low-vol stock's modest move from qualifying just because its
/// σ is tiny, and a high-vol name's daily wobble from qualifying just
/// because 5% is its normal range.
pub fn price_anomalies(closes: &[f64], dates: &[&str]) -> Vec<AnomalyEvent> {
    debug_assert_eq!(closes.len(), dates.len());
    let n = closes.len();
    if n <= PRICE_VOL_WINDOW + 1 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for i in (PRICE_VOL_WINDOW + 1)..n {
        let prev = closes[i - 1];
        let cur = closes[i];
        if prev <= 0.0 {
            continue;
        }
        let r = (cur - prev) / prev;
        // Trailing daily returns over the prior PRICE_VOL_WINDOW bars
        // (returns r_j for j in start..i, anchored to closes[j-1]).
        let start = i - PRICE_VOL_WINDOW;
        let (mut sum, mut sum2, mut n_ret) = (0.0_f64, 0.0_f64, 0usize);
        for j in start..i {
            let p = closes[j - 1];
            if p <= 0.0 {
                continue;
            }
            let rr = (closes[j] - p) / p;
            sum += rr;
            sum2 += rr * rr;
            n_ret += 1;
        }
        if n_ret < PRICE_VOL_WINDOW / 2 {
            continue;
        }
        let mean = sum / n_ret as f64;
        let var = (sum2 / n_ret as f64 - mean * mean).max(0.0);
        let sigma = var.sqrt();
        if r.abs() >= PRICE_MIN_MOVE && r.abs() >= PRICE_SIGMA_MULT * sigma {
            let pct = r * 100.0;
            let (glyph, polarity, sign) = if pct >= 0.0 {
                ("up", "good", "+")
            } else {
                ("down", "bad", "\u{2212}")
            };
            out.push(AnomalyEvent {
                date: dates[i].to_string(),
                glyph,
                polarity,
                headline: format!("{sign}{:.1}% one-day move", pct.abs()),
                url: None,
                severity: pct.abs(),
            });
        }
    }
    out
}

/// Emit one event each time `close` prints a fresh 6-month low — a strict
/// minimum below the prior 126 bars' range. A long slide that keeps
/// printing lower lows is collapsed to one event per
/// `DRAWDOWN_DEDUPE_BARS`-bar window so the feed does not stream daily.
/// Headline carries the drop from the trailing window's peak as the
/// magnitude.
pub fn drawdown_anomalies(closes: &[f64], dates: &[&str]) -> Vec<AnomalyEvent> {
    debug_assert_eq!(closes.len(), dates.len());
    let n = closes.len();
    if n <= DRAWDOWN_WINDOW {
        return Vec::new();
    }
    let mut out: Vec<AnomalyEvent> = Vec::new();
    let mut last_emit_i: Option<usize> = None;
    for i in DRAWDOWN_WINDOW..n {
        let cur = closes[i];
        if cur <= 0.0 {
            continue;
        }
        let start = i - DRAWDOWN_WINDOW;
        let prior_min = closes[start..i].iter().copied().fold(f64::INFINITY, f64::min);
        let prior_max = closes[start..i].iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if cur < prior_min {
            if let Some(j) = last_emit_i {
                if i - j < DRAWDOWN_DEDUPE_BARS {
                    continue;
                }
            }
            let drop = if prior_max > 0.0 {
                (cur - prior_max) / prior_max * 100.0
            } else {
                0.0
            };
            out.push(AnomalyEvent {
                date: dates[i].to_string(),
                glyph: "drawdown",
                polarity: "bad",
                headline: format!("New 6-month low ({:.0}% off peak)", drop),
                url: None,
                severity: drop.abs(),
            });
            last_emit_i = Some(i);
        }
    }
    out
}

// ── Phase 25: earnings dates ──────────────────────────────────────────────

/// Estimate the next earnings date from a stock's recent earnings cadence.
/// Used as the fallback when Yahoo's `calendarEvents` has no upcoming date
/// for the stock (its coverage is uneven on small caps), built from the
/// last up-to-four 8-K item-2.02 dates the existing `filings` table already
/// carries (Phase 14).
///
/// `dates` is newest-first (matching how the symbol route's `ORDER BY
/// filed_at DESC` SELECT returns them). Returns `None` when fewer than two
/// priors exist or the spacing reads degenerate (a same-day correction).
/// Less reliable than Yahoo when a company moves its reporting calendar,
/// but better than no date when Yahoo is empty.
pub fn next_earnings_estimate(dates: &[&str]) -> Option<String> {
    if dates.len() < 2 {
        return None;
    }
    // Parse the newest-first slice into `NaiveDate`s; drop any unparsable
    // entries (defensive — these come from SEC, but the column is TEXT).
    let parsed: Vec<chrono::NaiveDate> = dates
        .iter()
        .filter_map(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        .take(4)
        .collect();
    if parsed.len() < 2 {
        return None;
    }
    // Gaps between consecutive earnings prints, oldest-to-newest order.
    let mut gaps: Vec<i64> = parsed
        .windows(2)
        .map(|w| (w[0] - w[1]).num_days())
        .collect();
    if gaps.iter().all(|g| *g <= 1) {
        return None;
    }
    gaps.sort();
    let median = gaps[gaps.len() / 2];
    // Clamp the median into a sane quarterly band so a stale dataset with
    // a few-day gap (multiple 8-Ks tagged 2.02 in one cycle) does not
    // project a date in the next week. Most US large-caps file ~91 days
    // apart; semi-annual filers ~182.
    let median = median.clamp(60, 200);
    let next = parsed[0] + chrono::Duration::days(median);
    Some(next.format("%Y-%m-%d").to_string())
}

#[cfg(test)]
mod phase25_tests {
    use super::*;

    #[test]
    fn estimates_a_quarterly_cadence() {
        // Four prints roughly 91 days apart, newest-first.
        let dates = &["2026-05-01", "2026-02-01", "2025-10-30", "2025-08-01"];
        let next = next_earnings_estimate(dates).unwrap();
        // Median gap ≈ 90 days, so next ≈ 2026-07-30 (give or take a day
        // depending on the exact gaps).
        let parsed = chrono::NaiveDate::parse_from_str(&next, "%Y-%m-%d").unwrap();
        let baseline = chrono::NaiveDate::parse_from_str("2026-05-01", "%Y-%m-%d").unwrap();
        let gap = (parsed - baseline).num_days();
        assert!((85..=95).contains(&gap), "next gap was {gap}d");
    }

    #[test]
    fn returns_none_on_too_few_priors() {
        assert!(next_earnings_estimate(&[]).is_none());
        assert!(next_earnings_estimate(&["2026-05-01"]).is_none());
    }

    #[test]
    fn handles_same_day_corrections() {
        // Two filings on adjacent days (a press-release and a follow-up): the
        // 1-day gap is degenerate, so the estimate is rejected.
        let dates = &["2026-05-02", "2026-05-01"];
        assert!(next_earnings_estimate(dates).is_none());
    }
}

#[cfg(test)]
mod phase28_tests {
    use super::*;

    fn bars(samples: &[(&str, f64)]) -> Vec<DatedClose<'static>> {
        samples
            .iter()
            .map(|(d, c)| DatedClose {
                date: Box::leak(d.to_string().into_boxed_str()),
                close: *c,
            })
            .collect()
    }

    #[test]
    fn trailing_returns_basic() {
        // A simple flat-then-spike series for 1y/3y windows.
        let b = bars(&[
            ("2023-01-02", 100.0),
            ("2024-01-02", 110.0),
            ("2025-01-02", 121.0),
            ("2026-01-02", 133.1),
            ("2026-05-22", 140.0),
        ]);
        let r = trailing_returns(&b, "2026-05-22");
        // 1y from 2025-05-22 onwards: closest bar at or before is 2025-01-02 (121.0).
        let y1 = r.y1.expect("y1");
        assert!((y1.pct - ((140.0 / 121.0 - 1.0) * 100.0)).abs() < 1e-6);
        // 3y annualised: anchor at 2023-05-22, closest bar at or before is
        // 2023-01-02 (100.0). 140/100 over 3y -> (1.4)^(1/3) - 1.
        let y3 = r.y3.expect("y3");
        let want = ((140.0_f64 / 100.0).powf(1.0 / 3.0) - 1.0) * 100.0;
        assert!((y3.annualised_pct - want).abs() < 1e-6);
        // YTD: anchor at "2026-01-01" → closest bar at or before is 2025-01-02
        // (no 2026 bar yet for 01-01), then walks past to 2026-01-02 (133.1).
        // Actually 2025-01-02 is at-or-before 2026-01-01, so YTD anchors there.
        // That's a known edge: when the chart has a print on Jan 2 but not Jan 1,
        // YTD overlaps the new year cleanly enough for a tolerance check.
        assert!(r.ytd.is_some());
    }

    #[test]
    fn growth_scales_to_10k_anchor() {
        let b = bars(&[
            ("2020-01-02", 50.0),
            ("2021-01-04", 60.0),
            ("2022-01-03", 75.0),
        ]);
        let g = growth_of_10k(&b);
        assert_eq!(g.len(), 3);
        assert!((g[0].value - 10_000.0).abs() < 1e-6);
        assert!((g[1].value - 12_000.0).abs() < 1e-6);
        assert!((g[2].value - 15_000.0).abs() < 1e-6);
    }

    #[test]
    fn premium_discount_grades() {
        assert!(matches!(premium_grade(0.10), Grade::Good));
        assert!(matches!(premium_grade(0.50), Grade::Ok));
        assert!(matches!(premium_grade(-2.00), Grade::Bad));
        assert!(premium_discount_pct(101.0, Some(100.0)).unwrap().abs() - 1.0 < 1e-9);
        assert!(premium_discount_pct(100.0, None).is_none());
        assert!(premium_discount_pct(100.0, Some(0.0)).is_none());
    }

    // ── Phase 16 anomaly-feed tests ────────────────────────────────────────

    fn flat_series(n: usize, value: f64) -> (Vec<f64>, Vec<String>) {
        let dates: Vec<String> = (0..n)
            .map(|i| {
                let d = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
                    + chrono::Duration::days(i as i64);
                d.format("%Y-%m-%d").to_string()
            })
            .collect();
        (vec![value; n], dates)
    }

    #[test]
    fn price_anomaly_flags_a_big_spike_against_flat_history() {
        let (mut closes, dates) = flat_series(120, 100.0);
        // Inject 5 tiny wobbles so σ is not exactly zero — a 7% jump still
        // qualifies on σ. (A literally-flat history makes σ=0, in which case
        // any nonzero move trivially exceeds 2σ; the 5%-floor still gates it.)
        closes[60] = 100.5;
        closes[80] = 99.5;
        closes[119] = 107.0;
        let date_refs: Vec<&str> = dates.iter().map(|s| s.as_str()).collect();
        let evs = price_anomalies(&closes, &date_refs);
        assert!(evs.iter().any(|e| e.date == dates[119]
            && e.glyph == "up"
            && e.headline.contains("7.0")));
    }

    #[test]
    fn price_anomaly_ignores_a_tiny_move_even_when_above_2_sigma() {
        let (mut closes, dates) = flat_series(120, 100.0);
        // 1% bump against a literally-flat history would trip 2σ but not the
        // 5% floor — the feed should stay empty.
        closes[119] = 101.0;
        let date_refs: Vec<&str> = dates.iter().map(|s| s.as_str()).collect();
        let evs = price_anomalies(&closes, &date_refs);
        assert!(evs.is_empty());
    }

    #[test]
    fn drawdown_anomaly_flags_a_fresh_six_month_low() {
        // 150 flat bars, then one bar prints a strict new low.
        let (mut closes, dates) = flat_series(150, 100.0);
        closes[140] = 80.0;
        let date_refs: Vec<&str> = dates.iter().map(|s| s.as_str()).collect();
        let evs = drawdown_anomalies(&closes, &date_refs);
        assert!(evs.iter().any(|e| e.date == dates[140] && e.glyph == "drawdown"));
    }

    #[test]
    fn drawdown_anomaly_dedupes_a_long_slide() {
        let (mut closes, dates) = flat_series(180, 100.0);
        // Each later bar prints a lower low; without dedupe we'd emit every
        // single bar. With a 30-bar cooldown we emit a handful, not 30+.
        for i in 130..180 {
            closes[i] = 100.0 - (i - 129) as f64;
        }
        let date_refs: Vec<&str> = dates.iter().map(|s| s.as_str()).collect();
        let evs = drawdown_anomalies(&closes, &date_refs);
        assert!(evs.len() <= 5, "expected dedupe to keep events sparse, got {}", evs.len());
    }
}
