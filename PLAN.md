# finance — Project Plan and Resume Doc

`finance` is a self-hosted, real-timeish market-watching web app for stocks,
ETFs, and indexes: live charts, key stats, fundamentals, and SEC filings. It is
a single Rust + axum binary backed by SQLite, with a Vite frontend. It deploys
at `finance.bythewood.me` and is published on GitHub as `finance`.

It is for *watching* the market only. No portfolio, no holdings, no money or
cost-basis tracking, no accounts, no auth. Single operator.

---

## How to use this file

This is the **living resume document**. The user periodically clears the AI
context to save tokens and resumes work from this file alone.

**Keep it current.** After every phase, significant decision, or change of
direction: update the **Status** section and append to the **Decisions log**.
The file must always be accurate enough that a fresh context can continue with
nothing else. Treat updating it as part of finishing any unit of work.

**This is vibe-coded: the user riffs ideas.** When the user floats an idea,
budget it into this file right away, into the relevant phase or the Design
section, rather than acting on it immediately. Phases are deliberately kept as
small, self-contained cutoffs so the user can clear the AI context between them
and resume cleanly from this file alone, keeping token use low.

---

## Status

_Last updated: 2026-05-22_

**Current phase:** Phases 0 through 11 complete and verified. Next: Phase 12
(Polish + ship).

**Roadmap (restructured 2026-05-22, see decisions log):** the home-page
redesign and commodities are pre-ship MVP phases. Order: 9 Search +
add-symbol, 10 Commodities & futures, 11 Home dashboard redesign, 12 Polish +
ship. Post-MVP backlog is phases 13 through 19.

**Watchlists dropped from the MVP (2026-05-22, see decisions log):** the user
no longer wants watchlists for now and wants the app to stay an opinionated,
no-customization view of the market. Phase 9 was reshaped from "Watchlists +
search" to "Search + add-symbol"; watchlists are parked in the post-MVP
backlog as Phase 19. The `watchlists` / `watchlist_items` tables stay in the
schema, unused for now.

**Done**
- **Phase 0 skeleton** — complete, verified. `make run` serves the dark
  futuristic dashboard on port 8000, migrations apply on boot, routes plus a
  themed 404 work, the request log is colored.
- **Phase 1 universe + history** — complete, verified.
  - `universe/starter.csv`: 144 symbols (6 indexes, 28 ETFs, ~110 stocks).
  - Stooq history provider with apikey; `seed.rs` is resumable (skips symbols
    that already have history) and has a circuit breaker.
  - Seed run populated deep daily history for **142 of 144** symbols (e.g.
    AAPL 10,507 bars, ^SPX 39,701 bars back to 1789). `^RUT` and `^VIX` return
    "No data" from Stooq, so they hold no history; they will still get live
    quotes from Yahoo in Phase 3. `meta.seed_completed` is intentionally NOT
    set while those 2 remain historyless.
  - Dashboard `/` shows the symbol grid with prices; `/s/{ticker}` shows a
    working lightweight-charts candlestick chart with range selectors and key
    stats; the history JSON API works; unknown tickers 404 with the theme.
- **Phase 2 scheduler + incremental history.** Complete, verified.
  - `src/scheduler.rs`: one long-lived tokio task on a 60s tick, modelled on
    `status/src/scheduler.rs`. On boot it resets stale `fetching` states and
    runs the first-run seed while `meta.seed_completed` is unset; then each
    cycle it runs the incremental daily-history refresh when due and the prune.
  - Incremental history: re-fetches only symbols whose `history_synced_at` is
    older than 20h (stale), asking Stooq for the window since each symbol's
    last stored bar, reusing `seed::store_daily`. Paced at 1.5s/request with a
    4-consecutive-error circuit breaker. Falls due ~every 6h.
  - Prune: drops `intraday_bars` older than 14d and `fetch_log` older than
    30d, ~daily. `daily_prices` is permanent and never pruned.
  - Every data job upserts its `data_status` row (idle/fetching/ok/error plus
    `next_run_at`) and appends one bulk `fetch_log` row (ticker NULL).
  - Verified: boot ran the seed and prune (seed `data_status` + `fetch_log`
    rows written); a staged-state run exercised the history job, which
    refreshed AAPL/MSFT incrementally, retried the 2 historyless indexes, and
    held the breaker at 2/4.
- **Phase 3 endpoint guardrails.** Complete, verified.
  - Migration `0002_endpoint_guard.sql` adds the `endpoint_guard` table: one
    row of guard state per upstream (today only `stooq`).
  - `src/guard.rs` holds `EndpointGuard`: a persistent, DB-backed reactive
    circuit breaker, a hard per-hour request budget, and request pacing.
    `acquire()` grants or denies one request (sleeping for the 1.5s pacing gap
    on a grant); `record_success` / `record_failure` feed the outcome back.
  - Breaker: trips immediately on an explicit HTTP 429/503 signal, or after 4
    consecutive ordinary failures; exponential backoff per trip (30m, 1h, 2h,
    ..., capped at 24h, honoring a longer `Retry-After`); a single half-open
    probe closes it on success or re-opens it (longer) on failure. Budget: 200
    requests per rolling clock hour, then requests are refused until the hour
    rolls. All state is in SQLite, so it survives restarts and is shared by
    the server and the `finance seed` subcommand.
  - `seed.rs` and `scheduler.rs` retrofitted: every Stooq call now goes
    through the guard; the old ad-hoc per-run consecutive-error breaker and
    manual `sleep` pacing are gone. A guard-denied run stops cleanly (the seed
    is resumable; the history job logs `skipped` and retries next cycle).
  - `stooq.rs`: a 429/503 now surfaces as a typed `RateLimited` error the
    guard recognizes by downcast. Stooq's plain "No data" reply (^RUT, ^VIX)
    is now treated as a successful empty response, not a failure, so genuinely
    historyless symbols never feed the breaker.
  - Verified with staged runs: a bad apikey tripped the breaker after exactly
    4 failures (30m backoff); a restart with the breaker still open denied the
    history job immediately with zero requests (state persists across
    restarts); a half-open probe with a good key recovered the breaker and
    refreshed all 10 staged symbols; pacing held requests 1.5s apart and the
    hourly budget accumulated across separate runs.

- **Phase 4 visual redesign: Paper Ledger.** Complete, verified.
  - Design system in `frontend/static_src/base/styles/`: a warm-paper palette
    (paper / surface / well, ink / ink-dim / ink-faint), hairline rules, and
    semantic green/amber/red as the only hues. Tokens are CSS custom
    properties in `base.scss :root`; build-time mixins in `_mixins.scss`.
  - Typography: Source Serif 4 (headings only), Inter (body / UI), JetBrains
    Mono (figures), self-hosted via `@fontsource`. `space-grotesk` dropped.
  - New ink brand mark and favicon — a rising figures line over the
    accountant's double underline — replace the neon candlesticks
    (`base.html`, `seo.rs`). Re-themed the base shell, home dashboard, ticker
    cards, symbol page, 404, and the lightweight-charts theme.
  - Symbol-page key stats rebuilt from a flat card grid into three skimmable
    gauges over a shared `.track` meter primitive: the day's open/close on its
    low-high range, the price on its 52-week range (current + prev close
    marked), and volume vs its 3-month average. Marker positions are derived
    in `symbols.rs` via a new `compute::pos` helper; no new network calls.
  - Chart interaction reworked (a mid-phase user request): horizontal pan and
    zoom are disabled, and a Google-Finance-style drag-to-measure tool added.
  - A dedicated UI polish pass is intentionally deferred to the ship phase
    (Phase 12 after the 2026-05-22 renumber); see the decisions log.

