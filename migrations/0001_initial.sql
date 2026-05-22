-- finance: market-watching schema.
-- All *_at columns are UTC epoch-milliseconds (see db::now_ms).
-- Trading dates ("d") are TEXT YYYY-MM-DD: calendar days, not instants.

-- ── symbols: the watched universe (stocks, ETFs, indexes) ──
CREATE TABLE symbols (
    ticker                 TEXT PRIMARY KEY,            -- uppercase, e.g. AAPL, ^SPX
    name                   TEXT NOT NULL DEFAULT '',
    kind                   TEXT NOT NULL DEFAULT 'stock',  -- stock | etf | index
    exchange               TEXT,
    currency               TEXT NOT NULL DEFAULT 'USD',
    cik                    TEXT,                        -- 10-digit zero-padded SEC CIK; NULL for ETF/index
    sector                 TEXT,
    industry               TEXT,
    is_seeded              INTEGER NOT NULL DEFAULT 0,  -- member of the curated starter list
    is_watched             INTEGER NOT NULL DEFAULT 0,  -- in at least one watchlist (denormalized)
    history_synced_at      INTEGER,
    history_first_date     TEXT,
    history_last_date      TEXT,
    fundamentals_synced_at INTEGER,
    filings_synced_at      INTEGER,
    -- denormalized latest snapshot for fast list rendering and SSE seeding
    last_price             REAL,
    prev_close             REAL,
    last_quote_at          INTEGER,
    created_at             INTEGER NOT NULL,
    updated_at             INTEGER NOT NULL
);
CREATE INDEX symbols_kind       ON symbols(kind);
CREATE INDEX symbols_is_watched ON symbols(is_watched);
CREATE INDEX symbols_name       ON symbols(name);

-- ── daily_prices: deep historical OHLCV from Stooq. Permanent, never pruned. ──
CREATE TABLE daily_prices (
    ticker  TEXT NOT NULL REFERENCES symbols(ticker) ON DELETE CASCADE,
    d       TEXT NOT NULL,                              -- YYYY-MM-DD trading date
    open    REAL NOT NULL,
    high    REAL NOT NULL,
    low     REAL NOT NULL,
    close   REAL NOT NULL,
    volume  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (ticker, d)
);
CREATE INDEX daily_prices_d ON daily_prices(d);

-- ── intraday_bars: today's ~15-min-delayed bars from Yahoo. Pruned to recent days. ──
CREATE TABLE intraday_bars (
    ticker  TEXT NOT NULL REFERENCES symbols(ticker) ON DELETE CASCADE,
    ts      INTEGER NOT NULL,                           -- bar start, UTC epoch-ms
    open    REAL NOT NULL,
    high    REAL NOT NULL,
    low     REAL NOT NULL,
    close   REAL NOT NULL,
    volume  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (ticker, ts)
);
CREATE INDEX intraday_bars_ts ON intraday_bars(ts);

-- ── quotes: latest live quote snapshot per symbol (one row per ticker, upserted) ──
CREATE TABLE quotes (
    ticker        TEXT PRIMARY KEY REFERENCES symbols(ticker) ON DELETE CASCADE,
    price         REAL NOT NULL,
    prev_close    REAL,
    open          REAL,
    day_high      REAL,
    day_low       REAL,
    volume        INTEGER,
    market_state  TEXT,                                 -- PRE | REGULAR | POST | CLOSED (source-reported)
    source        TEXT NOT NULL DEFAULT 'yahoo',
    source_time   INTEGER,                              -- the source's own timestamp (epoch-ms)
    fetched_at    INTEGER NOT NULL
);

-- ── fundamentals: one row per (ticker, metric, fiscal period) from SEC XBRL facts ──
-- Long/narrow so new XBRL concepts need no schema change. Stocks only.
CREATE TABLE fundamentals (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    ticker      TEXT NOT NULL REFERENCES symbols(ticker) ON DELETE CASCADE,
    metric      TEXT NOT NULL,   -- revenue | net_income | eps_diluted | shares_diluted
                                 -- | dividends_per_share | assets | liabilities | equity
    period      TEXT NOT NULL,   -- 'FY2024' or 'Q3-2024'
    fiscal_year INTEGER NOT NULL,
    fiscal_qtr  INTEGER,         -- NULL for a full-year figure
    period_end  TEXT NOT NULL,   -- YYYY-MM-DD
    value       REAL NOT NULL,
    unit        TEXT,            -- USD | USD/shares | shares
    form        TEXT,            -- 10-K | 10-Q
    filed_at    TEXT,            -- YYYY-MM-DD
    UNIQUE (ticker, metric, period_end)
);
CREATE INDEX fundamentals_ticker_metric ON fundamentals(ticker, metric, period_end DESC);

-- ── filings: SEC filing history (10-K / 10-Q / 8-K and friends) ──
CREATE TABLE filings (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    ticker           TEXT NOT NULL REFERENCES symbols(ticker) ON DELETE CASCADE,
    accession        TEXT NOT NULL,
    form             TEXT NOT NULL,
    filed_at         TEXT NOT NULL,                     -- YYYY-MM-DD
    period_of_report TEXT,                              -- YYYY-MM-DD
    primary_doc      TEXT,
    url              TEXT NOT NULL,                     -- full EDGAR filing-index URL
    description      TEXT,
    UNIQUE (ticker, accession)
);
CREATE INDEX filings_ticker_filed ON filings(ticker, filed_at DESC);

-- ── watchlists ──
CREATE TABLE watchlists (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL,
    slug       TEXT NOT NULL UNIQUE,                    -- URL-safe, e.g. tech-megacaps
    position   INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE watchlist_items (
    watchlist_id INTEGER NOT NULL REFERENCES watchlists(id) ON DELETE CASCADE,
    ticker       TEXT NOT NULL REFERENCES symbols(ticker) ON DELETE CASCADE,
    position     INTEGER NOT NULL DEFAULT 0,
    added_at     INTEGER NOT NULL,
    PRIMARY KEY (watchlist_id, ticker)
);
CREATE INDEX watchlist_items_ticker ON watchlist_items(ticker);

-- ── fetch_log: append-only history of every background fetch. Drives the data-status UI. ──
CREATE TABLE fetch_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    job         TEXT NOT NULL,    -- seed | history | intraday | fundamentals | filings | prune
    provider    TEXT NOT NULL,    -- stooq | yahoo | sec | -
    ticker      TEXT,             -- NULL for bulk jobs
    status      TEXT NOT NULL,    -- ok | error | skipped
    detail      TEXT,
    rows        INTEGER,
    duration_ms INTEGER,
    started_at  INTEGER NOT NULL,
    finished_at INTEGER NOT NULL
);
CREATE INDEX fetch_log_started ON fetch_log(started_at DESC);
CREATE INDEX fetch_log_job     ON fetch_log(job, started_at DESC);

-- ── data_status: one row per job, current state, for the live status pill ──
CREATE TABLE data_status (
    job           TEXT PRIMARY KEY, -- seed | history | intraday | fundamentals | filings
    state         TEXT NOT NULL,    -- idle | fetching | ok | stale | error
    last_ok_at    INTEGER,
    last_error    TEXT,
    last_error_at INTEGER,
    next_run_at   INTEGER,
    updated_at    INTEGER NOT NULL
);

-- ── meta: one-off key-value settings (seed_completed flag, etc.) ──
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
