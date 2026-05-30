//! `GET /` — the markets dashboard.
//!
//! An opinionated, no-customization read of the market: a row of sparkline
//! cards for the major US indexes and the headline commodities, the day's
//! biggest movers, and a strongest / weakest read over the curated large-cap
//! stocks. There is deliberately no per-user layout — the app decides what
//! matters (see PLAN.md Phases 11 and 20). The full, browsable universe lives
//! on `/search`.

use std::cmp::Ordering;
use std::collections::HashMap;

use axum::{extract::State, response::Response, routing::get, Router};
use serde::Serialize;

use crate::compute::{self, Sparkline};
use crate::market;
use crate::models;
use crate::render::render;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(home))
}

/// The dashboard's index cards: each cash index paired with its index future.
/// Outside the regular cash session the future is shown in the card's place
/// (it trades nearly around the clock, while the cash index sits frozen on its
/// last close; see PLAN.md Phase 21). The Nasdaq Composite (`^NDQ`) has no
/// clean tradable future, so it always shows the cash index. Hardcoded on
/// purpose: the home page is a fixed, opinionated view, not a user-built
/// watchlist. Five cards = one clean desktop row.
const INDEXES: &[(&str, Option<&str>)] = &[
    ("^SPX", Some("ES=F")),
    ("^DJI", Some("YM=F")),
    ("^NDX", Some("NQ=F")),
    ("^RUT", Some("RTY=F")),
    ("^NDQ", None),
];

/// The dashboard's risk + commodity cards: the volatility gauge first, then
/// WTI crude, gold, and natural gas. Shown as the futures themselves for the
/// three commodities (no cash instrument to swap to) and as the cash index
/// for `^VIX` (no tradable future for it on Yahoo). VIX leads here rather
/// than living in the Indexes row because it is a derived sentiment gauge,
/// not a price index — putting it first turns the section into a quick
/// "what is the market worried about today" panel.
const COMMODITIES: &[&str] = &["^VIX", "CL=F", "GC=F", "NG=F"];

/// The dashboard's curated ETF cards (Phase 5): one broad-equity anchor
/// (`VOO`), a total-market and a growth/tech tilt (`VTI`, `QQQ`), a bond core
/// (`BND`), and a commodity (`GLD`) — five cards spanning the asset classes a
/// watcher scans first, mirroring the hardcoded `INDEXES` row. Each shows an
/// intraday sparkline plus its Phase-4 quality verdict. The full ETF universe
/// (and the movers strip below) lives on `/search?kind=etf`.
const ETF_CARDS: &[&str] = &["VOO", "VTI", "QQQ", "BND", "GLD"];

/// How many ETF gainers / losers the dashboard's compact movers strip lists.
const ETF_MOVERS_LIMIT: usize = 5;

/// NAV freshness gate for the ETF quality read's tracking factor (Phase 4):
/// NAV is struck daily, so a price-vs-NAV premium read against a NAV older than
/// this is meaningless and the factor drops out. Mirrors the symbol page.
const NAV_FRESH_MS: i64 = 3 * 24 * 3600 * 1000;

/// How many gainers and how many losers each movers panel lists.
const MOVERS_LIMIT: usize = 8;

/// How many stocks each side of the quality leaderboard (healthiest /
/// most-concerning) lists. Mirrors `MOVERS_LIMIT` so the panels read alike.
const HEALTH_LIMIT: usize = 8;

/// A symbol's latest session counts as the bars within this window of its most
/// recent intraday bar. The regular-plus-extended session spans ~16h, while
/// the prior session's bars sit a full ~24h earlier, so 23h cleanly isolates
/// just the latest day.
const SESSION_WINDOW_MS: i64 = 23 * 3600 * 1000;

/// Calendar days of daily closes to pull for the trajectory read. Comfortably
/// over the ~252 trading days `compute`'s trend window needs.
const TREND_LOOKBACK_DAYS: i64 = 400;

/// One sparkline card on the dashboard's top row.
#[derive(Serialize)]
struct SparkCard {
    ticker: String,
    name: String,
    price: Option<f64>,
    change_pct: Option<f64>,
    /// Sparkline geometry, `None` until the symbol has intraday bars.
    spark: Option<Sparkline>,
    /// Colour hook: true when the day's change is not negative (or unknown).
    up: bool,
}

/// One sparkline section of the dashboard (Indexes, Commodities): its cards and
/// the freshest quote behind them, so the section heading can carry a quiet
/// "prices as of ..." freshness caption (PLAN.md Phase 22).
#[derive(Serialize)]
struct SparkSection {
    cards: Vec<SparkCard>,
    /// The most recent `last_quote_at` (epoch-ms) across the section's symbols;
    /// `None` until at least one has ever been quoted.
    asof: Option<i64>,
}