- **Phase 5 live quotes + SSE.** Complete, verified.
  - `src/market.rs`: US equity session clock in `America/New_York` via
    `chrono-tz` — `Session` (Closed/Pre/Regular/Post) plus the helpers the
    scheduler keys on. No holiday calendar (deliberate; see decisions log).
  - `src/providers/yahoo.rs`: `YahooProvider` behind a new `QuoteProvider`
    trait. One `v8/finance/chart` call (`interval=15m&range=1d`) returns a live
    quote and the day's 15-minute bars together. Maps `^SPX`->`^GSPC`,
    `^NDQ`->`^IXIC`; surfaces 429/503 as the typed `RateLimited` the guard
    recognises. Yahoo's chart `meta` carries no `marketState` /
    `regularMarketOpen`, so those columns stay null and the header's freshness
    label is derived from our own session clock instead.
  - `src/stream.rs`: the `Hub` — a `tokio::broadcast` channel plus a per-ticker
    viewer-interest registry. `src/routes/stream.rs`: the `/stream` SSE
    endpoint — registers interest in `?symbols=` (validated against the
    universe so a client cannot steer the poller at arbitrary symbols), emits
    an initial `market` event and a `quote` snapshot, then forwards live
    events; interest is released when the stream drops.
  - Scheduler: a demand-driven `intraday` job (market-hours only, polls *only*
    the symbols a browser is viewing right now — nothing when nobody is
    watching) and a once-a-day `daily_close` job (snapshots the whole universe
    shortly after 16:00 ET). Both route through a new `yahoo` `EndpointGuard`
    (1000/hr budget; `EndpointGuard::with_budget` was added). Quotes upsert the
    `quotes` table and the denormalized `symbols` snapshot columns; intraday
    bars upsert `intraday_bars` (the prune job already covers them).
  - Frontend `base/scripts/stream.js`: one `EventSource`, patches `data-field`
    price/change nodes in place, flashes cards on a move, drives the
    market-state pill. Closes cleanly on `pagehide` (and reconnects from the
    bfcache) so navigating between pages no longer aborts the stream
    mid-response.
  - The home dashboard and the symbol header now prefer the live quote
    (`symbols.last_price` / `prev_close`), falling back to the last daily close.
  - Verified: boot ran `daily_close` for 2026-05-20 — 144/144 symbols, 0
    errors, 144 `quotes` rows and 8,884 `intraday_bars`; the `yahoo` guard sat
    closed (0 failures). `/stream` held a stable 20s connection with the
    keep-alive ping; the SSE snapshot and `market` event arrived; the symbol
    header rendered the live quote (`$417.26  +13.15 (+3.25%)  At close`); the
    pill showed "Market closed"; navigating pages left zero console errors.
    The live intraday poll itself is gated to market hours, so it first runs
    at the next open — its fetch/store/broadcast path is exactly the one
    `daily_close` exercised end to end.

- **Phase 6 data health page.** Complete, verified.
  - `routes/health.rs`: `GET /health` renders the page; `GET /api/health`
    returns the same snapshot as JSON. Both build one `Health` snapshot —
    every `endpoint_guard` row (breaker state, trips, hourly budget used),
    every `data_status` job (state, last-ok, next-run, last error), and the
    50 newest `fetch_log` rows.
  - The page is rendered entirely by `health/scripts/health.js` from that
    snapshot — one renderer, no server/client duplication. The page route
    embeds the initial snapshot in a `<script type="application/json">` blob
    (`<`/`>`/`&` escaped to `\uXXXX`) so it draws with no flash; the script
    then re-pulls `/api/health` on each live nudge, every 30s (to keep the
    relative times honest), and whenever the tab regains focus.
  - Liveness rides the Phase 5 SSE hub. A new content-free `StreamEvent::Health`
    is published by the scheduler whenever a job marks `fetching` or writes a
    status / log row (~9 ping points across the five job runners). `/stream`
    forwards it as an `event: health` frame; `base/stream.js` re-broadcasts
    that as a `finance:health` window event, so the health page reacts without
    opening a second EventSource. A live amber "fetching now" banner shows
    while any job is mid-fetch.
  - Migration `0003` adds `endpoint_guard.hourly_budget`. The guard's `load()`
    writes and self-corrects it, so the page shows "used / budget" straight
    from the table with no upstream ceilings hardcoded in the route. A new
    `register_endpoints` boot step touches both guards so their rows — and
    budgets (stooq 200, yahoo 1000) — are right from the first boot.
  - A discreet "data health" link sits in the footer; a `--warn-soft` token
    was added to the palette for the amber data-health states.
  - Verified: migration `0003` applied cleanly on the seeded DB; `/api/health`
    returns the snapshot (stooq 8/200, yahoo 144/1000 — budget correct from
    boot); `/health` renders endpoints, jobs and the log tail with zero console
    errors on desktop and at 375px (no horizontal overflow); a `curl` held
    across a server boot received the `event: health` frames the seed and
    prune jobs published; dispatching `finance:health` in the page pulled a
    fresh `/api/health` and repainted in place.

- **Phase 7 fundamentals + filings.** Complete, verified.
  - `src/providers/sec.rs`: a `SecProvider` behind the new `FundamentalsProvider`
    trait. Three SEC EDGAR endpoints, no key (the contact email is appended to
    the User-Agent by `http::build_sec_client`): `company_tickers.json` for the
    bulk ticker->CIK map, `companyfacts` for XBRL facts, `submissions` for
    filing history. A 404 is a definitive empty (not a breaker failure); a
    429/503 surfaces as the typed `RateLimited`.
  - companyfacts is parsed defensively. SEC's `fy` field tags a fact with the
    *filing's* fiscal year, not the period's, so a comparative figure in a
    later 10-K is mislabelled by it (this bit during verification: every figure
    was shifted two years). The fiscal year is instead derived from the
    period-end date plus the company's fiscal-year-end month (the mode of its
    annual end months), so e.g. AAPL's Oct-Dec quarter reads as Q1 of the next
    fiscal year. Only clean full-year and discrete-quarter durations are kept
    (year-to-date roll-ups dropped by span length); quarterly balance-sheet
    figures are deliberately not collected (a 10-Q mis-tags its prior-year-end
    comparative). Ten metrics: revenue, net income, diluted EPS, diluted
    shares, dividend per share, total and current assets, total and current
    liabilities, equity.
  - Migration `0004` rekeys `fundamentals` UNIQUE to (ticker, metric, period)
    so an annual and a same-period-end quarterly figure no longer collide.
  - One `sec` scheduler job (daily due-check, weekly per-company staleness):
    resolves any missing CIKs from one bulk call, then sweeps stale stocks for
    companyfacts + submissions through a new `sec` `EndpointGuard` (600/hr).
    Resumable: each company's `fundamentals_synced_at` / `filings_synced_at` is
    stamped only on success. Gated on `SEC_CONTACT_EMAIL` being set.
  - `compute.rs`: nine graded ratios (P/E, dividend yield, profit margin, ROE,
    ROA, debt-to-equity, current ratio, revenue growth, earnings growth),
    computed off the latest full fiscal year plus the live price. Each carries
    a good/ok/bad `Grade`, a one-word verdict, a value-specific plain-English
    reading, and a static "how to read it" explainer.
  - Symbol page (`routes/symbols.rs`, `symbol.html`): a Fundamentals section of
    graded ratio cards (semantic green/amber/red on the value plus a verdict
    pill), a Financials section with an annual/quarterly table toggle
    (`fundamentals.js`), and a Recent SEC filings list linking out to EDGAR.
    Stocks only; a stock not yet synced shows a pending note, ETFs and indexes
    show none of it.
  - `/health` gained the `sec` endpoint and the `sec` job.
  - Verified: migration `0004` applied cleanly on the seeded DB; the boot SEC
    sweep resolved all 110 stock CIKs from one bulk call and stored
    fundamentals + filings for 110/110 companies with 0 errors (the `sec`
    guard sat closed). `/s/AAPL` renders the nine graded ratio cards (P/E
    40.9x Weak, profit margin 26.9% Strong, current ratio 0.89 Weak, ...), the
    annual/quarterly Financials toggle, and 18 SEC filings linking to EDGAR;
    fiscal years line up with the period-end dates, AAPL's December-ending
    quarter reads as Q1 of the next fiscal year and MSFT's June fiscal year
    likewise. ETF and index pages show none of the SEC sections; a not-yet-
    synced stock shows the pending note. Desktop and 375px both render with no
    horizontal overflow and zero console errors. The drag-to-measure chart
    readout now clears together with its selection band.

