-- finance migration 0006: company leadership (Phase 14).
--
-- A company's officers and board, parsed from SEC Form 3/4/5 ownership XML
-- (each filing carries a structured reportingOwnerRelationship), plus the
-- 8-K item-5.02 leadership-change events surfaced from the filing history.
-- Stocks only: ETFs and indexes have no officers or board to track.

-- When this stock's leadership roster was last refreshed from SEC. NULL = never.
ALTER TABLE symbols ADD COLUMN leadership_synced_at INTEGER;

-- The 8-K item codes a filing reported, comma-separated as EDGAR's submissions
-- feed lists them (e.g. '5.02,9.01'). Set for 8-K rows; NULL for other forms
-- and for filing rows stored before this migration (refilled on the next SEC
-- sync). Item 5.02 is the officer/director departure-and-appointment event,
-- which the symbol page reads as the leadership-changes feed.
ALTER TABLE filings ADD COLUMN items TEXT;

-- One row per current insider of a company: its directors and Section-16
-- officers, identified by the reportingOwnerRelationship booleans on their
-- ownership filings. Filers who are only >10% beneficial owners (institutions)
-- are not stored — they are not leadership. Upserted incrementally as new
-- ownership filings are parsed; `last_seen` is the most recent ownership
-- filing date observed for the person, used to order the roster and to age out
-- insiders who have gone quiet (a rough proxy for a departure, since ownership
-- filings carry no clean "left the company" signal).
CREATE TABLE leadership (
    ticker        TEXT NOT NULL REFERENCES symbols(ticker) ON DELETE CASCADE,
    name          TEXT NOT NULL,            -- as filed: last-name-first, upper-case
    is_director   INTEGER NOT NULL DEFAULT 0,
    is_officer    INTEGER NOT NULL DEFAULT 0,
    officer_title TEXT,                      -- when is_officer, e.g. 'Chief Executive Officer'
    last_seen     TEXT NOT NULL,             -- most recent ownership filing date, YYYY-MM-DD
    PRIMARY KEY (ticker, name)
);