/// The dashboard hero (Phase 5): a one-line plain-language read of the day
/// blending the broad index move, market breadth, and the VIX risk tone, with
/// the headline figures behind it and a compact index strip. The verdict is a
/// descriptive read of today's tape, not a forecast — the non-advice note rides
/// with it.
#[derive(Serialize)]
struct Hero {
    /// The punchy lead, e.g. "Risk-on, and broad."
    verdict: String,
    /// The supporting clause, e.g. "Markets higher with wide participation."
    detail: String,
    /// Compact index chips beside the verdict (the resolved index cards' moves).
    chips: Vec<HeroChip>,
    /// The broad-market day change (the lead index card's move); drives the
    /// "S&P +0.9%" stat and feeds the verdict. `None` until that card prices.
    broad_pct: Option<f64>,
    /// Share of curated stocks trading green, 0..100, rounded; "78% green".
    green_pct: Option<u8>,
    /// The VIX read folded into one phrase, e.g. "calm at 13.2". `None` until
    /// the VIX card prices.
    vix_label: Option<String>,
}

/// One index chip in the hero strip: the resolved index card's ticker and day
/// move (cash index during the regular session, its future outside it).
#[derive(Serialize)]
struct HeroChip {
    ticker: String,
    change_pct: Option<f64>,
    up: bool,
}

/// Market breadth across the curated large-cap stocks (Phase 5): how many are
/// advancing vs declining today and the share green, plus the proportion-bar
/// segment widths. Drives the Breadth band and feeds the hero verdict.
#[derive(Serialize, Default)]
struct Breadth {
    advancers: usize,
    decliners: usize,
    unchanged: usize,
    /// Stocks with a computable day change (advancers + decliners + unchanged).
    total: usize,
    /// Advancers as a percent of `total`, rounded; `None` when `total` is 0.
    pct_green: Option<u8>,
    /// Proportion-bar segment widths (percent of `total`): green, flat, red.
    up_w: f64,
    flat_w: f64,
    down_w: f64,
}

/// One curated ETF with everything the dashboard ETF band needs: its price, the
/// close it is changing against, and its Phase-4 quality read. Built once and
/// fed to both the curated cards and the movers strip.
#[derive(Serialize, Clone)]
struct EtfRow {
    ticker: String,
    name: String,
    last: Option<f64>,
    prev: Option<f64>,
    change_pct: Option<f64>,
    last_quote_at: Option<i64>,
    quality: Option<compute::EtfQuality>,
}

/// One curated ETF card: its sparkline tile plus the quality verdict (Phase 5).
#[derive(Serialize)]
struct EtfCard {
    card: SparkCard,
    quality: Option<compute::EtfQuality>,
}

/// One pill in the compact ETF movers strip: ticker, day move, quality grade.
#[derive(Serialize, Clone)]
struct EtfMover {
    ticker: String,
    change_pct: f64,
    quality: Option<compute::EtfQuality>,
}

/// One row in a movers panel.
#[derive(Serialize, Clone)]
struct Mover {
    ticker: String,
    name: String,
    price: f64,
    change_abs: f64,
    change_pct: f64,
    /// Width (0..100) of the row's magnitude tint, scaled to the largest
    /// absolute move shown across both panels.
    bar: f64,
    /// The stock's rolled-up strong / fair / weak badge (Phase 20); `None`
    /// until its SEC fundamentals have synced.
    strength: Option<compute::Standing>,
}

/// One row in the quality leaderboard (healthiest / most-concerning panels,
/// Phase 17 / reframed in Phase 3). Carries the `HealthRead` directly so the
/// row can show the overall verdict alongside the three sub-readings, plus the
/// trailing-year return as a quiet price-performance anchor (folded in from the
/// old strongest / weakest panel when those merged into the leaderboard).
#[derive(Serialize, Clone)]
struct HealthRow {
    ticker: String,
    name: String,
    health: compute::HealthRead,
    /// Trailing 12-month return, percent; `None` when history is too short.
    ret_12m: Option<f64>,
    /// Width (0..100) of the row's magnitude tint, scaled to the largest
    /// absolute score shown across both panels.
    bar: f64,
    /// Colour hook: true when the composite score is not negative.
    up: bool,
}

/// One curated large-cap stock with everything the home panels need: its
/// price, the close it is changing against, its rolled-up standing, its
/// trailing-year return, and the health read (Phase 17). Built once per
/// render and fed to the movers, strongest / weakest, and health panels.
struct StockRow {
    ticker: String,
    name: String,
    last: Option<f64>,
    prev: Option<f64>,
    standing: Option<compute::Standing>,
    health: Option<compute::HealthRead>,
    ret_12m: Option<f64>,
    /// When this stock was last quoted (epoch-ms); feeds the movers panel's
    /// freshness caption (PLAN.md Phase 22).
    last_quote_at: Option<i64>,
    /// When this stock's SEC fundamentals last synced (epoch-ms); feeds the
    /// strongest / weakest panels' freshness caption.
    fundamentals_synced_at: Option<i64>,
    /// When this stock's SEC leadership roster last synced (epoch-ms); feeds
    /// the health panels' freshness caption.
    leadership_synced_at: Option<i64>,
    /// Yahoo `assetProfile` sector (Phase 15). `None` until the asset_profile
    /// sweep has reached this stock; the sector panels drop those rows.
    sector: Option<String>,
    /// When the asset_profile sweep last touched this stock (Phase 15).
    asset_profile_synced_at: Option<i64>,
}