- **Phase 8 chart indicators + range readout.** Complete, verified.
  - `compute.rs`: three pure numeric indicator functions — `sma`, `ema`
    (seeded with the first simple average, then `2/(period+1)` weighting),
    and Wilder's `rsi` — each taking a slice of closes and returning one
    `Option<f64>` per bar (`None` through the warm-up period). The maths
    lives here, not in SQL or JS.
  - `routes/symbols.rs`: `/api/symbols/{ticker}/history` now returns an object
    — `candles` plus `sma50`, `sma200`, `ema21`, `rsi14` line series — rather
    than a bare candle array. It fetches a fixed 320-day lookback *before* the
    requested range, computes the indicators across the whole set, then trims
    every series to the visible window, so even the 200-day average is correct
    from the very first shown bar (verified: a 1M view has all four indicators
    populated from bar 1).
  - `chart.js`: SMA 50 / SMA 200 / EMA 21 drawn as toggleable overlay line
    series; a volume histogram pinned to the bottom strip on its own price
    scale; RSI in a second pane, created/destroyed on toggle (30/70 guide
    lines, pinned 0..100) so no empty pane lingers while it is off. A toggle
    row of indicator buttons, each with a line-swatch chart.js paints from its
    own palette; defaults SMA 50/200 + volume on, EMA + RSI off.
  - Indicator overlays use a muted, non-semantic ink palette (dusty blue /
    brown / violet): the candles own green/red and the app reserves
    green/amber/red for good/ok/bad, so the lines must not borrow them. Noted
    as a deliberate exception to the semantic-color rule.
  - A range-change chip beside the range buttons shows the % and absolute move
    over the chart's *visible* span, computed from the visible logical range
    (not the raw candle array) so it always agrees with what is drawn. A deep
    MAX history (e.g. `^SPX` back to 1789 at $0.51) is clamped by
    lightweight-charts to what legibly fits, and the chip then honestly
    reports just that visible span ("over 8 years") instead of an absurd
    +1,457,350%.
  - Verified: indicator maths correct and lookback-accurate across 1M/1Y/MAX;
    all five toggles work; the RSI pane creates and destroys cleanly; the
    range chip tracks the selected range and the visible span; the
    drag-to-measure tool still works (`▲ +34.91%` over a dragged interval);
    desktop and 375px render with no overflow and zero console errors;
    index / ETF / historyless (`^VIX`) pages all handled.

- **Phase 9 search + add-symbol.** Complete, verified.
  - `routes/search.rs`: `GET /search` browses and searches the universe. One
    SQL query backs both modes: an empty `q` lists everything, a non-empty
    `q` matches ticker and company name (`LIKE`, wildcards escaped); a `kind`
    filter (index / etf / stock) narrows it; results cap at 240, ordered
    exact-ticker, then ticker-prefix, then index/etf/stock, then alphabetical.
    Reuses the `ticker_card` macro, so the live stream patches prices in place
    exactly as on the Markets grid.
  - `routes/symbols.rs`: `POST /api/symbols` adds a symbol the universe does
    not hold yet. The ticker is validated and described in one guarded Yahoo
    request (`YahooProvider::lookup`, new); the symbol row is inserted
    (`is_seeded = 0`), the quote that same request returned is stored, and the
    history job is brought forward (`schedule_next`) so the deep daily
    backfill lands within a scheduler tick rather than after the ~6h interval.
    Idempotent (an existing symbol is reported, not duplicated); rejects an
    unknown symbol (404), an unmodelled instrument type such as a future
    (422), a malformed ticker (400), and a guard denial (503).
  - `providers/yahoo.rs`: refactored around a shared `fetch_chart`. The new
    `lookup` reads the chart `meta` (`instrumentType` / `longName` /
    `exchangeName` / `currency`) to classify the symbol (EQUITY to stock,
    ETF / MUTUALFUND to etf, INDEX to index; anything else is `Unsupported`).
    A 404 or a `chart.error` body is a clean "unknown symbol", not a guard
    failure.
  - The Search page shows an "Add <TICKER>" affordance only on a genuine
    zero-results miss for a plausible ticker; `search/scripts/search.js` POSTs
    it to `/api/symbols` and, on success, lands on the new symbol's page.
  - Scheduler: the incremental history and daily-close jobs no longer filter
    on `is_seeded`, so a user-added symbol is backfilled and snapshotted like
    a curated one. The seed itself stays curated-list-only.
  - `Card` / `to_card` moved from `routes/home.rs` to `models.rs` so Search
    and the Markets dashboard render the same tile. New Vite entry `search`.
    The watchlists nav links are gone; topnav and bottom nav are now Markets
    / Search.
  - User-added symbols are reachable through Search and their own `/s/` page
    but are deliberately kept off the curated Markets grid; Phase 11's home
    redesign decides their placement.
  - Verified: browse lists 145 symbols and the kind filter narrows correctly;
    ticker and company-name search both work; the add affordance shows only
    on a zero-results miss. `POST /api/symbols` added RBLX and ZM (real Yahoo
    stocks absent from the starter list) with their names and quotes; a
    re-add reported `added:false`; ZZZZ returned 404 and a malformed ticker
    400. The triggered history job backfilled ZM to 1,783 daily bars within a
    tick, and `/s/ZM` then rendered a full candlestick chart with indicators.
    Desktop and 390px both render with no overflow and zero page console
    errors.

- **Phase 10 commodities & futures.** Complete, verified.
  - New symbol `kind` of `future`. `universe/starter.csv` gains 9 curated
    futures (153 symbols total): index futures `ES=F` / `NQ=F` / `YM=F` and
    commodity futures `CL=F` (WTI) / `BZ=F` (Brent) / `GC=F` / `SI=F` /
    `HG=F` / `NG=F`. Yahoo serves these as `=F` symbols, which pass through
    `yahoo_symbol` unchanged. No schema migration: `kind` is free-text TEXT.
  - `providers/yahoo.rs`: `symbol_info` now classifies Yahoo's `FUTURE`
    instrument type as `future` (previously an `Unsupported` rejection); the
    no-instrument-type fallback also reads a trailing `=F` as a future. So the
    quote provider, the `/stream` SSE path, the `daily_close` / `intraday`
    jobs, and the Phase 9 add-symbol flow all handle futures with no further
    change.
  - Futures are live-quotes-only: Stooq carries no `=F` history, so the seed
    and the incremental-history job both exclude `kind = 'future'` (no wasted
    Stooq calls — not even the "No data" round-trip the historyless indexes
    `^RUT`/`^VIX` still make). Their price data comes solely from Yahoo: the
    once-daily `daily_close` snapshot of the whole universe, plus demand-driven
    intraday polling while a future's page is open. A future therefore has no
    `daily_prices` row and its symbol-page candlestick chart is empty, exactly
    like `^VIX`; the header still shows a live quote.
  - `routes/symbols.rs`: `valid_ticker` now accepts `=`, so a future is
    addable through Search like any stock; the add-symbol "unsupported type"
    message lists futures as allowed.
  - The Markets home page gained a "Futures & commodities" section between
    Indexes and Symbols; Search gained a "Futures" kind-filter pill; both the
    home and search orderings sort index, then future, then etf, then stock.
  - Verified: boot seed parsed 153 symbols and queued only `^RUT`/`^VIX` for a
    Stooq backfill (the 9 futures excluded). `/` shows the 9-card Futures
    section, `/search?kind=future` returns 9, `q=gold` matches `GC=F`.
    `/s/GC=F` (and the `%3D`-encoded link form) renders with FUTURE/COMEX
    tags and an empty chart. `POST /api/symbols` for `RTY=F` (an off-basket
    Russell future) returned `kind:future, added:true` and `/s/RTY=F` showed
    a live $2,852.90 quote — confirming the whole Yahoo `=F` path; the test
    symbol was then removed. Desktop renders with no layout breakage and no
    new console errors.

