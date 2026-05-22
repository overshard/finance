-- finance migration 0008: ETF stats (Phase 28).
--
-- Layers Yahoo `quoteSummary` fund metadata onto the Phase 18 SEC N-PORT
-- fund profile so an ETF symbol page can read as densely as a stock's. Also
-- adds two N-PORT-derived aggregations (sector_mix, geography_mix) that
-- ride alongside the existing asset_mix on `fund_profiles`, and a curated
-- benchmark column on `symbols` for the relative-performance overlay.
--
-- Phase 26's stocks-only dividends path is lifted to ETFs in the same
-- phase, but that is a code-level filter change and needs no migration:
-- the `dividends` table already keys on (ticker, ex_date) for any ticker.

-- When the Yahoo `fund_metadata` scheduler section last refreshed this
-- ETF. NULL = never swept. ETFs only; stocks, indexes and futures keep
-- NULL forever.
ALTER TABLE symbols ADD COLUMN fund_metadata_synced_at INTEGER;

-- Curated benchmark index (a symbol such as `^SPX` / `^IXIC` / `^DJI` /
-- `^RUT`) that the fund aims to track. Hand-curated in
-- `universe/starter.csv` for the broad-market ETFs; a user-added ETF
-- simply omits it and the symbol page hides the relative-performance
-- overlay. NULL on every non-ETF row.
ALTER TABLE symbols ADD COLUMN benchmark TEXT;

-- N-PORT sector + geography exposure aggregated from each holding's
-- `industryCode` / issuer country at parse time. JSON of the same shape
-- as the existing `asset_mix` column: [[bucket, percent], ...] ordered
-- largest first, top ~10 buckets kept. NULL for a commodity-trust ETF
-- (GLD, SLV) that files no N-PORT, the same as `asset_mix` already is.
ALTER TABLE fund_profiles ADD COLUMN sector_mix TEXT;
ALTER TABLE fund_profiles ADD COLUMN geography_mix TEXT;

-- Yahoo `quoteSummary` fund metadata: the slow-moving figures the
-- prospectus carries that N-PORT does not (expense ratio, yield,
-- inception, category, fund family, the issuer's strategy paragraph),
-- plus NAV which drifts intraday. One row per ETF, replaced wholesale by
-- the `fund_metadata` scheduler section on each refresh. Kept separate
-- from `fund_profiles` because the two carry different sources and
-- staleness cadences (SEC quarterly vs Yahoo monthly).
CREATE TABLE fund_metadata (
    ticker             TEXT PRIMARY KEY REFERENCES symbols(ticker) ON DELETE CASCADE,
    expense_ratio      REAL,      -- annualReportExpenseRatio, decimal (e.g. 0.0003 = 0.03%)
    yield_pct          REAL,      -- summaryDetail.yield, decimal (e.g. 0.013 = 1.30%)
    trailing_yield_pct REAL,      -- summaryDetail.trailingAnnualDividendYield, decimal
    nav_price          REAL,      -- price.navPrice or fundProfile fallback, USD
    inception_date     TEXT,      -- YYYY-MM-DD; first trade date / fund start
    category           TEXT,      -- fundProfile.categoryName, e.g. "Large Blend"
    fund_family        TEXT,      -- fundProfile.family, e.g. "Vanguard"
    strategy_summary   TEXT,      -- assetProfile.longBusinessSummary (full paragraph)
    updated_at         INTEGER NOT NULL
);
