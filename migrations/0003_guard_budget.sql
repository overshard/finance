-- finance migration 0003: record each endpoint guard's per-hour request
-- budget in its own row (Phase 6).
--
-- The budget is a property of how each `EndpointGuard` is constructed
-- (`EndpointGuard::new` uses 200; the Yahoo guard uses 1000). Persisting it
-- lets the data-health page show "requests used / budget" straight from the
-- table, with no upstream-specific constant duplicated into the route layer.
-- `EndpointGuard::load` keeps this column in step with the guard's own budget.
--
-- The DEFAULT covers the rows that already exist (the guard corrects each one
-- to its true budget the next time that endpoint is used).

ALTER TABLE endpoint_guard ADD COLUMN hourly_budget INTEGER NOT NULL DEFAULT 200;
