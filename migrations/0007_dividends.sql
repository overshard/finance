-- finance migration 0007: dividend payout history (Phase 26).
--
-- A stock's per-event dividend history (ex-dividend date + per-share amount),
-- pulled from Yahoo's chart `events.dividends` series. Used on the symbol page
-- to show the payout cadence, prior-year and YTD totals, and an on-track pace
-- read. Stocks only: ETFs, indexes and futures do not pay regular dividends
-- in this app's sense.

-- When this stock's dividend history was last refreshed from Yahoo. NULL =
-- never. Driven by the new `dividends` scheduler job (weekly cadence).
ALTER TABLE symbols ADD COLUMN dividends_synced_at INTEGER;

-- One row per declared dividend payment. Keyed on (ticker, ex_date): a single
-- payout is uniquely identified by its ex-dividend date — the first trading
-- day on which a new buyer does NOT receive the upcoming payment, which is the
-- date Yahoo's chart events series timestamps each event by.
--
-- Amounts are per-share, in the symbol's reporting currency (USD for the
-- universe we follow). A correction or backfill of a known payout overwrites
-- the prior amount via the primary-key conflict.
CREATE TABLE dividends (
    ticker  TEXT NOT NULL REFERENCES symbols(ticker) ON DELETE CASCADE,
    ex_date TEXT NOT NULL,                  -- ex-dividend date, YYYY-MM-DD
    amount  REAL NOT NULL,                  -- per-share, in symbols.currency
    PRIMARY KEY (ticker, ex_date)
);
CREATE INDEX dividends_ticker_date ON dividends(ticker, ex_date DESC);
