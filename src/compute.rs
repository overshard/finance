//! Derived market figures. Ratios live here, not in the database, so they
//! always reflect the latest price.

use serde::Serialize;

/// Placeholder shown for a figure that cannot be computed. Matches the
/// `templates.rs` empty-value glyph (a middle dot).
const DASH: &str = "\u{00b7}";

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

// ──────────────────────────── dashboard sparkline ──────────────────────────
//
// Phase 11. Geometry for the tiny intraday line on each home-dashboard card.
// Drawn server-side from the latest session's bar closes; the stream client
// then nudges the trailing point as live quotes arrive.

/// Fixed viewBox the sparklines are drawn in: 100 wide, 36 tall. The line is
/// confined to y ∈ [`SPARK_TOP`, `SPARK_BOTTOM`] so a 1-2px stroke never clips
/// at the edges. The stream client's live-tip code mirrors these numbers.
const SPARK_W: f64 = 100.0;
const SPARK_TOP: f64 = 3.0;
const SPARK_BOTTOM: f64 = 33.0;

/// Geometry for one dashboard sparkline.
#[derive(Debug, Clone, Serialize)]
pub struct Sparkline {
    /// Polyline points (`"x,y x,y …"`, oldest first) for the line itself.
    pub line: String,
    /// The line closed down to the baseline at both ends, for the area fill.
    pub area: String,
    /// Value range the y-axis maps. The stream client places a live price on
    /// this same scale to move the trailing point.
    pub lo: f64,
    pub hi: f64,
    /// y of the previous close, drawn as a faint reference rule; `None` when
    /// no prior close is known.
    pub baseline: Option<f64>,
}

/// Build a [`Sparkline`] from a session's intraday closes (oldest first) and,
/// when known, the prior close. `None` for an empty series. The value range is
/// widened to include `prev_close` so the reference rule always lands in box.
pub fn sparkline(closes: &[f64], prev_close: Option<f64>) -> Option<Sparkline> {
    if closes.is_empty() {
        return None;
    }
    let mut lo = closes.iter().copied().fold(f64::INFINITY, f64::min);
    let mut hi = closes.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if let Some(p) = prev_close {
        lo = lo.min(p);
        hi = hi.max(p);
    }
    // Map a value to a y coordinate: a higher value sits closer to the top
    // (smaller y). A flat series (hi == lo) pins to the vertical midpoint.
    let y = |v: f64| -> f64 {
        let t = if hi > lo { (v - lo) / (hi - lo) } else { 0.5 };
        let yy = SPARK_BOTTOM - t * (SPARK_BOTTOM - SPARK_TOP);
        (yy * 100.0).round() / 100.0
    };
    // Evenly space the points across the full width; a lone point centres.
    let x = |i: usize| -> f64 {
        let xx = if closes.len() > 1 {
            i as f64 / (closes.len() - 1) as f64 * SPARK_W
        } else {
            SPARK_W / 2.0
        };
        (xx * 100.0).round() / 100.0
    };

    let mut line = String::new();
    for (i, &c) in closes.iter().enumerate() {
        if i > 0 {
            line.push(' ');
        }
        line.push_str(&format!("{},{}", x(i), y(c)));
    }
    let area = format!(
        "{},{} {} {},{}",
        x(0),
        SPARK_BOTTOM,
        line,
        x(closes.len() - 1),
        SPARK_BOTTOM,
    );

    Some(Sparkline {
        line,
        area,
        lo,
        hi,
        baseline: prev_close.map(y),
    })
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