/// One row in the home page's "Today's industries" panel: a sector with its
/// composite day move, member count, and the magnitude tint sized against
/// the largest absolute composite shown across the top / bottom rows.
#[derive(Serialize, Clone)]
struct IndustryRow {
    name: String,
    slug: String,
    members: usize,
    change_pct: f64,
    bar: f64,
    up: bool,
}

/// How many sectors to show in each of the top / bottom industry panels.
const INDUSTRY_LIMIT: usize = 3;

async fn home(State(state): State<AppState>) -> Response {
    let seeded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM symbols WHERE is_seeded = 1")
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);

    // No curated universe yet: the seed has not run. Show the same guidance
    // the page carried before the redesign.
    if seeded == 0 {
        let extra = minijinja::context! { title => "Markets", empty => true };
        return render(&state, "pages/home.html", "/", extra);
    }

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM symbols")
        .fetch_one(&state.pool)
        .await
        .unwrap_or(seeded);

    let (index_section, commodity_section) = dashboard_cards(&state).await;
    // One scan of the curated stocks feeds breadth, the movers, the industry
    // composites, and the quality leaderboard.
    let stocks = load_stocks(&state).await;
    let breadth = breadth(&stocks);
    // The hero blends the lead index card's move, breadth, and the VIX card into
    // a one-line read; both sections are already loaded, so it is free.
    let hero = build_hero(&index_section, &commodity_section, &breadth);
    // The ETF band: curated quality cards plus a compact gainers / losers strip.
    let (etf_cards, etf_gainers, etf_losers, etf_asof) = etf_band(&state).await;
    let (gainers, losers) = movers(&stocks);
    let (healthiest, concerning) = health_panels(&stocks);
    let (top_industries, bottom_industries) = industry_panels(&stocks);
    let industries_asof = stocks
        .iter()
        .filter_map(|s| s.asset_profile_synced_at)
        .max();

    // Section freshness (PLAN.md Phase 22): the movers panels date off the
    // freshest stock quote — the data each panel actually leans on.
    let movers_asof = stocks.iter().filter_map(|s| s.last_quote_at).max();
    // The quality leaderboard leans on both fundamentals and leadership, so its
    // freshness caption tracks whichever sync ran later.
    let health_asof = stocks
        .iter()
        .filter_map(|s| {
            [s.fundamentals_synced_at, s.leadership_synced_at]
                .iter()
                .filter_map(|x| *x)
                .max()
        })
        .max();

    let extra = minijinja::context! {
        title => "Markets",
        empty => false,
        hero => hero,
        breadth => breadth,
        etf_cards => etf_cards,
        etf_gainers => etf_gainers,
        etf_losers => etf_losers,
        etf_asof => etf_asof,
        index_cards => index_section.cards,
        index_asof => index_section.asof,
        commodity_cards => commodity_section.cards,
        commodity_asof => commodity_section.asof,
        gainers => gainers,
        losers => losers,
        movers_asof => movers_asof,
        healthiest => healthiest,
        concerning => concerning,
        health_asof => health_asof,
        top_industries => top_industries,
        bottom_industries => bottom_industries,
        industries_asof => industries_asof,
        total => total,
    };
    render(&state, "pages/home.html", "/", extra)
}

/// The dashboard's index and commodity sparkline cards.
///
/// Outside the regular cash session each index card resolves to its index
/// future (see `INDEXES`): the future trades nearly around the clock, so the
/// card stays live overnight instead of freezing on the 16:00 ET close.
async fn dashboard_cards(state: &AppState) -> (SparkSection, SparkSection) {
    let regular = matches!(
        market::session_at(chrono::Utc::now()),
        market::Session::Regular
    );
    // During the regular cash session show each index itself; outside it,
    // swap in the index future where one exists.
    let index_tickers: Vec<&str> = INDEXES
        .iter()
        .map(|&(index, future)| match future {
            Some(fut) if !regular => fut,
            _ => index,
        })
        .collect();
    let indexes = spark_cards_for(state, &index_tickers).await;
    let commodities = spark_cards_for(state, COMMODITIES).await;
    (indexes, commodities)
}