- **Phase 11 home dashboard redesign.** Complete, verified.
  - `routes/home.rs` rewritten: the flat ~155-card grid is gone, replaced by
    an opinionated, no-customization dashboard. The full browsable universe
    now lives solely on `/search`; a "Browse all N symbols" link points there.
  - Top row: nine sparkline cards — the six indexes (^SPX, ^DJI, ^NDX, ^NDQ,
    ^RUT, ^VIX) then the headline commodities (CL=F crude, GC=F gold, NG=F
    natural gas), a hardcoded curated set (`DASHBOARD` const). Each card shows
    a tiny current-session intraday line, the live price, and the day's %
    change.
  - The sparkline is server-rendered SVG: `compute::sparkline` maps a
    session's `intraday_bars` closes into polyline + area-fill points in a
    fixed `0 0 100 36` viewBox, with a faint dashed rule at the prior close.
    The latest session is isolated by a 23h window off each symbol's most
    recent bar. A symbol with no intraday bars shows a graceful "no intraday
    data" placeholder.
  - Movers: two panels, the day's top 8 gainers and top 8 losers, drawn from
    the curated large-cap stocks only (`is_seeded = 1 AND kind = 'stock'`) —
    deliberately not the whole universe, so a small user-added symbol's noise
    never crowds out a name worth noticing. Each row carries a soft magnitude
    tint scaled to the largest move shown across both panels.
  - Live: the dashboard registers stream interest in exactly its nine
    sparkline tickers (the `/stream?symbols=` query carries those nine and
    nothing else), so the demand-driven intraday job polls them — and only
    them — while the page is open. The movers panels are a fixed page-load
    snapshot (no `data-ticker`), keeping the polled set small and on-budget.
    The stream client patches each card's price/change in place, flips its
    up/down colour, flashes it on a move, and nudges the sparkline's trailing
    point onto the live price (`paintSparkline`, mirroring
    `compute::sparkline`'s y-scale via `data-lo`/`data-hi`).
  - New Vite `home` entry (`static_src/home/`) carries the dashboard styles;
    the `spark_card` and `mover_row` macros join `ticker_card` in
    `macros.html`.
  - Verified: `/` renders the nine sparkline cards and the two movers panels
    in the Paper Ledger look; the `/stream` request registered exactly the
    nine dashboard tickers; a forced daily-close snapshot populated the
    commodity cards (Yahoo serves the `=F` symbols cleanly); movers showed
    IBM +12% / INTU -20% with correctly scaled magnitude tints; desktop
    (1280px) and phone (390px) both render with no horizontal overflow and
    zero console errors.

**Resuming, next action**
Start **Phase 12 (Polish + ship)**: sitemap, favicon, the final Paper Ledger
polish pass (deferred from Phase 4), Dockerfile, docker-compose, sample files,
README, CLAUDE.md, `git init`. See the Phase 12 entry below.

Note: because `^RUT`/`^VIX` stay historyless, `meta.seed_completed` is never
set, so the boot seed re-runs on every restart (cheap: ~2 Stooq calls that
return "No data", which the guard now counts as successful empty responses,
everything else being a local upsert) and then defers the first incremental
history run by 6h. This is intended; see the decisions log. The 9 futures are
excluded from the seed entirely, so they add nothing to this.

**Build/run reminders for a fresh context**
- Build: `~/.cargo/bin/cargo build --manifest-path /home/dev/code/finance/Cargo.toml`
  (cargo is not on `PATH`; use the full path).
- Run the dev binary: `FINANCE_ROOT=/home/dev/code/finance PORT=8000 ./target/debug/finance`.
- `.env` exists with `STOOQ_APIKEY` (gitignored). The frontend is already built
  into `dist/`; rebuild it with `cd frontend && bun run build` after JS/SCSS edits.
- The DB at `data/db.sqlite3` is seeded; do not wipe it.

---

## Data sources

All free, no account, no API key.

- **Historical daily OHLCV — Stooq.** Endpoint
  `https://stooq.com/q/d/l/?s=<symbol>&i=d&apikey=<key>`. Stooq gates this
  endpoint behind an apikey obtained once via a captcha on stooq.com; the key
  lives in `.env` as `STOOQ_APIKEY` (gitignored, never committed). One call
  returns a symbol's entire daily history (decades). Behind the
  `HistoryProvider` trait, so swappable.

- **Intraday bars and live quotes — Yahoo Finance.** Endpoint
  `https://query1.finance.yahoo.com/v8/finance/chart/<symbol>`. No key, just a
  browser User-Agent; `interval=15m` returns intraday bars and the response
  `meta` block carries a live quote (price, previous close, day high/low,
  volume). Behind the `QuoteProvider` trait (added in Phase 5). Note: the
  chart `meta` does not include a market-state field, so the app derives the
  trading session from its own `market.rs` clock.

- **Fundamentals and filings — SEC EDGAR.**
  `company_tickers.json` (ticker to CIK), `data.sec.gov/api/xbrl/companyfacts`
  (XBRL facts), `data.sec.gov/submissions` (filing history). No key; SEC asks
  consumers to identify themselves, so `SEC_CONTACT_EMAIL` is appended to the
  User-Agent on SEC requests only. Stocks only; ETFs and indexes do not file.

P/E and dividend yield are **computed** in `src/compute.rs` from SEC EPS and
dividends plus the latest price; they are never stored.

### Anti-spam / caching policy

The user's hard requirement: minimal external calls, maximal local data, never
spam an endpoint.

- Validate an endpoint with **one** request before relying on a loop over it.
- Bulk jobs carry a **circuit breaker**: abort after a few (4) consecutive
  request errors instead of grinding the whole list.
- The seed is **resumable**: symbols that already have history are skipped, so
  a quota-limited run continues on the next `make seed`.
- The full per-symbol backfill runs **once**; results are stored permanently
  in `daily_prices` and never re-fetched in full.
- Bulk loops are paced at >= 1.5 s per request.
- Ongoing network use is small: a once-daily recent-window increment, and
  15-minute intraday polling for watched symbols only, during market hours only.
- Everything is cached in SQLite; the network is touched only for increments.

**Phase 3 hardened this into the `EndpointGuard`** (`src/guard.rs`, shipped).
The breaker and pacing are no longer ad-hoc per-run state inside `seed.rs` and
`scheduler.rs`: they are a persistent, per-endpoint guard, backed by the
`endpoint_guard` table, that every outbound call passes through. It adds a hard
per-hour request budget and trips at once on an explicit rate-limit signal
(429/503, honoring `Retry-After`), so a rate limit cannot be hit even across
restarts or by a future job.

---

## Architecture

- **Stack:** Rust + axum 0.8, single binary. sqlx + SQLite (WAL). minijinja
  templates. Vite frontend built with bun. lightweight-charts for charts.
- **Conventions:** match the sibling apps (`status`, `repos`). Tiny `main.rs`;
  `app.rs` builds `AppState` + `Config` + `router`; per-feature modules under
  `src/routes/`; `render.rs` / `middleware.rs` / `templates.rs` helpers; a
  Jinja2-faithful HTML formatter; assets resolved via `vite_asset` reading
  `dist/.vite/manifest.json`.
- **Providers:** traits in `src/providers/` (`HistoryProvider`, and later
  `QuoteProvider`, `FundamentalsProvider`), one struct per source, so a source
  is swappable without touching callers.
- **Scheduler (`scheduler.rs`):** one long-lived tokio loop running
  market-hours-aware background jobs, writing `data_status` and `fetch_log`
  and pinging the stream hub so the `/health` page tracks it live.
- **Endpoint guard (`guard.rs`):** a persistent, per-endpoint `EndpointGuard`
  (reactive circuit breaker + hard per-hour request budget + pacing,
  DB-backed) that every outbound data call passes through. Shipped in Phase 3;
  see the Anti-spam policy.
- **Real-time (`stream.rs`, shipped Phase 5):** a `tokio::sync::broadcast` hub
  that also carries a per-ticker viewer-interest registry; the scheduler
  publishes quote and market-session events; the `/stream` axum SSE endpoint
  forwards them; the browser uses `EventSource` and patches the DOM in place.
  The registry makes intraday polling demand-driven — only the symbols a
  browser is currently viewing are fetched.
- **Design, "Paper Ledger":** an old-school accounting-ledger feel reimagined
  futuristic and modern. Warm paper background, ink-dark text, hairline rules,
  monospace ledger figures, restrained serif headings. Color is semantic and
  sparing: green / yellow / red mean good / ok / bad (price moves, fundamental
  ratios, data-health states), never decoration. The UI must be skimmable:
  every number's meaning clear at a glance, small visualizations (range bars,
  comparisons, paired values) over flat metric-card grids. Both phone and
  desktop are first-class; built mobile-first in CSS; charts touch- and
  pointer-driven. Reference points: railway.com, openai.com, anthropic.com.
  Shipped in Phase 4; tokens live in `base.scss :root`. A final polish pass is
  deferred to the ship phase (Phase 12).

