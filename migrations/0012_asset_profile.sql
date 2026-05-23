-- finance migration 0012: asset profile sync tracker (Phase 15).
--
-- Sector and industry classification for each stock comes from Yahoo's
-- `quoteSummary.assetProfile` module. The columns the values land in —
-- `symbols.sector` and `symbols.industry` — already exist (since
-- `0001_initial.sql`) but were never populated by any prior phase. Phase 15
-- finally fills them through a new `asset_profile` scheduler section on the
-- existing `yahoo` `EndpointGuard`.
--
-- This migration only adds the sync-staleness timestamp. Stocks only — the
-- column stays NULL forever on every non-stock row (ETFs carry the Phase 28
-- `fund_metadata.category` for the equivalent classification; indexes and
-- futures have no sector).

ALTER TABLE symbols ADD COLUMN asset_profile_synced_at INTEGER;