/// Build a sparkline section for `tickers`, in that order: a card per ticker
/// (current price, the day's change, a sparkline of the latest session's bars)
/// plus the freshest quote across them. A ticker the universe does not hold is
/// skipped.
async fn spark_cards_for(state: &AppState, tickers: &[&str]) -> SparkSection {
    if tickers.is_empty() {
        return SparkSection {
            cards: Vec::new(),
            asof: None,
        };
    }
    // One query for the price rows. The `IN` list is built from the hardcoded
    // dashboard consts — never user input — so the placeholder count is fixed
    // and safe. The trailing `last_quote_at` feeds the section's freshness.
    type SparkRow = (String, String, String, Option<f64>, Option<f64>, Option<i64>);
    let placeholders = vec!["?"; tickers.len()].join(",");
    let sql = format!(
        "SELECT s.ticker, s.name, s.kind, \
           COALESCE(s.last_price, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1)), \
           COALESCE(s.prev_close, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1 OFFSET 1)), \
           s.last_quote_at \
         FROM symbols s WHERE s.ticker IN ({placeholders})"
    );
    let mut q = sqlx::query_as::<_, SparkRow>(&sql);
    for t in tickers {
        q = q.bind(*t);
    }
    let rows: Vec<SparkRow> = q.fetch_all(&state.pool).await.unwrap_or_default();
    let asof = rows.iter().filter_map(|r| r.5).max();
    let mut by_ticker: HashMap<String, SparkRow> =
        rows.into_iter().map(|r| (r.0.clone(), r)).collect();

    let mut cards = Vec::with_capacity(tickers.len());
    for &t in tickers {
        // Skip a dashboard symbol the universe somehow does not hold.
        let Some((ticker, name, _kind, last, prev, _quote_at)) = by_ticker.remove(t) else {
            continue;
        };

        // The latest session's intraday closes, oldest first. The window keys
        // off this symbol's own most recent bar (see SESSION_WINDOW_MS).
        let closes: Vec<f64> = sqlx::query_scalar(
            "SELECT close FROM intraday_bars \
             WHERE ticker = ? \
               AND ts >= (SELECT MAX(ts) FROM intraday_bars WHERE ticker = ?) - ? \
             ORDER BY ts",
        )
        .bind(&ticker)
        .bind(&ticker)
        .bind(SESSION_WINDOW_MS)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

        let change_pct = match (last, prev) {
            (Some(l), Some(p)) => Some(compute::change(l, p).pct),
            _ => None,
        };
        cards.push(SparkCard {
            ticker,
            name,
            price: last,
            change_pct,
            spark: compute::sparkline(&closes, prev),
            up: change_pct.map_or(true, |p| p >= 0.0),
        });
    }
    SparkSection { cards, asof }
}

/// Market breadth across the curated large-cap stocks: advancers vs decliners
/// today and the share green, plus the proportion-bar segment widths. Reuses
/// the already-loaded [`StockRow`] scan — no extra query. A stock without a
/// computable change (no price, or a non-positive prior close) is left out of
/// every count so a missing quote never reads as "flat".
fn breadth(stocks: &[StockRow]) -> Breadth {
    let mut b = Breadth::default();
    for s in stocks {
        let (Some(last), Some(prev)) = (s.last, s.prev) else {
            continue;
        };
        if prev <= 0.0 {
            continue;
        }
        b.total += 1;
        match last.partial_cmp(&prev) {
            Some(Ordering::Greater) => b.advancers += 1,
            Some(Ordering::Less) => b.decliners += 1,
            _ => b.unchanged += 1,
        }
    }
    if b.total > 0 {
        let total = b.total as f64;
        b.pct_green = Some((b.advancers as f64 / total * 100.0).round() as u8);
        b.up_w = b.advancers as f64 / total * 100.0;
        b.down_w = b.decliners as f64 / total * 100.0;
        b.flat_w = (100.0 - b.up_w - b.down_w).max(0.0);
    }
    b
}

/// A VIX level read into one plain word. The bands suit the ^VIX cash gauge:
/// sub-14 is a placid tape, the teens are normal, the low-20s start to show
/// stress, and 28+ is outright fear.
fn vix_tone(level: f64) -> &'static str {
    match level {
        v if v < 14.0 => "calm",
        v if v < 20.0 => "steady",
        v if v < 28.0 => "elevated",
        _ => "stressed",
    }
}

/// Blend the broad-market move, breadth, and the VIX read into the hero's
/// two-line verdict. `broad_pct` is the lead index card's day change (the cash
/// S&P during the regular session, its future outside it); `green_pct` the
/// share of curated stocks green; `vix_level` / `vix_pct` the volatility gauge
/// and its move. Returns `(lead, detail)`. A descriptive read of the tape, not
/// a forecast — direction comes from the broad move, falling back to breadth
/// when the index is flat; width and risk tone colour the wording.
fn market_verdict(
    broad_pct: Option<f64>,
    green_pct: Option<u8>,
    vix_level: Option<f64>,
    vix_pct: Option<f64>,
) -> (String, String) {
    // Direction: the broad index move is the headline truth, so the verdict's
    // direction tracks its sign — never the opposite of the "S&P +x%" figure
    // shown beside it. A near-flat index (|move| < 0.05%) reads as mixed even if
    // breadth skews, since that is genuinely a directionless tape. Breadth only
    // sets direction when there is no index price at all (e.g. it never quoted).
    // 1 up / -1 down / 0 flat.
    let dir = match broad_pct {
        Some(p) if p > 0.05 => 1,
        Some(p) if p < -0.05 => -1,
        Some(_) => 0,
        None => match green_pct {
            Some(g) if g >= 55 => 1,
            Some(g) if g <= 45 => -1,
            _ => 0,
        },
    };
    // Breadth width: 2 broad / 1 split / 0 narrow.
    let width = match green_pct {
        Some(g) if g >= 60 => 2,
        Some(g) if g <= 40 => 0,
        _ => 1,
    };
    let vix_rising = vix_pct.is_some_and(|p| p > 4.0);
    let vix_elevated = vix_level.is_some_and(|v| v >= 20.0);

    let lead = match (dir, width) {
        (1, 2) if !vix_elevated => "Risk-on, and broad.",
        (1, 2) => "Higher across the board.",
        (1, 0) => "Higher, but narrow.",
        (1, _) => "Modestly higher.",
        (-1, _) if vix_rising || vix_elevated => "Risk-off.",
        (-1, 0) => "Broadly lower.",
        (-1, _) => "Softer today.",
        _ => "Quiet, mixed tape.",
    };
    let move_word = match dir {
        1 => "higher",
        -1 => "lower",
        _ => "little changed",
    };
    let part_word = match width {
        2 => "wide participation",
        0 => "narrow participation",
        _ => "mixed participation",
    };
    (lead.to_string(), format!("Markets {move_word} with {part_word}."))
}