### SQLite schema (`migrations/0001_initial.sql`)

Timestamps are UTC epoch-ms; trading dates are `TEXT` `YYYY-MM-DD`.

- `symbols` — the universe (stock/etf/index/future), CIK, sync timestamps,
  denormalized last price.
- `daily_prices` — deep daily OHLCV. Permanent, never pruned.
- `intraday_bars` — recent intraday OHLCV. Pruned to ~14 days.
- `quotes` — latest live quote snapshot, one row per symbol.
- `fundamentals` — long/narrow SEC XBRL facts, one row per metric/period;
  UNIQUE rekeyed to (ticker, metric, period) by migration `0004`.
- `filings` — SEC filing history.
- `watchlists` + `watchlist_items` — named lists of tickers. In the schema
  but unused: the watchlist feature is deferred to Phase 19 (see Status).
- `fetch_log` — append-only history of background fetches.
- `data_status` — current state per job, for the live status pill.
- `endpoint_guard` — per-upstream guard state + hourly budget (migrations
  `0002`, `0003`).
- `meta` — key-value settings (`seed_completed`, etc.).

---

## Phases

- [x] **Phase 0 — Skeleton.** Scaffold, `AppState`/`router`, migration, home +
  seo routes, Vite base entry, themed shell. `make run` serves a styled empty
  dashboard.
- [x] **Phase 1 — Universe + history.** `starter.csv`, provider traits,
  Stooq history provider, `seed` (universe + history backfill), `compute.rs`,
  symbol detail page with a real daily chart.
- [x] **Phase 2 — Scheduler + incremental history.** `scheduler.rs` job loop,
  daily history increment, prune job, `data_status` / `fetch_log`, first-run
  seed on boot.
- [x] **Phase 3 — Endpoint guardrails.** A persistent, per-endpoint
  `EndpointGuard` so a third-party rate limit can never be hit. New
  `endpoint_guard` table (migration `0002`) and `src/guard.rs`; retrofit
  `seed.rs` and `scheduler.rs` to route every Stooq call through it. The guard
  combines: a reactive circuit breaker that opens immediately on HTTP
  429/503/`Retry-After` (honored) or after a failure streak, with exponential
  backoff per trip (e.g. 30m, 1h, 2h, capped 24h) and a half-open probe to
  recover; a hard per-hour request budget per endpoint (when spent, jobs skip
  the rest of their work until the hour rolls); and request pacing. All state
  is DB-backed, so it survives restarts and is shared across jobs. Replaces the
  current ad-hoc per-run consecutive-error breaker.
- [x] **Phase 4 — Visual redesign: Paper Ledger.** Establish the design system
  (see Architecture, Design): warm-paper palette, ink text, hairline rules,
  monospace ledger figures, serif headings, semantic green/yellow/red. Re-theme
  the base shell, home dashboard, symbol page and 404. Redesign the symbol
  page's key stats to be skimmable instead of a flat card grid: the 52-week
  range drawn as a bar/line with the current price and previous close marked
  along it; volume shown against its own average; open and close paired with
  their % moves. Build reusable components so every later phase is built into
  this look. Anticipate (but do not yet wire) live-price elements.
- [x] **Phase 5 — Live quotes + SSE.** `market.rs` (US market hours via
  `chrono-tz`), Yahoo quote provider, `stream.rs` broadcast hub, `/stream` SSE
  endpoint, frontend stream client, live ticker cards, status pill. Shipped;
  intraday polling is demand-driven (only the symbols a browser is viewing,
  during market hours) plus a once-a-day whole-universe close snapshot.
- [x] **Phase 6 — Data health page.** A `/health` page that shows everything
  openly and cleanly: per-endpoint guard state and per-hour budget used, each
  background job's status / last-ok / next-run, a live "fetching now"
  indicator, and a streaming tail of `fetch_log`. Built on the Phase 5 SSE hub
  so live fetches and logs update in place.
- [x] **Phase 7 — Fundamentals + filings.** SEC provider, SEC jobs, computed
  P/E and dividend yield, fundamentals tables and filings list on the symbol
  page. Fundamental ratios get semantic color coding (red = poor, yellow = ok,
  green = strong) so each number's quality reads at a glance. Each metric also
  carries a short plain-English explanation of what it means and how to read
  it (e.g. "P/E around 20 is healthy; around 300 is very richly priced"), and
  each ticker's own value is interpreted as good / ok / bad against sensible
  thresholds, so a non-expert can tell at a glance whether a number is
  encouraging or a concern.
- [x] **Phase 8 — Chart indicators.** SMA 50/200 + EMA 21 overlays and an RSI
  pane, plus a volume histogram, all toggleable; indicator maths in
  `compute.rs`; the history API returns the series with a lookback so they are
  correct from the first shown bar. A range-change chip reports the move over
  the chart's visible span so the headline and the chart agree.
- [x] **Phase 9 — Search + add-symbol.** A `/search` page that browses and
  searches the whole universe (filter by kind, match ticker and company
  name), and a `POST /api/symbols` add-symbol flow that validates an unknown
  ticker against Yahoo, registers it, and triggers its history backfill.
  Watchlists were dropped from the MVP (see decisions log) and parked as
  Phase 19.
- [x] **Phase 10 — Commodities & futures.** (Promoted from the post-MVP
  backlog on 2026-05-22 — see decisions log — because the Phase 11 home
  redesign needs commodity data.) Extend the universe and the Yahoo quote
  provider to index futures (S&P, Nasdaq, Dow) and commodity futures (oil,
  gold, silver, natural gas, ...). Adds a new symbol `kind` of `future`. Yahoo
  serves these as `=F` symbols (`ES=F`, `CL=F`, `GC=F`), so they slot into the
  Phase 5 quote provider; if Stooq has no deep history for them, treat them
  like the historyless indexes (live quotes only).
