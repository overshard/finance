-- finance migration 0011: earnings calendar (Phase 25).
--
-- The next-expected earnings date for each stock, fetched from Yahoo's
-- `quoteSummary.calendarEvents` module. Past earnings dates are not stored
-- here: they ride for free on the 8-K item-2.02 filings the existing
-- `filings` table already carries (Phase 14 added the `items` column), so a
-- single SELECT against `filings.items LIKE '%2.02%'` lists them.
--
-- Stocks only — ETFs, indexes and futures have no earnings, so these stay
-- NULL forever on every non-stock row. A stock Yahoo has no calendar data
-- for (Yahoo's coverage is uneven on small caps) keeps `next_earnings_at`
-- NULL even after a successful sync, and the symbol page falls back to a
-- cadence estimate derived from the past 8-K item-2.02 dates.

-- Forward-looking next earnings date (UTC epoch-ms; date precision but
-- stored as a timestamp so it sorts and ages alongside the other
-- `*_at` columns). NULL when Yahoo has no upcoming date or the stock
-- has not been swept yet.
ALTER TABLE symbols ADD COLUMN next_earnings_at INTEGER;

-- When the Yahoo `earnings_calendar` scheduler section last refreshed
-- this stock's `next_earnings_at`. NULL = never swept. Stocks only.
ALTER TABLE symbols ADD COLUMN earnings_synced_at INTEGER;
