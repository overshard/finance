-- Phase 7: rekey the fundamentals uniqueness constraint.
--
-- 0001 keyed `fundamentals` UNIQUE on (ticker, metric, period_end). That breaks
-- once both annual and quarterly figures are stored: a full-year revenue and a
-- fourth-quarter revenue can share the same period_end (the fiscal year end),
-- so one would overwrite the other. The fiscal `period` label ('FY2024' vs
-- 'Q4-2024') is the value that is genuinely unique per figure, so the key moves
-- there.
--
-- SQLite cannot alter a table constraint in place, and `fundamentals` is still
-- empty (Phase 7 introduces its first writer), so the table is simply recreated.

DROP TABLE IF EXISTS fundamentals;

CREATE TABLE fundamentals (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    ticker      TEXT NOT NULL REFERENCES symbols(ticker) ON DELETE CASCADE,
    metric      TEXT NOT NULL,   -- revenue | net_income | eps_diluted | shares_diluted
                                 -- | dividends_per_share | assets | liabilities | equity
                                 -- | assets_current | liabilities_current
    period      TEXT NOT NULL,   -- 'FY2024' or 'Q3-2024'
    fiscal_year INTEGER NOT NULL,
    fiscal_qtr  INTEGER,         -- NULL for a full-year figure
    period_end  TEXT NOT NULL,   -- YYYY-MM-DD
    value       REAL NOT NULL,
    unit        TEXT,            -- USD | USD/shares | shares
    form        TEXT,            -- 10-K | 10-Q
    filed_at    TEXT,            -- YYYY-MM-DD
    UNIQUE (ticker, metric, period)
);
CREATE INDEX fundamentals_ticker_metric ON fundamentals(ticker, metric, period_end DESC);