/// Assemble the hero from the already-loaded index + commodity cards and the
/// breadth read. The broad-market move is the lead index card (resolved to the
/// cash S&P or its future by `dashboard_cards`); the VIX read is the lead
/// commodity card (`^VIX` always sits first). The chips mirror the index cards.
fn build_hero(index: &SparkSection, commodity: &SparkSection, breadth: &Breadth) -> Hero {
    let broad_pct = index.cards.first().and_then(|c| c.change_pct);
    let vix = commodity.cards.first();
    let vix_level = vix.and_then(|c| c.price);
    let vix_pct = vix.and_then(|c| c.change_pct);
    let green_pct = breadth.pct_green;

    let (verdict, detail) = market_verdict(broad_pct, green_pct, vix_level, vix_pct);
    let vix_label = vix_level.map(|v| format!("{} at {:.1}", vix_tone(v), v));
    let chips = index
        .cards
        .iter()
        .map(|c| HeroChip {
            ticker: c.ticker.clone(),
            change_pct: c.change_pct,
            up: c.up,
        })
        .collect();

    Hero {
        verdict,
        detail,
        chips,
        broad_pct,
        green_pct,
        vix_label,
    }
}

/// The dashboard ETF band: the curated quality cards and a compact gainers /
/// losers strip, plus the freshest quote behind them. Builds the full curated
/// ETF read once ([`load_etfs`]); the cards reuse the sparkline query for their
/// intraday tiles and attach each ETF's quality, and the strip ranks the whole
/// curated ETF set by day move.
async fn etf_band(state: &AppState) -> (Vec<EtfCard>, Vec<EtfMover>, Vec<EtfMover>, Option<i64>) {
    let etfs = load_etfs(state).await;
    let quality_by: HashMap<&str, compute::EtfQuality> = etfs
        .iter()
        .filter_map(|e| e.quality.map(|q| (e.ticker.as_str(), q)))
        .collect();

    // Curated cards: sparkline tiles for the hardcoded set, each carrying its
    // quality verdict. `spark_cards_for` skips a ticker the universe lacks.
    let spark = spark_cards_for(state, ETF_CARDS).await;
    let etf_cards: Vec<EtfCard> = spark
        .cards
        .into_iter()
        .map(|card| {
            let quality = quality_by.get(card.ticker.as_str()).copied();
            EtfCard { card, quality }
        })
        .collect();

    let (gainers, losers) = etf_movers(&etfs);
    // Freshest quote across the band: the cards' section asof, else the widest
    // quote across the curated ETF set.
    let etf_asof = spark
        .asof
        .or_else(|| etfs.iter().filter_map(|e| e.last_quote_at).max());
    (etf_cards, gainers, losers, etf_asof)
}