- [x] **Phase 11 — Home dashboard redesign.** (Captured 2026-05-22; vision
  expanded 2026-05-22, see decisions log.) The current home page is a flat
  grid of ~144 ticker cards, which the user finds unhelpful. The goal: an
  opinionated, no-customization home page that lets you grasp what the market
  is doing at a glance on every front, and drill in from there. No
  watchlists, no per-user layout; the app decides what matters and the user
  reads it. Planned pieces: a row of sparkline cards across the top for the
  indexes and critical commodities (each a tiny current-day intraday line
  from `intraday_bars`, Phase 5, with last price and day change), and top /
  bottom mover lists, the day's biggest gainers and losers from daily plus
  intraday price change. The flat full-universe grid is demoted (grouped,
  shrunk, or moved off the landing view; decide when building). It is also
  where user-added symbols (Phase 9) find their place on the home view.
  Built in the Paper Ledger system.
- [ ] **Phase 12 — Polish + ship.** Sitemap, favicon, the final Paper Ledger
  polish pass (deferred from Phase 4 — see decisions log),
  Dockerfile, docker-compose, sample files, README, CLAUDE.md, `git init`.
  Registering the project in `taproot` is left as a manual step for the user.

Phases 13 through 19 are the post-MVP backlog: ideas captured during planning,
to be built after the Phase 12 ship. Order among them is loose, and several
depend on Phase 5 (live quotes) and Phase 7 (SEC data).

- [ ] **Phase 13: Market heat map.** A home-dashboard market-cap heat map: one
  tile per symbol, sized by market cap, colored green or red by the day's move
  (a treemap). Market cap is shares outstanding (from SEC, Phase 7) times the
  latest price. (The movers list and index sparklines once bundled here moved
  into the Phase 11 home redesign when it was promoted.)
- [ ] **Phase 14: Company leadership.** Track each company's current
  executives and board, leadership changes over time, and a per-leader track
  record: how the company fared during their tenure, and whether they are an
  industry insider or an outsider. Data source needs research: SEC DEF 14A
  proxy statements and 8-K item 5.02 (officer / director changes) give the
  roster and the changes, but an objective "track record" is partly editorial
  and needs a deliberate sourcing decision.
- [ ] **Phase 15: Industry trends.** Treat industries as first-class:
  aggregate symbols by industry (sector and industry come from SEC in Phase
  7), show industry-level performance, seasonality (the months an industry
  tends to do well or poorly, computed from `daily_prices`), and how the
  industry is trending currently.
- [ ] **Phase 16: Per-ticker anomaly feed.** On the symbol page, a feed of
  notable recent events for that one ticker: large changes in its
  fundamentals, leadership changes, and unusually large price moves or
  drawdowns. Builds on Phases 7 and 14.
- [ ] **Phase 17: Stock health read.** Synthesize fundamentals, price
  trajectory, leadership, and industry context into a single non-advice
  "health" read: is this a healthy company (capable leadership familiar with
  the industry, solid fundamentals, consistent gains with the occasional
  acceptable setback)? Explicitly NOT buy or sell advice, and labelled as such
  in the UI. Builds on Phases 7, 14 and 15.
- [ ] **Phase 18: ETF profiles.** Treat ETFs as first-class rather than the
  Phase 7 stocks-only treatment: top holdings and their weights (from SEC
  N-PORT), expense ratio, fund category, and the fund's own filing history
  (prospectus / 485BPOS, N-CEN). Needs a fund ticker-to-CIK source distinct
  from the operating-company `company_tickers.json`. Captured 2026-05-22 from a
  user question about ETF filings; see decisions log.

- [ ] **Phase 19: Watchlists.** Named lists of symbols the user curates:
  watchlist and per-list pages plus mutation APIs (create / delete / rename
  lists, add / remove symbols). The schema already carries the `watchlists`
  and `watchlist_items` tables and the `symbols.is_watched` flag. Dropped
  from the MVP on 2026-05-22 (see decisions log) because the app is meant to
  be an opinionated, no-customization market view; parked here and can be
  re-promoted later if that changes (as commodities once were).

---

## Key files

```
finance/
  Cargo.toml  Makefile
  migrations/  0001_initial.sql  0002_endpoint_guard.sql  0003_guard_budget.sql
               0004_fundamentals_unique.sql
  universe/starter.csv                curated seed list
  src/
    main.rs        entry + `seed` subcommand
    app.rs         AppState + Config + router
    db.rs          SqlitePool init, now_ms, meta helpers
    render.rs  middleware.rs  templates.rs
    models.rs  compute.rs  seed.rs
    scheduler.rs   background job loop (seed/history/intraday/daily_close/prune)
    market.rs      US market-session clock
    stream.rs      SSE pub/sub hub + viewer-interest registry
    providers/  mod.rs (traits)  http.rs  stooq.rs  yahoo.rs  sec.rs
    routes/  mod.rs  home.rs  symbols.rs  search.rs  stream.rs  health.rs  seo.rs
    guard.rs
  templates/  base.html  includes/  pages/
  frontend/static_src/  base/  home/  symbol/  health/  search/
```

---

## Commands

- `make run` — Vite watch + `cargo run` on port 8000 (dev).
- `make build` — Vite assets + release binary.
- `make seed` — re-run the universe seed (idempotent). Same as `finance seed`.
- `make start` — run the release binary.
- Build with the full cargo path in this container: `~/.cargo/bin/cargo`.
- Run the dev binary with `FINANCE_ROOT=/home/dev/code/finance` so it finds
  `templates/` and `dist/`.

---

## Decisions log

- **2026-05-21 — Stooq history endpoint hit an apikey gate.** Stooq's free
  per-ticker CSV now returns "Get your apikey:" and its bulk database download
  returns "Unauthorized". Briefly considered moving history to Yahoo.
- **2026-05-21 — Stooq kept, with a user-supplied apikey.** The user obtained a
  Stooq apikey (captcha flow). It works: one call returns a symbol's full daily
  history. The key is stored in `.env` (`STOOQ_APIKEY`, gitignored). Stooq
  stays the history source; Yahoo remains the intraday/quote source.
- **2026-05-21 — Anti-spam policy added.** After a seed run hammered Stooq 144
  times with zero successes, added the validate-before-loop rule and a
  consecutive-error circuit breaker (see Anti-spam / caching policy). `seed.rs`
  is now resumable: it skips symbols that already have history.
- **2026-05-21: Phase 2 scheduler shipped.** `src/scheduler.rs` runs the boot
  seed, the ~6-hourly incremental history refresh, and the ~daily prune, each
  writing `data_status` and `fetch_log`. The incremental job re-fetches only
  symbols stale beyond 20h, so each symbol is hit at most about once per
  trading day. After a boot seed the history job is deferred one interval so
  it cannot re-fetch, on the same boot, the symbols the seed just touched.
- **2026-05-21: Visual direction set to "Paper Ledger".** The original dark,
  cyan/magenta neon look read dated. New direction: an old-school accounting
  ledger reimagined futuristic-modern, on warm paper with ink text, hairline
  rules and monospace figures, color used only semantically (green/yellow/red
  = good/ok/bad). Reference points: railway.com, openai.com, anthropic.com.
  See Architecture, Design. Becomes its own phase (Phase 4).
- **2026-05-21: Endpoint guardrails promoted to their own phase (Phase 3).**
  The user considers never hitting a rate limit critical. The ad-hoc per-run
  breaker is replaced by a persistent, per-endpoint `EndpointGuard` carrying
  both a reactive circuit breaker and a hard per-hour request budget. Done
  before live quotes because Phase 5 adds a second endpoint and a more frequent
  job.
- **2026-05-21: Roadmap expanded to 11 phases for clean cutoffs.** The user is
  vibe-coding and riffs ideas; they are budgeted into the plan as they arrive.
  New phases added: 3 endpoint guardrails, 4 visual redesign, 6 data health
  page, 8 chart indicators. Old phases 3 to 6 shifted to 5, 7, 9, 10. Phases
  are kept small and self-contained so context can be cleared and resumed
  between them.
