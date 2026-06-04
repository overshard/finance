-- Phase C: session-scoped dashboard watchlists.
--
-- The demand-only refocus made the dashboard a personal, editable watchlist
-- rather than a fixed view. There are no accounts: a browser is identified by
-- an opaque `fin_sid` cookie, and its watchlist symbols live here keyed on that
-- sid. Clearing the browser's cookies loses the list (acceptable by design).
-- The S&P 500 baseline is not stored — it is always shown by the dashboard.
--
-- `position` orders the list as the user arranged it; `added_at` is a tiebreak.
-- The FK cascades a symbol prune (seed reconcile) into the watchlists, so a
-- dropped universe symbol cannot linger as a dangling watchlist row.
CREATE TABLE watchlist (
    sid       TEXT    NOT NULL,
    ticker    TEXT    NOT NULL REFERENCES symbols(ticker) ON DELETE CASCADE,
    position  INTEGER NOT NULL DEFAULT 0,
    added_at  INTEGER NOT NULL,
    PRIMARY KEY (sid, ticker)
);

CREATE INDEX idx_watchlist_sid ON watchlist (sid, position);
