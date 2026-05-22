-- finance migration 0005: ETF fund profiles (Phase 18).
--
-- An ETF files with the SEC as a registered fund, not an operating company,
-- so its portfolio comes from quarterly N-PORT filings rather than the XBRL
-- companyfacts that back the stock fundamentals. These two tables hold the
-- parsed snapshot of an ETF's latest N-PORT; a physical-commodity grantor
-- trust (GLD, SLV) files 10-Ks instead and gets the degenerate `kind` below.

-- The fund's SEC series id, e.g. S000002839. One registrant CIK can host many
-- fund series (the Vanguard and iShares trusts host dozens), so the series id
-- is what pins an N-PORT lookup to a single ETF. NULL for a single-fund trust
-- (SPY, DIA) and for every non-ETF symbol.
ALTER TABLE symbols ADD COLUMN series_id TEXT;
-- When this ETF's fund profile was last refreshed from SEC. NULL = never.
ALTER TABLE symbols ADD COLUMN fund_synced_at INTEGER;

-- One profile row per ETF: the headline figures from its latest N-PORT, or,
-- for a commodity trust, the AUM from its 10-K companyfacts.
CREATE TABLE fund_profiles (
    ticker         TEXT PRIMARY KEY REFERENCES symbols(ticker) ON DELETE CASCADE,
    kind           TEXT NOT NULL,      -- 'portfolio' | 'commodity_trust'
    net_assets     REAL,               -- total net assets (AUM), USD
    total_assets   REAL,               -- gross assets, USD
    holdings_count INTEGER,            -- positions in the full portfolio
    report_date    TEXT,               -- N-PORT "as of" date, YYYY-MM-DD
    asset_mix      TEXT,               -- JSON [[bucket, percent], ...]; portfolio funds only
    updated_at     INTEGER NOT NULL
);

-- The largest holdings of a portfolio fund, ranked by weight. Only the top
-- slice is kept (a bond aggregate fund holds thousands of positions); rank 1
-- is the largest. Replaced wholesale on each refresh.
CREATE TABLE fund_holdings (
    ticker    TEXT NOT NULL REFERENCES symbols(ticker) ON DELETE CASCADE,
    rank      INTEGER NOT NULL,        -- 1 = largest weight
    name      TEXT NOT NULL,
    pct       REAL,                    -- percent of net assets, e.g. 8.4
    value_usd REAL,
    PRIMARY KEY (ticker, rank)
);