- **2026-05-21: Phase 3 endpoint guard shipped.** `src/guard.rs` plus the
  `endpoint_guard` table (migration `0002`). Every Stooq call now routes
  through a persistent `EndpointGuard`: a DB-backed reactive circuit breaker
  (trips on a 429/503 at once or after 4 consecutive failures, 30m to 24h
  exponential backoff, half-open probe recovery), a hard 200-request-per-hour
  budget, and 1.5s pacing. The old per-run consecutive-error breaker in
  `seed.rs` and `scheduler.rs` is gone. Two refinements made during the work:
  a 429/503 now surfaces as a typed `RateLimited` error so the breaker trips
  immediately and honors `Retry-After`; and Stooq's "No data" reply for
  genuinely historyless symbols (^RUT, ^VIX) is now treated as a successful
  empty response rather than an error, so it no longer feeds the breaker (this
  also supersedes the Phase 2 "held the breaker at 2/4" behavior, as those two
  symbols were never a real endpoint failure). The 200/hour budget sits
  comfortably above one full universe refresh (~144 calls), so legitimate jobs
  are never starved while a runaway loop is still capped.
- **2026-05-21: Phase 4 Paper Ledger redesign shipped.** The dark neon theme
  is fully replaced by the "Paper Ledger" design system: warm paper, ink text,
  hairline rules, monospace figures, restrained serif headings, semantic
  color only. Typeface pairing chosen with the user: Source Serif 4 for
  headings, Inter for body, JetBrains Mono for figures (`space-grotesk`
  dropped). The symbol page's flat key-stats card grid became three skimmable
  gauges over a shared `.track` meter primitive. A new ink brand mark and
  favicon — a rising figures line over the accountant's double underline —
  replace the neon candlesticks the user disliked.
