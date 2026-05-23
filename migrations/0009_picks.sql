-- finance migration 0009: top picks snapshots (Phase 30).
--
-- A new `picks` table records, for each (snapshot_date, horizon), the five
-- stocks the app's forecast-horizon picker named at end-of-day. Snapshotted by
-- the scheduler right after `daily_close` runs (a known once-per-day moment
-- when every symbol carries a fresh close), one row per pick.
--
-- The snapshot is what makes the `/backtest` page honest: without it the
-- backtest could only replay today's algo over old data, and every algo tweak
-- would silently rewrite history. With it, the picks the app actually made on
-- every past trading day are immutable; the backtest reads them back and
-- simulates following them. v1 grows the table forward from the first deploy
-- — no retroactive backfill — so the backtest is honest about "history since".
--
-- `score` is the per-horizon ranker's raw value (higher is better); kept for
-- debugging and for the backtest's stat table. `price_at_pick` is the close
-- the pick was named at, so the backtest can compute the per-pick return at
-- the next snapshot without re-reading `daily_prices`.

CREATE TABLE picks (
    snapshot_date  TEXT    NOT NULL,                 -- YYYY-MM-DD ET trading date
    horizon        TEXT    NOT NULL,                 -- day | week | month | year
    rank           INTEGER NOT NULL,                 -- 1..5
    ticker         TEXT    NOT NULL REFERENCES symbols(ticker) ON DELETE CASCADE,
    score          REAL    NOT NULL,                 -- ranker's raw score, higher is better
    price_at_pick  REAL    NOT NULL,                 -- close used as the entry price
    PRIMARY KEY (snapshot_date, horizon, rank)
);

CREATE INDEX picks_ticker ON picks(ticker);
CREATE INDEX picks_date   ON picks(snapshot_date);
