-- 0010_quarter_horizon.sql
-- The four picker horizons are day / week / month / quarter (PLAN.md Phase 30,
-- reworked 2026-05-23). The original `year` horizon was dropped: its score was
-- a pass-through of today's standing, so every historical backtest year
-- produced an identical top-5. Quarter (~one earnings cycle, 60-day trailing
-- momentum gated on SMA200) replaces it.
--
-- The `picks` snapshot table stores horizons as text; clear any snapshot rows
-- with the retired key so /backtest never surfaces stale `year` slates.
-- Today's daily_close scheduler tick will write a fresh `quarter` snapshot.

DELETE FROM picks WHERE horizon = 'year';