- **2026-05-21: chart pan/zoom disabled; drag-to-measure added.** A mid-Phase-4
  user request. The lightweight-chart no longer scrolls or zooms: panning only
  revealed empty space past the loaded range, and the range buttons already
  cover navigation. In its place, a Google-Finance-style measure tool —
  click-drag across the chart shades the interval and shows its % and absolute
  change with an up/down indicator. All client-side in `chart.js`, no new
  network calls. (Related to Phase 8's chart work, but shipped now.)
- **2026-05-21: final UI polish deferred to Phase 10.** The user judged the
  Paper Ledger redesign a clear improvement but still short of fully polished,
  and chose to hold a dedicated polish pass until all features are built
  rather than polish now and re-polish after every later phase. Phase 10's
  "final theme pass" absorbs this.
- **2026-05-21: post-MVP backlog captured (Phases 11 to 16).** A vibe-coding
  session produced eight feature ideas, budgeted into the plan as future
  phases rather than acted on now: futures and commodities (11); a market-cap
  heat map plus a movers list (12); company leadership tracking (13); industry
  trends and seasonality (14); a per-ticker anomaly feed (15); and a
  synthesized non-advice "stock health" read (16). One further idea, plain
  -English explanations of each fundamental plus a good / ok / bad reading of
  each ticker's value, was folded into the existing Phase 7 rather than given
  its own phase. These sit after the Phase 10 ship; ordering among 11 to 16 is
  loose. Phase 13's "leader track record" still needs a real data-source
  decision (SEC proxies give the roster, not an objective record).
- **2026-05-21: Phase 5 polling is demand-driven, not market-wide.** Asked how
  the intraday job should choose what to poll. The user's rule: poll a symbol
  only while it is actually being viewed during market hours, and give
  everything one closing update at the bell. So the stream `Hub` carries a
  per-ticker viewer-interest registry: each `/stream` connection registers the
  tickers its page shows, and the `intraday` job polls exactly
  `hub.viewed()` — nothing when nobody is watching. A separate once-a-day
  `daily_close` job snapshots the whole universe shortly after 16:00 ET so
  every symbol still gets a same-day close. Both go through a new `yahoo`
  `EndpointGuard` with a 1000/hr budget (`EndpointGuard::with_budget` added;
  Stooq keeps the 200 default). Routine 1-minute `intraday` runs write only
  `data_status`, not `fetch_log`, so the minute cadence does not bury the log;
  errors and guard stops are still logged.
- **2026-05-21: no market-holiday calendar.** `market.rs` models weekday
  pre/regular/post hours in `America/New_York` but not exchange holidays. A
  full holiday calendar needs yearly upkeep and the cost of omitting it is
  tiny: on a holiday the demand-driven job just polls a flat market (only if
  someone is watching), and `daily_close` fetches one unchanged quote per
  symbol. Neither risks a rate limit or stores bad data.
- **2026-05-21: SSE closed on `pagehide`.** Navigating between pages was
  aborting the open `EventSource` mid-response, which Chrome logs as
  `ERR_INCOMPLETE_CHUNKED_ENCODING` — one console error per navigation. The
  stream client now calls `es.close()` on `pagehide` (and reconnects from the
  bfcache on `pageshow`), so navigation is clean.
- **2026-05-21: two future improvements captured (vibe-coding).** Budgeted into
  later phases rather than acted on now: (1) the **selected chart timeframe
  should drive the headline change readout** — % and points over the chosen
  range — because the symbol header's day change reads as confusing once a
  longer range is selected; folded into Phase 8. (2) **Current-day sparklines
  on the dashboard index cards**, drawn from `intraday_bars`; folded into
  Phase 12.
- **2026-05-21: Phase 6 data-health page shipped.** `/health` (plus an
  `/api/health` JSON feed) lays the background-data machinery open: per-endpoint
  guard state and hourly budget used, each job's state / last-ok / next-run, a
  live "fetching now" banner, and a tail of `fetch_log`. Design decisions: the
  page has a single renderer — `health.js` builds the DOM from a snapshot, and
  the page route just embeds the first one in the HTML (so there is no flash)
  and serves the rest from `/api/health` — rather than rendering once in
  minijinja and again in JS. Liveness reuses the Phase 5 SSE hub via a new
  content-free `StreamEvent::Health` the scheduler pings on job transitions;
  `stream.js` re-broadcasts it as a `finance:health` window event so the page
  needs no second EventSource. Migration `0003` persists each guard's
  `hourly_budget` in its `endpoint_guard` row (written and self-corrected by the
  guard, and registered for both endpoints on boot) so the page reads
  "used / budget" from the table instead of duplicating the scheduler's request
  ceilings into the route layer.
- **2026-05-22: Phase 7 fundamentals + filings shipped.** SEC EDGAR is now a
  third data source behind a `FundamentalsProvider` trait (`providers/sec.rs`),
  swept by one `sec` scheduler job through a `sec` `EndpointGuard`. Key design
  calls: (1) one consolidated `sec` job does CIK resolution plus companyfacts
  plus submissions per company, rather than separate `fundamentals` and
  `filings` jobs: fewer guard cycles and one resumable sweep, though both
  `*_synced_at` columns are still tracked independently so a half-done company
  is finished next cycle. (2) The fiscal year of every XBRL fact is derived
  from its period-end date and the company's fiscal-year-end month, NOT SEC's
  `fy` field, which tags facts with the filing's year and so mislabels
  comparatives (caught in verification: stored data was shifted two years).
  (3) Quarterly balance-sheet figures are not collected (a 10-Q mis-tags its
  prior-year-end comparative as a quarter); the quarterly financials table
  shows the flow metrics only. (4) Ratios are computed off the latest full
  fiscal year, not a trailing-twelve-month sum: robust against the XBRL
  year-to-date trap and clearly labelled with the basis year. The user chose
  the rich ratio set (nine ratios) and the annual + quarterly table toggle.
- **2026-05-22: chart measure-tool readout `[hidden]` bug fixed.** The
  drag-to-measure readout chip stayed on screen after the selection was
  cleared: `.chart-readout` sets `display: flex`, which overrides the
  `[hidden]` attribute's `display: none`. Added
  `.chart-readout[hidden] { display: none }`, the same fix `.health-banner`
  already carries. User-reported during Phase 7.
- **2026-05-22: ETF filings noted as future work (now Phase 18).** Asked why
  ETFs show no filings. Phase 7 is operating-companies-only by design: ETFs
  have no XBRL `companyfacts` (no revenue / EPS), and EDGAR's
  `company_tickers.json` is an operating-company map. ETFs do file (N-PORT
  holdings, prospectuses) under a fund CIK, so an "ETF profile" (holdings,
  expense ratio, fund filings) is a genuine feature; budgeted as a post-MVP
  phase rather than stretched into Phase 7. Indexes genuinely do not file, so
  their empty state is correct.
- **2026-05-22: Phase 8 chart indicators shipped.** Toggleable SMA 50, SMA
  200, EMA 21 overlays, a volume histogram, and an RSI pane on the symbol
  chart, plus a range-change chip. Design calls: (1) the indicator maths
  (`sma`/`ema`/`rsi`) is pure numeric code in `compute.rs` returning
  `Option<f64>` per bar; the route shapes it. (2) The history API fetches a
  320-day lookback before the requested range so a 200-day average is correct
  from the first *shown* bar, then trims — the API response shape changed from
  a bare candle array to `{candles, sma50, sma200, ema21, rsi14}`. (3) RSI is
  a real second pane created/destroyed on toggle (lightweight-charts does not
  drop empty panes, so `removePane` is called explicitly). (4) Indicator lines
  use a muted blue/brown/violet palette — a deliberate exception to the
  semantic-color rule, since candles own green/red and lines are wayfinding,
  not value judgments. (5) The user picked EMA + volume + RSI on top of the
  core SMAs, and a change chip beside the range buttons. The chip is computed
  from the chart's *visible* logical range, not the candle array, so it agrees
  with what is drawn: a deep MAX history (`^SPX` to 1789) is clamped by the
  chart to what fits, and the chip reports only that visible span — this is
  the literal fix for "the chart range and the headline change should agree".
- **2026-05-22: roadmap restructured — home redesign + commodities promoted
  pre-ship.** The user judged the home page (a flat grid of ~144 ticker
  cards) unhelpful and wants it redesigned before the MVP ships: index and
  critical-commodity sparkline cards plus top/bottom mover lists. Since that
  needs commodity data, the old post-MVP "Futures and commodities" phase is
  promoted too. New MVP order: 9 Watchlists + search, 10 Commodities &
  futures, 11 Home dashboard redesign, 12 Polish + ship. The post-MVP backlog
  renumbered: old 11→10, 12→13 (now just the universe heat map — its movers
  and index sparklines moved into Phase 11), 13→14, 14→15, 15→16, 16→17,
  17→18. The Phase 4 "final polish pass" now lands in Phase 12. The user chose
  sparkline cards (over a combined normalized chart or a heat map) for the
  index/commodity display.
- **2026-05-22: watchlists dropped from the MVP; Phase 9 reshaped to search +
  add-symbol.** Asked how much watchlist editing Phase 9 should cover, the
  user said they do not want the watchlist feature for now: the app should
  stay an opinionated, no-customization view of the market. Phase 9 was
  reshaped from "Watchlists + search" to "Search + add-symbol" and shipped as
  such: a `/search` page (browse the whole universe, filter by kind, search
  ticker and company name) plus a `POST /api/symbols` add-symbol flow that
  validates an unknown ticker against Yahoo, registers it, and triggers its
  history backfill. Watchlists are parked in the post-MVP backlog as Phase 19
  (the `watchlists` / `watchlist_items` tables stay in the schema, unused for
  now) and can be re-promoted later, as commodities once were. Design calls:
  (1) one SQL query backs both browse and search, the filters guarded so an
  empty input is a no-op; (2) the add affordance appears only on a genuine
  zero-results miss, so a company-name search that did find hits is not
  nagged; (3) the add-symbol lookup reuses the Yahoo chart endpoint (one
  request yields the symbol's identity plus its quote) and goes through the
  shared endpoint guard; (4) the history and daily-close jobs dropped their
  `is_seeded` filter so a user-added symbol is maintained like a curated one,
  and the add route brings the history job forward so the backfill lands
  within a tick.
- **2026-05-22: the home page is an opinionated, no-customization market
  view.** The user's steer for Phase 11: the home page should let you grasp
  what the market is doing at a glance on every front, and drill in from
  there. It is deliberately an opinionated view with no per-user
  customization (the same reasoning that dropped watchlists). Folded into the
  Phase 11 description.
- **2026-05-22: Phase 10 commodities & futures shipped.** A new `future`
  symbol kind and 9 curated Yahoo `=F` futures (3 index — ES/NQ/YM; 6
  commodity — CL/BZ/GC/SI/HG/NG) added to the universe. Design calls: (1) the
  user picked a core 9-symbol basket over a wider one with agriculture and a
  Russell future. (2) Futures are live-quotes-only — Stooq has no `=F` data,
  so the seed and incremental-history job exclude `kind = 'future'` outright
  rather than letting them make a pointless "No data" Stooq call each cycle as
  the historyless indexes do; their prices come only from Yahoo's daily-close
  snapshot and demand-driven intraday polling. A future thus has no
  `daily_prices` and an empty candlestick chart, like `^VIX`. (3) `symbol_info`
  reclassifies Yahoo's `FUTURE` type from an `Unsupported` rejection to the new
  kind, and `valid_ticker` now accepts `=`, so futures are also user-addable
  through Search. (4) The Markets home page gained a "Futures & commodities"
  section and Search a "Futures" filter; no schema migration was needed
  (`kind` is free-text TEXT).
- **2026-05-22: Phase 11 home dashboard redesign shipped.** The flat ~155-card
  grid is replaced by an opinionated, no-customization dashboard: a row of
  nine sparkline cards (six indexes, three headline commodities) over two
  movers panels (the day's biggest gainers and losers). The browsable full
  universe moved entirely to `/search`. Design calls, several from the
  resume-session Q&A: (1) the sparkline set is a hardcoded curated nine —
  indexes plus crude / gold / natural gas — not a user watchlist. (2) Movers
  are restricted to curated large-cap stocks (`is_seeded` stocks), not the
  whole universe: the user wants names worth noticing (an AAPL down 5%), not a
  small user-added corp's noise. (3) The dashboard registers stream interest
  in only its nine sparkline tickers, so the demand-driven intraday poller
  stays small and on-budget; the movers panels are a fixed page-load snapshot
  with no live registration. (4) Sparklines are server-rendered SVG
  (`compute::sparkline`) over the latest session's `intraday_bars`; the stream
  client live-nudges the trailing point onto each new quote. The phone
  sparkline grid stays one column; a 2-up phone layout is left as a Phase 12
  polish candidate.

---

## Verification

- `make run`, confirm `data/db.sqlite3` is created and migrations apply.
- `make seed`, confirm `symbols` and `daily_prices` are populated and
  `meta.seed_completed` is set only when the seed actually succeeded.
- Dashboard `/` shows the symbol grid; `/s/AAPL` shows a working candlestick
  chart with range selectors and key stats.
- `/search` browses and searches the universe (filter by kind, match ticker
  and company name); searching an untracked ticker offers to add it, and
  adding it registers the symbol and backfills its history within a tick.
- `/` has a "Futures & commodities" section; futures (`kind = 'future'`) are
  never sent to Stooq and carry a live quote only — `/s/GC=F` renders with no
  daily chart, like `^VIX`.
- Phone (~360 px) and desktop are both fully usable: no unintended horizontal
  scroll, chart resizes, every feature reachable.
- No automated test suite or linter, matching the sibling projects.
