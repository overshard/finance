-- finance migration 0002: per-endpoint request-guard state (Phase 3).
--
-- One row per outbound data endpoint (today only 'stooq'). It is the persistent
-- backing store for `EndpointGuard` (src/guard.rs): a reactive circuit breaker
-- plus a hard per-hour request budget, so a third-party rate limit can never be
-- hit even across restarts or by a future job. All *_at columns are UTC
-- epoch-milliseconds, matching the rest of the schema.

CREATE TABLE endpoint_guard (
    endpoint        TEXT PRIMARY KEY,                       -- logical upstream id, e.g. 'stooq'

    -- ── circuit breaker ──
    state           TEXT    NOT NULL DEFAULT 'closed',      -- closed | open | half_open
    fail_streak     INTEGER NOT NULL DEFAULT 0,             -- consecutive ordinary failures while closed
    trip_count      INTEGER NOT NULL DEFAULT 0,             -- consecutive trips; drives the backoff length
    opened_at       INTEGER,                                -- when the breaker last opened
    retry_at        INTEGER,                                -- earliest time a half-open probe may run

    -- ── hard per-hour request budget ──
    hour_start      INTEGER,                                -- start of the current rolling budget hour
    hour_count      INTEGER NOT NULL DEFAULT 0,             -- requests let through in the current hour

    -- ── bookkeeping ──
    last_request_at INTEGER,                                -- last request let through (drives pacing)
    last_ok_at      INTEGER,
    last_error      TEXT,
    last_error_at   INTEGER,
    updated_at      INTEGER NOT NULL
);