/// Every curated ETF, each rolled into an [`EtfRow`] with its Phase-4 quality
/// read. Mirrors `load_stocks` but for funds: one query for price + the Yahoo
/// metadata (expense ratio, NAV) + the SEC AUM, a second for top-10 holdings
/// concentration, then `compute::etf_quality`. The tracking factor reads a
/// price-vs-NAV premium only against a *fresh* NAV (the daily `fund_nav` job),
/// else it drops out — a stale NAV must never assert a bogus premium.
async fn load_etfs(state: &AppState) -> Vec<EtfRow> {
    type EtfPriceRow = (
        String,         // ticker
        String,         // name
        Option<f64>,    // last
        Option<f64>,    // prev
        Option<i64>,    // last_quote_at
        Option<f64>,    // expense_ratio
        Option<f64>,    // nav_price
        Option<i64>,    // nav_synced_at
        Option<f64>,    // net_assets
    );
    let rows: Vec<EtfPriceRow> = sqlx::query_as(
        "SELECT s.ticker, s.name, \
           COALESCE(s.last_price, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1)), \
           COALESCE(s.prev_close, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1 OFFSET 1)), \
           s.last_quote_at, m.expense_ratio, m.nav_price, m.nav_synced_at, fp.net_assets \
         FROM symbols s \
         LEFT JOIN fund_metadata m ON m.ticker = s.ticker \
         LEFT JOIN fund_profiles fp ON fp.ticker = s.ticker \
         WHERE s.is_seeded = 1 AND s.kind = 'etf'",
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    if rows.is_empty() {
        return Vec::new();
    }

    // Top-10 holdings concentration per ETF (summed weight of the ten largest
    // holdings, percent) for the diversification factor. A fund with no
    // holdings (a commodity trust) simply misses the map, so that factor drops
    // out of its blend rather than reading as zero.
    let top10_rows: Vec<(String, Option<f64>)> = sqlx::query_as(
        "SELECT ticker, SUM(pct) FROM ( \
           SELECT ticker, pct, ROW_NUMBER() OVER (PARTITION BY ticker ORDER BY rank) AS rn \
           FROM fund_holdings \
         ) WHERE rn <= 10 AND pct IS NOT NULL GROUP BY ticker",
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    let top10_by: HashMap<String, f64> = top10_rows
        .into_iter()
        .filter_map(|(t, p)| p.filter(|v| *v > 0.0).map(|v| (t, v)))
        .collect();

    let now = crate::db::now_ms();
    rows.into_iter()
        .map(
            |(ticker, name, last, prev, last_quote_at, expense_ratio, nav_price, nav_synced_at, net_assets)| {
                let change_pct = match (last, prev) {
                    (Some(l), Some(p)) if p > 0.0 => Some(compute::change(l, p).pct),
                    _ => None,
                };
                // Tracking factor: read the price-vs-NAV premium only against a
                // NAV synced within NAV_FRESH_MS; else let the factor drop out.
                let nav_fresh = nav_synced_at.is_some_and(|t| now - t <= NAV_FRESH_MS);
                let premium_pct = if nav_fresh {
                    last.and_then(|p| compute::premium_discount_pct(p, nav_price))
                } else {
                    None
                };
                let top10_pct = top10_by.get(&ticker).copied();
                let quality =
                    compute::etf_quality(expense_ratio, premium_pct, top10_pct, net_assets);
                EtfRow {
                    ticker,
                    name,
                    last,
                    prev,
                    change_pct,
                    last_quote_at,
                    quality,
                }
            },
        )
        .collect()
}

/// The day's biggest ETF gainers and losers among the curated funds, each
/// carrying its quality grade for a quiet chip. A compact strip (top
/// [`ETF_MOVERS_LIMIT`] each), so the band stays scannable beneath the cards.
fn etf_movers(etfs: &[EtfRow]) -> (Vec<EtfMover>, Vec<EtfMover>) {
    let mut all: Vec<EtfMover> = etfs
        .iter()
        .filter_map(|e| {
            Some(EtfMover {
                ticker: e.ticker.clone(),
                change_pct: e.change_pct?,
                quality: e.quality,
            })
        })
        .collect();
    if all.is_empty() {
        return (Vec::new(), Vec::new());
    }
    all.sort_by(|a, b| {
        b.change_pct
            .partial_cmp(&a.change_pct)
            .unwrap_or(Ordering::Equal)
    });
    let gainers: Vec<EtfMover> = all.iter().take(ETF_MOVERS_LIMIT).cloned().collect();
    let losers: Vec<EtfMover> = all.iter().rev().take(ETF_MOVERS_LIMIT).cloned().collect();
    (gainers, losers)
}

/// Every curated large-cap stock, each graded into a [`StockRow`].
///
/// Restricted to `is_seeded` stocks on purpose (see PLAN.md Phase 11): the
/// home panels are meant to show names worth noticing, not a small user-added
/// symbol's noise. Three queries — price, fundamentals, trailing-year closes —
/// then each stock is graded in `compute`. ETFs, indexes and futures are
/// excluded: only single stocks have the SEC fundamentals a standing needs.
async fn load_stocks(state: &AppState) -> Vec<StockRow> {
    // 1. Price per curated stock: the live last price, else the latest daily
    //    close; plus the prior close it is changing against. Also pulls each
    //    stock's fundamentals and leadership sync times (for the panels'
    //    freshness captions, Phase 22 + Phase 17) and the Phase 15 sector +
    //    asset-profile sync time (for the Today's industries panel).
    type PriceRow = (
        String,
        String,
        Option<f64>,
        Option<f64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<String>,
        Option<i64>,
    );
    let price_rows: Vec<PriceRow> = sqlx::query_as(
        "SELECT s.ticker, s.name, \
           COALESCE(s.last_price, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1)), \
           COALESCE(s.prev_close, \
             (SELECT close FROM daily_prices p WHERE p.ticker = s.ticker ORDER BY d DESC LIMIT 1 OFFSET 1)), \
           s.last_quote_at, s.fundamentals_synced_at, s.leadership_synced_at, \
           s.sector, s.asset_profile_synced_at \
         FROM symbols s WHERE s.is_seeded = 1 AND s.kind = 'stock'",
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    if price_rows.is_empty() {
        return Vec::new();
    }

    // 2. Every stored fundamentals fact for the curated stocks, grouped by
    //    ticker — the basis for each stock's graded ratios.
    let fact_rows: Vec<(String, String, String, i64, Option<i64>, f64, String)> = sqlx::query_as(
        "SELECT f.ticker, f.metric, f.period, f.fiscal_year, f.fiscal_qtr, f.value, f.period_end \
         FROM fundamentals f JOIN symbols s ON s.ticker = f.ticker \
         WHERE s.is_seeded = 1 AND s.kind = 'stock'",
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    let mut facts: HashMap<String, Vec<models::FundFact>> = HashMap::new();
    for (ticker, metric, period, fiscal_year, fiscal_qtr, value, period_end) in fact_rows {
        facts.entry(ticker).or_default().push(models::FundFact {
            metric,
            period,
            fiscal_year,
            fiscal_qtr,
            value,
            period_end,
        });
    }

    // 3. The trailing-year daily closes per curated stock, oldest first.
    let cutoff = (chrono::Utc::now().date_naive() - chrono::Duration::days(TREND_LOOKBACK_DAYS))
        .to_string();
    let close_rows: Vec<(String, f64)> = sqlx::query_as(
        "SELECT p.ticker, p.close FROM daily_prices p JOIN symbols s ON s.ticker = p.ticker \
         WHERE s.is_seeded = 1 AND s.kind = 'stock' AND p.d >= ? ORDER BY p.ticker, p.d",
    )
    .bind(&cutoff)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    let mut closes: HashMap<String, Vec<f64>> = HashMap::new();
    for (ticker, close) in close_rows {
        closes.entry(ticker).or_default().push(close);
    }

    // 4. Recent 8-K item-5.02 leadership-change count per curated stock,
    //    inside the Phase 17 stability window. One bulk query: the GROUP BY
    //    returns only stocks with at least one match, so the lookup misses
    //    cleanly for a fully-stable company (read below as 0).
    let lead_cutoff = (chrono::Utc::now().date_naive()
        - chrono::Duration::days(compute::LEADERSHIP_STABILITY_DAYS))
    .to_string();
    let change_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT f.ticker, COUNT(*) FROM filings f JOIN symbols s ON s.ticker = f.ticker \
         WHERE s.is_seeded = 1 AND s.kind = 'stock' \
           AND f.form LIKE '8-K%' AND f.items LIKE '%5.02%' \
           AND f.filed_at >= ? \
         GROUP BY f.ticker",
    )
    .bind(&lead_cutoff)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    let lead_counts: HashMap<String, usize> = change_rows
        .into_iter()
        .map(|(t, n)| (t, n.max(0) as usize))
        .collect();

    // Assemble: grade each stock off its facts and price, read its trajectory
    // off its closes, and read its leadership stability off the change count.
    // A stock with no fundamentals stored yet simply has no standing or health
    // and is left out of the strongest / weakest and healthiest rankings.
    price_rows
        .into_iter()
        .map(
            |(
                ticker,
                name,
                last,
                prev,
                last_quote_at,
                fundamentals_synced_at,
                leadership_synced_at,
                sector,
                asset_profile_synced_at,
            )| {
                let stock_closes = closes.get(&ticker).map(Vec::as_slice).unwrap_or(&[]);
                let ratios = facts.get(&ticker).and_then(|f| {
                    let inputs = models::latest_annual_inputs(f, last)?;
                    Some(compute::compute_ratios(&inputs))
                });
                let standing = ratios
                    .as_ref()
                    .and_then(|r| compute::standing(r, stock_closes));
                // Leadership-change count is `None` until the leadership sweep
                // has reached this stock; the composite then drops that
                // component instead of penalising an unsynced company.
                let recent_changes = leadership_synced_at
                    .map(|_| lead_counts.get(&ticker).copied().unwrap_or(0));
                let health = ratios
                    .as_ref()
                    .and_then(|r| compute::health_read(r, stock_closes, recent_changes));
                StockRow {
                    ticker,
                    name,
                    last,
                    prev,
                    standing,
                    health,
                    ret_12m: compute::trailing_return(stock_closes),
                    last_quote_at,
                    fundamentals_synced_at,
                    leadership_synced_at,
                    sector: sector.filter(|s| !s.is_empty()),
                    asset_profile_synced_at,
                }
            },
        )
        .collect()
}

/// Today's biggest sector composites (top / bottom equal-weight day moves)
/// for the home page. Curated `is_seeded` stocks only — same universe as
/// movers / strongest — so a small user-added stock's noise does not crowd
/// out a recognised sector. Sectors only (industry-level moves are noisier
/// and live on `/industries`).
fn industry_panels(stocks: &[StockRow]) -> (Vec<IndustryRow>, Vec<IndustryRow>) {
    // Bucket each stock's day move by sector. Drop a stock without both a
    // sector classification and a computable change.
    let mut by_sector: HashMap<String, Vec<f64>> = HashMap::new();
    for s in stocks {
        let Some(sector) = s.sector.as_deref() else {
            continue;
        };
        let (Some(last), Some(prev)) = (s.last, s.prev) else {
            continue;
        };
        if prev <= 0.0 {
            continue;
        }
        by_sector
            .entry(sector.to_string())
            .or_default()
            .push((last - prev) / prev * 100.0);
    }
    if by_sector.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let mut rows: Vec<IndustryRow> = by_sector
        .into_iter()
        .map(|(name, pcts)| {
            let avg = pcts.iter().sum::<f64>() / pcts.len() as f64;
            IndustryRow {
                slug: crate::routes::industries::slug(&name),
                name,
                members: pcts.len(),
                change_pct: avg,
                bar: 0.0,
                up: avg >= 0.0,
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        b.change_pct
            .partial_cmp(&a.change_pct)
            .unwrap_or(Ordering::Equal)
    });
    let mut top: Vec<IndustryRow> = rows.iter().take(INDUSTRY_LIMIT).cloned().collect();
    let mut bottom: Vec<IndustryRow> = rows.iter().rev().take(INDUSTRY_LIMIT).cloned().collect();

    // Scale every magnitude tint to the largest absolute move shown across
    // both panels (mirrors movers / strongest / health).
    let max_abs = top
        .iter()
        .chain(bottom.iter())
        .map(|r| r.change_pct.abs())
        .fold(0.0_f64, f64::max);
    for r in top.iter_mut().chain(bottom.iter_mut()) {
        r.bar = if max_abs > 0.0 {
            (r.change_pct.abs() / max_abs * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
    }
    (top, bottom)
}

/// The day's biggest gainers and losers among the curated large-cap stocks.
/// Each row also carries the stock's strong / fair / weak badge (Phase 20).
fn movers(stocks: &[StockRow]) -> (Vec<Mover>, Vec<Mover>) {
    // Keep only stocks with a computable change.
    let mut all: Vec<Mover> = stocks
        .iter()
        .filter_map(|s| {
            let (last, prev) = (s.last?, s.prev?);
            if prev == 0.0 {
                return None;
            }
            let c = compute::change(last, prev);
            Some(Mover {
                ticker: s.ticker.clone(),
                name: s.name.clone(),
                price: last,
                change_abs: c.abs,
                change_pct: c.pct,
                bar: 0.0,
                strength: s.standing,
            })
        })
        .collect();
    if all.is_empty() {
        return (Vec::new(), Vec::new());
    }

    // Sorted by the day's % change: gainers from the top, losers from the
    // bottom (most negative first).
    all.sort_by(|a, b| {
        b.change_pct
            .partial_cmp(&a.change_pct)
            .unwrap_or(Ordering::Equal)
    });
    let mut gainers: Vec<Mover> = all.iter().take(MOVERS_LIMIT).cloned().collect();
    let mut losers: Vec<Mover> = all.iter().rev().take(MOVERS_LIMIT).cloned().collect();

    // Scale every magnitude tint to the largest absolute move on display, so a
    // +1% and a -1% row read the same width.
    let max_abs = gainers
        .iter()
        .chain(losers.iter())
        .map(|m| m.change_pct.abs())
        .fold(0.0_f64, f64::max);
    for m in gainers.iter_mut().chain(losers.iter_mut()) {
        m.bar = if max_abs > 0.0 {
            (m.change_pct.abs() / max_abs * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
    }
    (gainers, losers)
}

/// The quality leaderboard: the healthiest and most-concerning curated stocks
/// by their Phase 17 composite — fundamentals + recent trajectory + leadership
/// stability rolled into one read. The single non-advice "best / worst quality
/// right now" surface that replaced the old Top picks, Strongest & weakest, and
/// Stock health panels (Phase 3). Each row also carries its trailing-year
/// return as a price-performance anchor. Stocks without a health read
/// (fundamentals not synced) are left out; the panels are a fixed page-load
/// snapshot.
fn health_panels(stocks: &[StockRow]) -> (Vec<HealthRow>, Vec<HealthRow>) {
    let mut ranked: Vec<&StockRow> = stocks.iter().filter(|s| s.health.is_some()).collect();
    if ranked.is_empty() {
        return (Vec::new(), Vec::new());
    }
    ranked.sort_by(|a, b| {
        let (sa, sb) = (a.health.unwrap().score, b.health.unwrap().score);
        sb.partial_cmp(&sa).unwrap_or(Ordering::Equal)
    });

    let row = |s: &StockRow| {
        let health = s.health.unwrap();
        HealthRow {
            ticker: s.ticker.clone(),
            name: s.name.clone(),
            health,
            ret_12m: s.ret_12m,
            bar: 0.0,
            up: health.score >= 0.0,
        }
    };
    let mut healthiest: Vec<HealthRow> =
        ranked.iter().copied().take(HEALTH_LIMIT).map(&row).collect();
    let mut concerning: Vec<HealthRow> = ranked
        .iter()
        .copied()
        .rev()
        .take(HEALTH_LIMIT)
        .map(&row)
        .collect();

    let max_abs = healthiest
        .iter()
        .chain(concerning.iter())
        .map(|r| r.health.score.abs())
        .fold(0.0_f64, f64::max);
    for r in healthiest.iter_mut().chain(concerning.iter_mut()) {
        r.bar = if max_abs > 0.0 {
            (r.health.score.abs() / max_abs * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
    }
    (healthiest, concerning)
}
