-- Phase 3 drops short-horizon prediction. The Day/Week/Month/Quarter pickers,
-- the /backtest page, and the once-a-day snapshot job that fed it are all gone;
-- the home page now reads quality off the existing health composite. Nothing
-- writes or reads the `picks` table any more, so remove it and sweep the stale
-- scheduler bookkeeping the job left in the status tables (otherwise /health
-- would keep showing a "picks" job frozen at its last run forever).
DROP TABLE IF EXISTS picks;
DELETE FROM data_status WHERE job = 'picks';
DELETE FROM fetch_log   WHERE job = 'picks';
DELETE FROM meta        WHERE key = 'picks_snapshot_date';
