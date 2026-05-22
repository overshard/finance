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

**Current phase: none in progress. Phase 22 (show data age everywhere) is
complete and verified locally; not yet committed or deployed.** Phases 0
through 12 (the MVP) plus Phase 14 (company leadership), Phase 18 (ETF
profiles), Phase 20 (strongest & weakest home panels), Phase 21 (home &
search refinements) and Phase 23 + 24 (financials table) are complete,
verified, and **live in production at https://finance.bythewood.me**.
Phase 22 adds a consistent, quiet data-age caption across the whole app —
the home dashboard, search, and every symbol-page data section, not just
`/health` — see the Done list and the decisions log; it ships on the next
`git push server master`. Remaining post-MVP work: the loose-ordered
Phase 13, 15, 16, 17, 19 backlog.

**Roadmap (restructured 2026-05-22, see decisions log):** the home-page
redesign and commodities are pre-ship MVP phases. Order: 9 Search +
add-symbol, 10 Commodities & futures, 11 Home dashboard redesign, 12 Polish +
ship. Post-MVP backlog is phases 13 through 19; the user picked Phase 18 (ETF
profiles) as the first post-MVP phase, done 2026-05-22. Phase 20 (strongest &
weakest home panels) was then inserted 2026-05-22 as a detour, to be built
next, ahead of the loose-ordered 13-17 backlog.

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

- **Phase 12 polish + ship.** Complete, verified, and live in production.
  - `Dockerfile`: multi-stage `rust:alpine` builder + `alpine:3.23` runtime,
    modelled on `analytics` but with no chromium and no Typst (finance renders
    no PDFs). The runtime stage copies the binary, `dist/`, `templates/`,
    `migrations/`, and `universe/` (the seed reads `universe/starter.csv`),
    runs as a non-root uid-1000 user, and sets `FINANCE_DATA_DIR=/data`.
    `docker-compose.yml` (container name `finance`, `/srv/data/finance:/data`
    volume) and `.dockerignore` alongside it.
  - `samplefiles/` gained `Caddyfile.sample` and `post-receive.sample` next to
    the existing `env.sample`, matching the sibling apps' standard
    `git push server master` deploy.
  - `README.md` and the project `CLAUDE.md` written.
  - Sitemap bug fixed: `routes/seo.rs` still listed the dropped `watchlists`
    page; it now emits `/` and `/search` only. The favicon was already done
    in Phase 4 (an inline SVG served at `/favicon.ico`).
  - `git init -b master` plus an initial commit (`.env`, `data/`, `dist/`,
    `target/`, `node_modules/` all correctly gitignored). A `server` remote
    (`root@bythewood.me:/srv/git/finance.git`) was added for the deploy. No
    GitHub repo yet — the user deferred GitHub.
  - Verified: `docker build` produces a 49.9MB image; the container boots
    (migrations apply, scheduler starts, listens on 8000), serves `/` and
    `/health` with 200 and resolved `/static/` assets, and degrades
    gracefully when `STOOQ_APIKEY` / `SEC_CONTACT_EMAIL` are unset (seed and
    SEC jobs disabled, prune still runs).
  - Final Paper Ledger polish pass: `base.scss` makes `<body>` a flex column
    with `main` flex-growing, so the footer pins to the bottom of the
    viewport on short pages (404, empty states) instead of floating mid-page
    over bare paper; `home.scss` lays the nine home sparkline cards 2-up on
    phones (they were one long column) while wider screens keep the existing
    auto-fill flow. Verified at 360 / 390 / 1280 px — desktop unchanged, no
    overflow, no new console errors.
  - taproot registration (done at the user's request — the plan had left
    this as a manual step): `finance|overshard/finance|master|yes|no` added
    to `projects.conf`, a `finance.bythewood.me` block added to the Caddyfile,
    `finance.bythewood.me` added to the caddy network aliases, and the taproot
    `CLAUDE.md` project table updated. A latent `quickstart.sh` bug was fixed
    in the same pass: it created `/srv/data/<name>` root-owned, so a uid-1000
    container could not write its db — it now `chown`s the dir to 1000 (this
    bit the finance deploy; see the decisions log).
  - Deployed to production at **https://finance.bythewood.me** on 2026-05-22.
    The server (alpine, `root@bythewood.me`) was provisioned the GitHub-free
    way: a bare repo `/srv/git/finance.git`, a working clone `/srv/docker/finance`
    whose `origin` is that bare repo, `/srv/data/finance` (chowned 1000), a
    hand-written `.env` (real `STOOQ_APIKEY`, `SEC_CONTACT_EMAIL`,
    `BASE_URL=https://finance.bythewood.me`), and the standard post-receive
    hook. `git push server master` deploys: the hook rebuilds the image,
    recreates the container, and reattaches it to `bythewood-edge`. Caddy was
    updated (Caddyfile + alias) and recreated. Verified: `/` and `/health`
    return 200 over HTTPS with a valid cert and the page renders the Paper
    Ledger UI; the first-run seed backfills on the live box.

- **Phase 18 ETF profiles.** Complete, verified, deployed to production.
  - The first post-MVP phase (the user picked it over the rest of the 13-19
    backlog). ETFs are now first-class: each ETF page carries a fund profile
    section — net assets (AUM), holdings count, top 25 holdings by weight,
    asset-class mix — plus its own SEC filing history.
  - **Data source: SEC N-PORT.** An ETF files as a registered fund, so its
    portfolio is not in the XBRL `companyfacts` behind the stock fundamentals;
    it is in quarterly N-PORT filings, one large XML each. `providers/sec.rs`
    gained inherent fund methods on `SecProvider` (N-PORT is wholly SEC-
    specific, no second source to trait over): `fund_ticker_map` (the bulk
    `company_tickers_mf.json`, ticker -> CIK + series id), `fund_filings` (one
    browse-edgar Atom request -> filing list + the fund's shape), and
    `fund_portfolio` (fetch + stream-parse one N-PORT `primary_doc.xml`).
    Migration `0005` adds `symbols.series_id` / `fund_synced_at` and the
    `fund_profiles` + `fund_holdings` tables. `quick-xml` is a new dependency
    (streaming parser — a bond-fund N-PORT runs to 15+ MB / 13k positions).
  - **Series filtering.** One registrant CIK can host dozens of fund series
    (the Vanguard and iShares trusts). Lookups are keyed on the SEC *series
    id* via browse-edgar, so a fund never picks up a sibling fund's filings or
    N-PORT. The unit investment trusts (SPY, DIA) and the commodity grantor
    trusts (GLD, SLV) are absent from `company_tickers_mf.json`, so a small
    hardcoded CIK fallback covers them.
  - **Filing-agent N-PORTs.** An N-PORT's accession number leads with the
    *filer's* CIK, which for a fund using a filing agent is not the
    registrant's; the Archives path needs the registrant CIK. The N-PORT XML
    is located from the filing's browse-edgar index-page URL (which always
    carries the registrant CIK) rather than from the accession.
  - **Commodity trusts.** GLD and SLV hold physical bullion, not a securities
    portfolio, so they file no N-PORT. They are detected by shape (files 10-K,
    no N-PORT, no N-CEN) and get a minimal profile: AUM from their 10-K
    `companyfacts` `Assets`, the filing list, and a "holds physical bullion"
    note — no holdings table.
  - **Scheduler.** `run_sec` now also resolves ETF fund CIKs and sweeps stale
    ETF profiles (weekly staleness on `fund_synced_at`), through the same
    `sec` `EndpointGuard` — 2 requests per ETF, well inside the 600/hr budget.
  - **No expense ratio, no category** (user decision, see decisions log):
    those are not in SEC structured data, only in prospectus HTML, so they are
    dropped. The asset mix derived from N-PORT holdings stands in.
  - **UI.** `symbol.html` gained a "Fund profile" panel (AUM / holdings / as-of
    + a thin ink-shaded asset-mix bar), a "Top holdings" list (each row a
    weight bar scaled to the largest holding), and the filing list is now
    shown for ETFs as well as stocks. Holdings display the N-PORT issue
    `title` (clean mixed-case) over the issuer `name` (often truncated caps).
  - Verified: a full 28-ETF sweep stored 28/28 profiles with 0 errors (26
    portfolio funds, 2 commodity trusts) in ~83s. N-PORT parsing is correct —
    QQQ 101 holdings (NVIDIA 9.0%, Apple 8.0%, ...), VOO 518, SPY 503, AGG
    13,186 from a 15.8 MB XML; AUM figures plausible (VTI $2.06T, GLD $155B).
    `/s/QQQ`, `/s/GLD`, `/s/AGG` render the profile, mix bar and holdings in
    the Paper Ledger look at 1280px and 390px with no overflow and zero
    console errors; `/s/AAPL` (stock) and `/s/^VIX` (index) are unchanged.

- **Phase 20 strongest & weakest home panels.** Complete, verified, deployed
  to production.
  - `compute.rs` gained a Phase 20 section: a `Standing` (a strong / fair /
    weak `Grade` plus a combined score) and the pure functions behind it.
    `grade_value` / `graded_mean` roll the nine Phase 7 ratio grades into a
    fundamental-strength score; `price_trend_score` reads a trailing-year
    return blended with how steady the climb was (the share of ~monthly
    sub-blocks that did not fall); `trajectory_score` blends that price trend
    equally with the revenue- and earnings-growth ratio grades; `standing`
    combines strength and trajectory ~2:1 in favour of fundamentals (a user
    steer); `trailing_return` exposes the 12-month return for display.
  - The badge's verdict reflects fundamental strength alone (it sits over the
    ratio cards); the combined score, which also folds in trajectory, is what
    the home panels rank by.
  - `models.rs` now owns `FundFact` (moved out of `routes/symbols.rs`) and a
    shared `latest_annual_inputs` that assembles `RatioInputs` for the latest
    fiscal year, so the symbol page and the home ranking grade a stock
    identically. `Card` gained an optional `strength`.
  - `routes/home.rs`: one `load_stocks` scan of the curated `is_seeded`
    stocks (price + all fundamentals + the trailing-year daily closes, three
    queries) feeds both the movers and the new strongest / weakest panels.
    `movers` was refactored to reuse that scan and now carries each row's
    badge; `strength_panels` ranks the graded stocks by combined score and
    takes the top 8 and bottom 8, with a magnitude tint scaled like the
    movers tint.
  - `routes/search.rs`: `attach_standings` batch-loads fundamentals for the
    stock rows among the results and attaches each card's badge.
  - A shared `verdict_badge` macro and a `.vbadge` style (a semantic
    green/amber/red pill, in `base.scss`); the badge rides on ticker cards,
    mover rows, the new standing rows, and above the symbol page's ratio
    cards. A new `standing_row` macro and a "Strongest & weakest" home
    section mirror the movers layout.
  - Verified: cargo + bun build clean; `/` renders the two new panels with
    badges on the movers; `/s/NVDA` shows a "Strong" overall badge above the
    ratios and `/s/AAPL` a "Fair" one; `/search` badges the stock cards and
    leaves ETFs / indexes / futures unbadged; desktop (1280px) and phone
    (390px) render with no overflow and zero console errors. `/` renders in
    ~225ms warm — the per-render standings scan, a fixed page-load snapshot
    as planned.

- **Phase 14 company leadership.** Complete, verified, deployed to production.
  - Migration `0006` adds the `leadership` table (one row per insider:
    director/officer flags, officer title, `last_seen`),
    `symbols.leadership_synced_at`, and an `items` column on `filings` for the
    8-K item codes.
  - `providers/sec.rs`: two inherent `SecProvider` methods, one HTTP request
    each so the `sec` guard wraps every call (as with the Phase 18 fund
    methods) — `ownership_index` (a company's recent Form 3/4/5 filings, from
    the `submissions` JSON) and `ownership_doc` (one ownership XML,
    stream-parsed by `parse_ownership` into its reporting people and their
    `reportingOwnerRelationship` flags). The `submissions` parse also now
    reads the `items` array, so 8-K item 5.02 is stored on every filing.
  - The `submissions` feed names a Form 4's primary document as an xsl-styled
    viewer path (`xslF345X06/form4.xml`) that serves rendered HTML; the raw
    parseable XML is the bare filename, so `ownership_doc` strips to the
    basename. Caught in verification — rosters came back empty until the fix.
  - `scheduler.rs`: `run_sec` gained a 4th section — for each stock whose
    `leadership_synced_at` is stale (monthly cadence, `LEADERSHIP_STALE_SECS`),
    it parses up to `LEADERSHIP_MAX_FILINGS` (30) recent ownership filings
    (only those since the last sync, after the first sweep), filters to
    directors and officers, and upserts the roster (`store_leadership`, a
    `last_seen`-guarded conflict so a newer role always wins). Shares the
    `sec` guard and early-exit; resumable.
  - `routes/symbols.rs` + `symbol.html`: a Leadership section on the stock
    symbol page — the current officer/board roster (officers ahead of
    directors, chiefs first; names title-cased, e.g. `O'BRIEN` -> `O'Brien`)
    over a provenance note, plus a "Recent leadership changes" list of 8-K
    item-5.02 filings linking to EDGAR. Stocks only; an unsynced stock shows a
    pending note, ETFs and indexes show no section. The roster is filtered to
    insiders seen filing within ~18 months, so departed people age out
    (ownership filings carry no explicit departure signal).
  - The "industry insider vs outsider" read was dropped (not in SEC structured
    data) and there is no per-leader tenure track record — the scope the user
    chose (see the decisions log).
  - Verified: migration `0006` applied on the seeded DB; the `sec` job's
    leadership sweep parsed ownership XML for the curated stocks (AAPL's roster
    came back as Tim Cook CEO + 7 more officers and 7 directors with titles;
    8-K item-5.02 changes detected back through 2024). `/s/AAPL` renders the
    roster and changes feed in the Paper Ledger look, CEO first; `/s/WMT` (not
    yet swept) shows the pending note; `/s/QQQ` and `/s/^SPX` show no
    Leadership section. Desktop (1280px) and phone (390px) render with no
    overflow and zero console errors. The sweep is paced and guarded — it
    backfills the universe over several daily `sec` cycles.

- **Phase 21 home & search refinements.** Complete, verified, deployed to production.
  - Three independent refinements to already-shipped features.
  - **Home page index/commodity split + index-futures swap.** `routes/home.rs`
    replaced the flat `DASHBOARD` const with `INDEXES` (each cash index paired
    with its index future) and `COMMODITIES`; `dashboard_cards` now returns two
    card lists, and a new `spark_cards_for` builds a sparkline section from any
    ticker slice. Outside the regular cash session each index card resolves to
    its index future (`^SPX`->`ES=F`, `^DJI`->`YM=F`, `^NDX`->`NQ=F`,
    `^RUT`->`RTY=F`), which trades nearly around the clock; `^NDQ` and `^VIX`
    have no clean tradable future and always show the cash index.
    `templates/pages/home.html` now renders two sections, "Indexes" and
    "Commodities", in place of the single "Indexes & commodities" row. `RTY=F`
    (Russell 2000 E-Mini) was added to `universe/starter.csv` as a
    live-quotes-only future so `^RUT` has a future to swap to.
  - **Synchronous full backfill on add-symbol.** A new
    `scheduler::backfill_symbol` pulls a freshly-added symbol's deep daily
    history from Stooq and its full SEC data (CIK resolution, fundamentals,
    filings and the leadership roster for a stock; the fund profile for an
    ETF), routed through the same `EndpointGuard`s as the background jobs and
    reusing the existing store helpers. `POST /api/symbols` calls it before
    responding, replacing the old deferred `schedule_next("history")`, so a
    user-added symbol's page is complete the moment the add returns.
    Best-effort: a guard denial or upstream error for any one piece is logged
    and skipped, leaving that piece for the normal scheduler sweep.
  - **Search auto-navigate on a single result.** `routes/search.rs` now
    redirects (303) straight to `/s/{ticker}` when a non-empty query matches
    exactly one symbol; browse mode (empty query) and multi-result searches
    render the search page as before.
  - Verified: `/` renders "Indexes" and "Commodities" as two sections; during
    the regular session the index cards are the cash indexes, and a forced
    off-hours run showed them swapped to `ES=F`/`YM=F`/`NQ=F`/`RTY=F` with
    `^NDQ`/`^VIX` staying cash. Adding `RDDT` (Reddit — a real stock absent
    from the universe) through `POST /api/symbols` took 51s and returned with
    545 daily bars, 73 fundamentals, 24 filings and a 13-person leadership
    roster all already stored; `/s/RDDT` then rendered complete.
    `/search?q=AAPL` and `?q=Microsoft` 303-redirect to the symbol page;
    browse and a multi-hit query do not. Desktop (1280px) and phone (390px)
    render with no overflow and zero console errors. (`RDDT` was a
    verification-only add and was removed afterwards; `RTY=F` stays as a
    curated symbol.)

- **Phase 23 + 24 financials table — Q4 column & readability.** Complete,
  verified, and deployed to production. Built together as one symbol-page
  financials pass; no new data source, no schema change, no network.
  - **Phase 23 — derived Q4 column.** SEC XBRL carries no discrete fourth
    quarter (no Q4 10-Q; Q4 lives only inside the 10-K's full-year figure, so
    `fundamentals` holds zero `fiscal_qtr = 4` rows). `routes/symbols.rs`
    gained `derive_q4`: for every fiscal year with the full year and all three
    prior quarters present, it derives `Q4 = FY - (Q1 + Q2 + Q3)` for the four
    flow / per-share metrics (revenue, net income, diluted EPS, dividend per
    share) and emits a synthetic `FundFact` with period label `Q4-<year>`.
    `build_fundamentals` folds the derived rows in before building the
    quarterly periods and the cell lookup, so a Q4 column slots into the
    quarterly table exactly like a stored quarter. Diluted EPS does not
    decompose perfectly (the diluted share count drifts quarter to quarter)
    but the residual is small; the plan calls for showing it. A genuine Q4
    row, should XBRL ever carry one, wins over the derived one.
  - **Phase 24a — per-cell growth cues.** Each financials-table cell now
    carries a period-over-period cue against the column to its left: a small
    up / down triangle, plus a semantic green / red where a rise has a clear
    good/bad reading. `FundRow.cells` changed from `Vec<String>` to
    `Vec<FundCell>` (a formatted figure plus a `dir` and a `sense`); a new
    `Trend` on each `TableMetric` sets the reading — `RiseGood` for revenue,
    net income, EPS and dividend; `RiseBad` for total liabilities; `Neutral`
    for total assets and shareholder equity (a rise there can be debt-funded
    or a fall a buyback, so the arrow shows but stays uncoloured). The first
    column, a flat figure, and a cell with a missing value on either side
    carry no cue.
  - **Phase 24b — missing-value glyph.** The middle dot (`·`), which read as
    a stray decimal point, was replaced with an em dash (`—`), the universal
    "no data" mark. There were three copies of the same placeholder constant
    (`routes/symbols.rs`, `compute.rs`, `templates.rs`), all changed, so the
    glyph is consistent across the financials table, the ratio cards, the ETF
    fund stats and every empty-value template filter app-wide.
  - Verified: cargo + bun build clean. `/s/AAPL` quarterly table shows the
    derived Q4-2024 and Q4-2025 columns and the maths checks out (Q4-2025
    revenue $102.5B = FY $416.2B − Q1 $124.3B − Q2 $95.4B − Q3 $94.0B). Growth
    cues render green / red on the four flow metrics and inverted on
    liabilities, with faint uncoloured arrows on the neutral rows; the first
    column and a flat dividend carry none. `/s/DELL` (which has reporting
    gaps) shows the em-dash glyph and correctly drops the cue across a missing
    column; its two uncomputable ratio cards also show `—`. No `·` remains
    except as a legitimate separator (page title, footer, the leadership role
    line). Desktop (1280px) and phone (390px) render with no overflow and
    zero console errors; the wide quarterly table scrolls within its card.

- **Phase 22 show data age everywhere.** Complete, verified locally; not yet
  committed or deployed. A consistent, quiet data-freshness caption now rides
  across the whole app — the home dashboard, search, and every symbol-page
  data section — not only `/health`. Built per a small design pass (see the
  decisions log): a section-level caption (one quiet line per section, never
  per card), and mixed wording — a relative "N ago" for live quotes and SEC
  syncs, an absolute clock time or date for daily and dated data.
  - **Two new minijinja filters** (`templates.rs`): `asof` (epoch-ms -> an
    absolute market-clock anchor — a time of day `2:14pm` when the moment is
    today, else `May 21`, else `May 21, 2024`) and `shortdate` (a
    `YYYY-MM-DD` string -> `May 21` / `May 21, 2024`). The existing `ago`
    filter (relative `N ago`) is reused for sync times. `asof` formats in
    `America/New_York`, matching `market.rs`.
  - **Home** (`routes/home.rs`, `home.html`): each of the four sections
    carries a caption on its `.section-title`. Indexes / Commodities show
    "prices as of {clock}" off the freshest `last_quote_at` among the
    section's symbols; Today's movers the same off the curated stocks;
    Strongest & weakest "fundamentals synced {ago}" off the freshest
    `fundamentals_synced_at`. `spark_cards_for` now returns a
    `SparkSection { cards, asof }`; `load_stocks` carries each stock's
    `last_quote_at` and `fundamentals_synced_at`.
  - **Live sections stay honest.** The Indexes / Commodities cards stream
    live, so their caption's time sits in a `data-field="spark-asof"` span
    that `stream.js` refreshes to the current market clock whenever a
    sparkline-card quote lands. The movers / strongest-weakest panels are
    fixed page-load snapshots (per Phases 11 and 20), so their captions are
    plain static text.
  - **Search** (`routes/search.rs`, `search.html`): a "Prices as of {clock}"
    line above the results grid, off the freshest quote among the matches.
  - **Symbol page** (`symbol.html`; `symbols.rs` needed no change — the
    `SymbolRow` already carries every sync timestamp). The header's quote
    line gains the real quote age — "Live · quoted {ago}", live-refreshed to
    "quoted just now" by the stream client on each quote; the daily-close
    fallback reads "Last close · {shortdate}". Every data section title
    carries a caption: Key stats "as of {close date}", Fundamentals /
    Financials "synced from SEC {ago}" (`fundamentals_synced_at`), Leadership
    (`leadership_synced_at`), Fund profile (`fund_synced_at`), Recent SEC
    filings (`filings_synced_at`, falling back to `fund_synced_at` for an ETF,
    whose filings ride along with the Phase 18 fund sweep), Top holdings
    "holdings as of {N-PORT date}". Filing dates render through `shortdate`.
  - **Style.** A shared `.section-title__asof` (base.scss): a quiet ink-faint
    caption riding past the section's hairline rule (`order: 1`), the
    eyebrow's uppercase voice dropped for a calmer annotation; `.section-title`
    gained `flex-wrap: wrap` so a long caption drops to its own line on a
    narrow phone rather than overflowing. A `.results-asof` line for search.
  - Verified: cargo + bun build clean. `/` shows the four section captions —
    Indexes / Commodities live-refreshing their clock as SSE quotes landed
    (server-rendered "8:46am" became "3:47pm" once the intraday poll ran),
    movers / standings static page-load snapshots. `/search` shows the
    results caption. `/s/AAPL` header read "Live · quoted just now" and every
    section title carried its sync / close caption; `/s/QQQ` (ETF) showed the
    fund-profile, top-holdings (N-PORT date) and filings captions, the last
    correctly sourced from `fund_synced_at`. Desktop (1280px) and phone
    (360px) render with no horizontal overflow and zero console errors; a
    too-long caption wraps cleanly to a second line at 360px.

**Resuming, next action**
**Phase 22 (show data age everywhere) is complete and verified locally**
(2026-05-22) but **not yet committed or deployed** — it ships on the next
`git push server master`. The MVP plus Phase 14, Phase 18, Phase 20,
Phase 21 and Phase 23 + 24 are live at https://finance.bythewood.me;
Phase 22 will join them on the next push. No phase is in progress.
Remaining post-MVP work: the loose-ordered Phase 13, 15, 16, 17, 19
backlog; the user picks which to take next. There is still no GitHub repo
for finance: the user deferred that; if one is created later, add it as
`origin` and the `overshard/finance` slug already in taproot's
`projects.conf` lines up.

Note: Phase 18 added the `quick-xml` crate (N-PORT XML streaming parser) and
migration `0005`. A fresh `make run` applies `0005`; the ETF fund profiles
populate on the first `sec` job cycle that finds them stale.

Note: because `^RUT`/`^VIX` stay historyless, `meta.seed_completed` is never
set, so the boot seed re-runs on every restart (cheap: ~2 Stooq calls that
return "No data", which the guard now counts as successful empty responses,
everything else being a local upsert) and then defers the first incremental
history run by 6h. This is intended; see the decisions log. The 10 futures are
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
- `filings` — SEC filing history; `items` (8-K item codes, e.g. `5.02`) added
  by migration `0006`.
- `leadership` — a company's current officers and board, one row per insider,
  parsed from SEC Form 3/4/5 ownership XML (migration `0006`). Stocks only.
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
- [x] **Phase 12 — Polish + ship.** Sitemap fix, Dockerfile, docker-compose,
  sample files, README, CLAUDE.md, `git init`, the final Paper Ledger polish
  pass, taproot registration, and a live production deploy to
  https://finance.bythewood.me. Complete and verified — see the Phase 12
  entry in the Status section and the decisions log.

Phases 13 through 19 are the post-MVP backlog: ideas captured during planning,
to be built after the Phase 12 ship. Order among them is loose, and several
depend on Phase 5 (live quotes) and Phase 7 (SEC data).

- [ ] **Phase 13: Market heat map.** A home-dashboard market-cap heat map: one
  tile per symbol, sized by market cap, colored green or red by the day's move
  (a treemap). Market cap is shares outstanding (from SEC, Phase 7) times the
  latest price. (The movers list and index sparklines once bundled here moved
  into the Phase 11 home redesign when it was promoted.)
- [x] **Phase 14: Company leadership.** Complete, verified and deployed
  2026-05-22; see the Phase 14 entry in Status and the decisions log.
  (Picked as the next backlog phase and scoped 2026-05-22.) Two things on the
  symbol page,
  stocks only: a **current roster** of officers and board (name + title),
  built from SEC Form 3/4/5 ownership XML — each form carries a structured
  `reportingOwnerRelationship` (`isDirector` / `isOfficer` / `officerTitle`),
  so directors and Section-16 officers are identified and the >10%-owner
  filers filtered out; and a **leadership-changes feed**, a dated list of 8-K
  **item 5.02** filings (officer / director departures and appointments), read
  from the `items` array of the `submissions` JSON Phase 7 already fetches.
  The "industry insider vs outsider" read is **dropped** — it is not in SEC
  structured data (only DEF 14A prose), the same wall Phase 18 hit with
  expense ratios. **No per-leader tenure track record** (the user picked the
  roster-plus-changes scope over the fuller tenure-record one). Builds only on
  the SEC source already shipped; the ownership-XML sweep is network-heavier
  than Phase 7 (one request per filing) but paced and budgeted by the existing
  `sec` `EndpointGuard`.
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
  in the UI. Builds on Phase 20 (its composite fundamental-strength grade and
  trajectory measure are the core of the read), layering Phase 14 leadership
  and Phase 15 industry context on top. Builds on Phases 7, 14, 15 and 20.
- [x] **Phase 18: ETF profiles.** Complete and verified (2026-05-22) — the
  first post-MVP phase, see the Phase 18 entry in Status and the decisions
  log. ETFs are first-class: a fund profile (AUM, holdings count, top 25
  holdings by weight, asset mix) and a fund filing history, sourced from SEC
  N-PORT via the new `company_tickers_mf.json` ticker map. Expense ratio and
  fund category were dropped (not in SEC structured data — user decision).
  Commodity grantor trusts (GLD, SLV) get a minimal AUM-only profile.

- [ ] **Phase 19: Watchlists.** Named lists of symbols the user curates:
  watchlist and per-list pages plus mutation APIs (create / delete / rename
  lists, add / remove symbols). The schema already carries the `watchlists`
  and `watchlist_items` tables and the `symbols.is_watched` flag. Dropped
  from the MVP on 2026-05-22 (see decisions log) because the app is meant to
  be an opinionated, no-customization market view; parked here and can be
  re-promoted later if that changes (as commodities once were).

- [x] **Phase 20: Strongest & weakest (home page).** Complete, verified, and
  deployed to production (2026-05-22) — see the Phase 20 entry in Status and
  the decisions log. (Captured 2026-05-22 as a detour ahead of the 13-17
  backlog.) A second pair of home panels alongside the day's movers, but
  a fundamentals-and-trajectory lens rather than a one-day price move: the
  strongest stocks and the weakest, a broader read on what is built well and
  what is struggling ("very similar to top movers, just a broader view").
  Planned pieces: (1) a composite fundamental-strength grade in `compute.rs`
  that rolls the nine Phase 7 graded ratios into a single strong / fair / weak
  verdict per stock; (2) a trajectory measure blending recent price trend
  (trailing return and how consistent the climb has been, from `daily_prices`)
  with fundamental growth (the Phase 7 revenue-growth and earnings-growth
  grades); (3) a combined per-stock score over strength plus trajectory, the
  home page showing the top N strongest and bottom N weakest by it, mirroring
  the movers panels (curated `is_seeded` stocks only, soft magnitude tint, a
  fixed page-load snapshot). Fundamentals exist only for stocks, so the ranking
  is necessarily stocks-only. (4) The composite strong / fair / weak verdict is
  surfaced consistently across the app: an overall badge on the symbol page
  above the per-ratio cards, plus a badge on search result rows and mover rows.
  All of it is derived from data already stored (Phase 7 fundamentals plus
  `daily_prices`): no new data source, no new network calls, no new endpoint
  guard. This phase is the foundation for Phase 17: it ships the
  fundamentals-plus-trajectory half of the eventual "health read", and Phase 17
  later layers leadership (Phase 14) and industry context (Phase 15) on top.
  Built in the Paper Ledger system.

- [x] **Phase 21: Home & search refinements.** Complete and verified
  2026-05-22 (not yet deployed); see the Phase 21 entry in Status and the
  decisions log. (Captured 2026-05-22 from three vibe-coding "side notes" while
  Phase 14 was being scoped; budgeted here, not acted on mid-phase.) Three
  independent tweaks to already-shipped features:
  (1) **Home page — split commodities from indexes** into their own section,
  and during the pre-market and post-market sessions show the index *futures*
  in place of the cash indexes (e.g. the S&P 500 E-mini `ES=F` instead of
  `^SPX`), the way the commodity futures are already shown. Needs a small
  design pass: the index->future mapping (^SPX->ES=F, ^NDX->NQ=F, ^DJI->YM=F;
  ^RUT->RTY=F; ^NDQ and ^VIX have no clean index future — decide their
  treatment) and which sessions count as "show the future" (pre + post; decide
  the overnight Closed window). (2) **Add-symbol — pull all data immediately.**
  Today `POST /api/symbols` stores the lookup quote and brings the history job
  forward a tick; the user wants the full backfill (history + whatever else)
  pulled synchronously on add, not deferred to the next scheduler cycle.
  (3) **Search — auto-navigate on a single result.** When `/search?q=` matches
  exactly one symbol, redirect straight to that symbol's page instead of
  rendering a one-card result the user must then click.

- [x] **Phase 22: Show data age everywhere.** Complete and verified locally
  2026-05-22; not yet committed or deployed. See the Phase 22 Done entry in
  Status and the decisions log. (Captured 2026-05-22 from a
  vibe-coding side note.) The user considers data freshness critically
  important and wants the age of displayed data surfaced consistently across
  the whole app, the home page included — not only on `/health`. Today
  freshness shows unevenly: the symbol header has a session-derived label and
  a last-close date, the ETF profile an "as of" date, `/health` the per-job
  last-ok times — but the home dashboard's sparkline cards and movers, the
  search cards, the new leadership roster and the fundamentals carry no
  visible age. The phase: a consistent, quiet "as of / N ago" treatment
  (quote time, daily-close date, last sync) wherever data is shown. Needs a
  small design pass on the wording and where it rides without cluttering the
  Paper Ledger look.

- [x] **Phase 23: Q4 in the quarterly financials table.** Complete, verified
  and deployed 2026-05-22, built together with Phase 24 — see their shared
  Done entry in Status and the decisions log. (Captured 2026-05-22
  from a vibe-coding side note.) The symbol page's quarterly financials table
  shows only Q1-Q3. The cause is confirmed: SEC XBRL carries no discrete Q4 —
  there is no Q4 10-Q, the fourth quarter is reported only inside the 10-K's
  full-year `FY` figure (zero `fiscal_qtr = 4` rows exist in `fundamentals`).
  The fix is to derive it: Q4 = FY - (Q1 + Q2 + Q3) for the flow metrics shown
  in that table (revenue, net income, diluted EPS, dividend/share), computed
  in `routes/symbols.rs` for each fiscal year where the full-year figure and
  all three quarters are present. No schema change and no new data — a pure
  derivation from facts already stored.

- [x] **Phase 24: Financials table readability.** Complete, verified and
  deployed 2026-05-22, built together with Phase 23 — see their shared Done
  entry in Status and the decisions log. (Captured 2026-05-22 from
  two vibe-coding side notes raised while Phase 21 was in progress.) Two
  presentation refinements to the symbol page's fundamentals area, no new data
  source: (1) **Per-cell growth cues.** In the annual and quarterly financials
  tables, make it visible at a glance whether the company is growing period
  over period: a semantic color and a small up/down icon on each figure
  showing whether it improved or worsened against the prior period (year over
  year in the annual table, quarter over quarter in the quarterly one). Needs
  a small design pass on which metrics a "rise is good" reading even applies
  to (revenue and net income clearly; total liabilities is the opposite) and
  how the cue rides without cluttering the Paper Ledger table. (2)
  **Missing-value glyph.** A fundamentals cell the company did not report
  currently shows a middle dot (`·`, the `DASH` const in `routes/symbols.rs`),
  which reads as a stray decimal point — the user noticed it on DELL. Replace
  it with an em dash (`—`) or a similar unambiguous "no data" mark.

---

## Key files

```
finance/
  Cargo.toml  Makefile
  migrations/  0001_initial.sql  0002_endpoint_guard.sql  0003_guard_budget.sql
               0004_fundamentals_unique.sql  0005_fund_profiles.sql
               0006_leadership.sql
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
- **2026-05-22: Phase 12 ship infrastructure shipped.** The deploy and docs
  scaffolding for the standard `git push server master` flow: a multi-stage
  `Dockerfile` (`rust:alpine` builder + `alpine:3.23` runtime) plus
  `docker-compose.yml` and `.dockerignore`, modelled on the `analytics`
  sibling but with no chromium and no Typst (finance renders no PDFs); the
  runtime image also copies `universe/` since the seed reads
  `universe/starter.csv`. `Caddyfile.sample` and `post-receive.sample` joined
  `env.sample` in `samplefiles/`. A `README.md` and a project `CLAUDE.md`
  were written. The sitemap in `routes/seo.rs` still listed the dropped
  `watchlists` page — fixed to emit `/` and `/search` only. `git init` plus
  an initial commit (71 files). The favicon was already done in Phase 4.
- **2026-05-22: Phase 12 polish pass + production deploy — MVP shipped.**
  Final Paper Ledger polish pass: a flex-column `<body>` so the footer pins
  to the viewport bottom on short pages instead of floating over bare paper
  (the 404 had a visible dead band), and a 2-up phone grid for the home
  sparkline cards (they were one long column — the candidate noted in the
  Phase 11 entry above). Desktop layout unchanged. Then, at the user's
  request (overriding the plan's "taproot is a manual step"), finance was
  registered in `taproot` and deployed live to **https://finance.bythewood.me**.
  taproot: a `projects.conf` line, a Caddyfile block, a caddy network alias,
  the CLAUDE.md table. Server provisioning was done GitHub-free — the working
  clone's `origin` is the local bare repo `/srv/git/finance.git`, not GitHub,
  so `git push server master` is the whole deploy loop (the user deferred
  GitHub). One real bug surfaced and was fixed: `quickstart.sh` created
  `/srv/data/<name>` root-owned, but project containers run as a uid-1000
  user and a bind mount keeps the host dir's ownership, so finance
  crash-looped on "Permission denied" until the data dir was `chown`ed to
  1000; `quickstart.sh` now does that `chown` itself. The deploy is verified:
  HTTPS 200 on `/` and `/health` with a valid cert, the Paper Ledger UI
  renders, and the first-run seed backfills on the live box. This closes
  Phase 12; the MVP is shipped. Remaining work is the post-MVP backlog
  (phases 13-19).
- **2026-05-22: Phase 18 picked as the first post-MVP phase.** Asked which of
  the loose-ordered backlog (13-19) to take first, the user chose 18, ETF
  profiles. It is the most self-contained: it builds only on data sources
  already shipped (SEC EDGAR) and needs no prior backlog phase.
- **2026-05-22: expense ratio and fund category dropped from Phase 18.**
  Research found neither is in SEC structured data — N-PORT carries holdings,
  net assets and an asset-class breakdown, but the expense ratio and a
  Morningstar-style category live only in prospectus (485BPOS) HTML, with no
  clean machine-readable field. Offered to curate them in `starter.csv`, parse
  the prospectus, or drop them; the user chose to drop both and show only what
  N-PORT provides. The asset-class mix computed from N-PORT holdings stands in
  for a category label.
- **2026-05-22: GLD/SLV get a minimal commodity-trust profile.** GLD and SLV
  are grantor trusts holding physical metal: they file 10-Ks, not N-PORT, and
  have no securities portfolio. The user chose a minimal profile for them —
  AUM (from the 10-K `companyfacts`), the filing list, and a plain note that
  the trust holds bullion directly — over either a bare filings list or the
  no-section treatment indexes get.
- **2026-05-22: Phase 18 ETF profiles shipped.** SEC N-PORT is now a fourth
  use of the EDGAR source. Design calls made during the build: (1) the fund
  methods are inherent to `SecProvider`, not behind a trait — N-PORT is wholly
  SEC-specific with no second source to abstract over, unlike the
  `HistoryProvider` / `QuoteProvider` / `FundamentalsProvider` concerns. (2)
  N-PORT lookups are keyed on the SEC *series id* (via the legacy browse-edgar
  Atom interface, validated to still work and routed through the `sec` guard),
  because one registrant CIK hosts many fund series and the modern
  `submissions` JSON cannot filter by series. (3) The N-PORT XML is located
  from the filing's browse-edgar index-page URL, not from the accession
  number, because a fund that files through a filing agent has the agent's
  CIK on the accession while the Archives path needs the registrant's — the
  index URL always carries the registrant CIK (this bit during verification:
  AGG and SPY 404'd until the fix). (4) `quick-xml` was added as a streaming
  parser: a bond aggregate fund's N-PORT runs to 15+ MB and 13k positions, so
  a DOM parse is wrong; only the top 25 holdings, the count and the asset mix
  are kept. (5) Holdings display the N-PORT issue `title` over the issuer
  `name`, since `name` often arrives truncated and all-caps. (6) The asset-mix
  bar uses ink shades, not semantic green/amber/red — a fund's composition is
  not a good/ok/bad judgement (the same exception the Phase 8 chart-indicator
  palette took). Phase 18 was deployed to production on 2026-05-22 via
  `git push server master` (migration `0005` applies on the box; the `sec` job
  backfills the 28 ETF profiles on its first due cycle).
- **2026-05-22: dashboard futures cards now update overnight.** The user
  noticed the home-page sparkline cards showing stale "last night" numbers
  while futures were trading. Cause: the demand-driven `intraday` poll was
  gated to the US *equity* session (`session.is_open()`: pre, regular, post),
  so through the overnight `Closed` window nothing was polled and the
  commodity-futures cards (CL=F, GC=F, NG=F) sat frozen on the 16:00 ET
  daily-close snapshot. Fix: `run_intraday` now always runs; inside a trading
  session it polls every viewed symbol as before, but outside one it polls
  only viewed *futures*, which trade nearly around the clock. Indexes, stocks
  and ETFs stay correctly frozen off-hours. Still demand-driven and guarded:
  nothing is polled unless a browser is viewing it. No futures-hours calendar
  is modelled (a closed futures market just returns a flat quote), consistent
  with the no-holiday-calendar decision in `market.rs`.
- **2026-05-22: futures pages no longer prompt to seed; SEC job runs on boot.**
  A future's symbol page showed the generic "no price history, run `make
  seed`" empty state, which is wrong: a future has no daily history by design
  (Stooq carries no `=F` data) and seeding cannot change that. The symbol page
  now hides the daily chart and key-stats sections for any symbol without
  daily history and shows an honest message: a short "followed with live
  quotes only" note for a future, a plain "no daily history available" for a
  historyless index (^RUT, ^VIX). The live quote in the header is unaffected.
  Separately, the `sec` scheduler job is now brought forward to the first tick
  on boot, the way the history seed is, so a deploy that introduces new
  SEC-backed data backfills within a tick instead of waiting out the ~24h
  interval. That is what populates the Phase 18 ETF profiles on the production
  box right after a deploy; the job is cheap when nothing is stale.
- **2026-05-22: Phase 20 inserted — strongest & weakest home panels.** The
  user wants the home page to carry, alongside the day's movers, a broader
  fundamentals-and-trajectory read: the strongest stocks (best combination of
  fundamental strength and price / business trajectory) and the weakest, "very
  similar to top movers, just a broader view". They also want the strong /
  fair / weak verdict shown consistently across the app, not just per-ratio on
  the symbol page. Budgeted as a new Phase 20, inserted as the next phase to
  build, ahead of the loose-ordered 13-17 backlog. Q&A settled four points:
  (1) it ships next, before 13-17; (2) it is the foundation for Phase 17, not
  a replacement: Phase 20 builds the composite fundamental-strength grade, the
  trajectory measure and the home panels, and Phase 17 later layers leadership
  (Phase 14) and industry context (Phase 15) on top into the fuller "health
  read"; (3) "trajectory" blends both recent price trend (from `daily_prices`)
  and fundamental growth (the Phase 7 revenue / earnings growth grades); (4)
  the rolled-up strong / fair / weak badge appears everywhere: symbol pages,
  search result rows, mover rows, and the new panels. It is numbered after the
  existing backlog rather than renumbering 13-19 (backlog ordering is already
  loose, as Phase 18 going first showed); its Phases-list entry is flagged as
  the next to build. No new data source: it is derived wholly from Phase 7
  fundamentals and `daily_prices`.
- **2026-05-22: Phase 20 strongest & weakest shipped.** The home page gained a
  "Strongest & weakest" pair of panels beside the movers, and a rolled-up
  strong / fair / weak badge now rides across the app. Design calls made
  during the build: (1) the badge's verdict reflects fundamental strength
  alone — the mean of the nine Phase 7 ratio grades — because it sits directly
  above the ratio cards on the symbol page and must be explainable by what is
  on screen; the home panels instead rank by a *combined* score that also
  folds in trajectory. (2) Per the user's steer the combined score weights
  fundamentals ~2:1 over trajectory (`STRENGTH_WEIGHT = 2/3`). (3) Trajectory
  blends a price-trend score with the revenue/earnings-growth grades; the
  price trend is a trailing-year (12 months, the user's pick) return blended
  with a steadiness measure — the share of ~monthly sub-blocks that closed up.
  The two growth grades thus feed both halves (strength and trajectory); the
  overlap is deliberate, the plan specified it. (4) Verdict cutoffs are a
  narrow ±0.2 band on the [-1, 1] score, since curated large-caps cluster near
  zero; they are tunable consts in `compute.rs`. (5) `FundFact` and the
  latest-fiscal-year `RatioInputs` assembly moved into `models.rs` so the
  symbol page and the home ranking grade a stock identically. (6) The home
  route computes the standings per render — one scan of the curated stocks
  (price + all fundamentals + a trailing year of daily closes) feeding both
  the movers and the new panels, a fixed page-load snapshot as planned; `/`
  renders in ~225ms warm. No new data source and no new network calls.
  Deployed to production on 2026-05-22 via `git push server master`.
- **2026-05-22: Phase 14 picked next and scoped.** Asked which loose-ordered
  backlog phase to take next, the user chose 14, company leadership. The plan
  had flagged it as needing a data-source decision; research settled it. SEC
  exposes two things cleanly and structured: a company's officer/board
  **roster** via Form 3/4/5 ownership XML (each carries
  `reportingOwnerRelationship` booleans + `officerTitle`), and a
  **leadership-changes** signal via 8-K **item 5.02**, readable from the
  `items` array of the `submissions` JSON Phase 7 already fetches (zero new
  network calls for the changes feed itself). The "industry insider vs
  outsider" read is not in SEC structured data — only DEF 14A prose — so it
  was dropped, the same call Phase 18 made on the expense ratio; the user
  confirmed drop over hand-curation. Scope chosen: the current roster + the
  changes feed, **not** the fuller variant that adds a per-leader
  return-during-tenure track record. Phase 14 builds only on the SEC source
  already shipped; its ownership-XML sweep is heavier than Phase 7 (one
  request per filing) but paced and budgeted by the existing `sec`
  `EndpointGuard`.
- **2026-05-22: three home/search side notes captured as Phase 21.** While
  Phase 14 was being scoped the user floated three refinements to shipped
  features: split the home page's commodities out from the indexes and show
  index *futures* (the S&P E-mini, etc.) during pre/post-market; make the
  add-symbol flow pull a new ticker's full data immediately instead of
  deferring the backfill to the next scheduler tick; and auto-navigate to a
  symbol's page when a search yields exactly one result. Per the vibe-coding
  process they were budgeted into the plan rather than acted on mid-phase —
  added as Phase 21 (Home & search refinements), to be taken up after Phase
  14. The home-page change carries an open design question (the index->future
  mapping and which sessions trigger it), noted in the Phase 21 entry.
- **2026-05-22: Phase 14 company leadership shipped.** SEC Form 3/4/5
  ownership XML is now a fifth use of the EDGAR source, behind two new
  inherent `SecProvider` methods (`ownership_index` / `ownership_doc`, one
  HTTP request each so the `sec` guard wraps every call, as with the Phase 18
  fund methods). Design calls made during the build: (1) the roster is built
  incrementally — the first sweep parses the 30 most recent ownership filings,
  later monthly sweeps only the filings since, and `store_leadership` upserts
  with a `last_seen`-guarded conflict clause so a stale re-parse never
  overwrites a newer role; the symbol page filters the roster to insiders seen
  within ~18 months so departed people age out (ownership filings carry no
  explicit departure signal). (2) The leadership sweep runs on its own monthly
  cadence (`LEADERSHIP_STALE_SECS`), slower than the weekly fundamentals /
  filings sweep, because leadership changes slowly and the ownership-XML sweep
  is the heaviest SEC work — kept comfortably within the guard's pacing and
  budget. (3) A real bug surfaced in verification: the `submissions` feed
  names a Form 4's primary document as an xsl viewer path
  (`xslF345X06/form4.xml`) that serves rendered HTML; the raw parseable XML is
  the bare filename, so `ownership_doc` strips to the basename. (4) The 8-K
  item-5.02 changes feed reuses the `filings` table — a new `items` column
  (migration `0006`) populated from the `submissions` `items` array — so it
  cost no new request. Names are stored as filed (last-name-first, caps) and
  title-cased for display. Deployed to production on 2026-05-22 via
  `git push server master` (commit `aea52ba`).
- **2026-05-22: two more side notes captured.** (1) The user wants data
  freshness — the age of displayed data — shown consistently across the whole
  app, the home page included, since it is critically important; budgeted as
  Phase 22. (2) The SEC User-Agent contact email should be the real
  `isaac@bythewood.me` on both local and the server, set via `.env`: the local
  `.env` already carries it, so no change there; the server's hand-written
  `.env` should be checked to match (a manual step — `.env` is not in the
  repo).
- **2026-05-22: Q4-in-quarterly-financials side note captured as Phase 23.**
  The user noticed the symbol page's quarterly financials table stops at Q3.
  Confirmed the cause: SEC XBRL has no discrete Q4 (no Q4 10-Q; the quarter
  lives only in the 10-K's full-year figure — zero `fiscal_qtr = 4` rows in
  `fundamentals`). Budgeted as Phase 23 — derive Q4 as FY - (Q1+Q2+Q3) for the
  flow metrics.
- **2026-05-22 — two financials-readability side notes captured as Phase 24.**
  While Phase 21 was in progress the user floated two refinements to the
  symbol page's fundamentals area: per-cell growth cues (a semantic color and
  a small up/down icon marking whether each figure improved or worsened
  against the prior period, year over year and quarter over quarter), and
  replacing the `·` middle-dot missing-value placeholder (which reads as a
  stray decimal) with a clearer em-dash-style mark. Budgeted as Phase 24 per
  the vibe-coding process rather than acted on mid-phase.
- **2026-05-22 — Phase 14 confirmed deployed; Phase 21 picked next.** The
  Status section briefly recorded Phase 14 as "not yet deployed"; on resuming,
  the server `master` was checked (`git ls-remote server master`) and found at
  commit `aea52ba`, the Phase 14 commit, so Phase 14 is in fact live in
  production. The plan's deployment notes were corrected. Asked which
  loose-ordered backlog phase to take next, the user chose Phase 21, Home &
  search refinements.
- **2026-05-22 — Phase 21 home & search refinements shipped.** Three
  independent refinements to already-shipped features. Design Q&A settled
  three points: (1) `RTY=F`, the Russell 2000 E-Mini, was added to the universe
  so `^RUT` also swaps to a future off-hours, alongside the clean
  `^SPX`/`^DJI`/`^NDX` swaps; `^NDQ` (Nasdaq Composite) and `^VIX` have no
  clean tradable future and keep the cash index. (2) The index-future swap
  shows whenever the regular cash session is closed (pre-market, after-hours,
  and the overnight/weekend Closed window), matching how the commodity cards
  already behave. (3) For add-symbol the user chose a fully synchronous
  backfill: `POST /api/symbols` pulls the new symbol's deep history and its
  entire SEC data (fundamentals, filings, leadership roster, or an ETF fund
  profile) inside the request before responding, accepting a longer request
  (the `RDDT` verification add took 51s) over deferring anything to a later
  scheduler cycle. Build calls: the home page's single sparkline row became
  two sections (`INDEXES` + `COMMODITIES` consts, `dashboard_cards` returning
  two lists, a shared `spark_cards_for`); `scheduler::backfill_symbol` reuses
  the existing store helpers and endpoint guards and is best-effort, so a
  guard denial only defers a piece to the normal sweep rather than failing the
  add; the search redirect is a 303 to `/s/{ticker}` on a single
  non-empty-query match. Verified end to end (see the Phase 21 Done entry).
  Not yet deployed; ships on the next push, and the boot seed adds `RTY=F` on
  the box.
- **2026-05-22 — Phase 21 confirmed deployed.** On resuming, server `master`
  was checked (`git ls-remote server master`) and found at commit `5cb5dc2`,
  the Phase 21 commit — so Phase 21 is in fact live in production, despite the
  entry above recording it as "not yet deployed" (the same plan/reality drift
  the Phase 14 entry hit). The Status section was corrected.
- **2026-05-22 — Phase 23 + 24 picked and built together.** Asked which
  backlog phase to take next, the user chose Phase 23 and Phase 24 as a single
  pass: both are small, self-contained refinements to the symbol page's
  financials area with no new data, and Phase 23's derived Q4 column and
  Phase 24's growth cues touch the same table. Design calls made during the
  build: (1) Q4 is derived per metric, not all-or-nothing — the `Q4-<year>`
  column appears if any of the four flow metrics derives, and a metric missing
  a quarter shows `—` in that column. (2) The growth cue's good/bad reading is
  a per-metric `Trend`: revenue / net income / EPS / dividend are `RiseGood`,
  total liabilities is `RiseBad`, and total assets and shareholder equity are
  `Neutral` — a rise in either can be debt-funded or a buyback, so they get an
  uncoloured directional arrow rather than a green/red verdict (the plan left
  the metrics beyond revenue / net income / liabilities as an open design
  question). (3) The cue compares each cell to the column immediately left of
  it (year over year in the annual table, quarter over quarter in the
  quarterly one); a gap on either side breaks the comparison. (4) The
  missing-value glyph fix was applied to all three copies of the placeholder
  constant (`routes/symbols.rs`, `compute.rs`, `templates.rs`), not only the
  one the plan named, since they all mean "no value" and a mix of `—` and `·`
  would be worse than the original; the em dash is now the single app-wide
  empty-value mark. Deployed to production on 2026-05-22 via
  `git push server master`.
- **2026-05-22 — Phase 22 picked, and a design pass settled it.** Asked which
  loose-ordered backlog phase to take next, the user chose Phase 22 (show data
  age everywhere). The plan had flagged it as needing a design pass on the
  wording and where the freshness rides; a Q&A settled two points. (1) On the
  list / grid surfaces (home, search) the freshness rides as **one quiet
  caption per section**, not one per card — a timestamp on each of the 8 + 8
  mover rows would clutter the Paper Ledger look; a per-section line stays
  skimmable. (2) **Mixed wording** — a relative "N ago" for live quotes and
  SEC syncs, an absolute clock time or date for daily and dated data — since a
  relative age on a daily close ("18h ago") reads vaguely where an absolute
  date ("May 21") is precise, while a relative age is exactly right for a
  quote you are watching tick.
- **2026-05-22 — Phase 22 show data age everywhere shipped (local).** A
  consistent, quiet data-freshness caption now rides across the whole app
  (home dashboard, search, every symbol-page data section), not only
  `/health`. Two new minijinja filters carry it: `asof` (epoch-ms -> an
  absolute market-clock anchor) and `shortdate` (a `YYYY-MM-DD` string -> a
  short date); the existing `ago` filter is reused for sync ages. Design
  calls made during the build: (1) the home Indexes / Commodities captions
  are the **only** ones that live-update — those sparkline cards stream, so
  their "as of" time sits in a `data-field` the stream client refreshes as
  quotes land; the movers and strongest / weakest panels are fixed page-load
  snapshots (per Phases 11 and 20), so their captions are static text, and
  the symbol header's quote age is reset to "just now" on each live quote.
  (2) An ETF's "Recent SEC filings" caption falls back from `filings_synced_at`
  to `fund_synced_at`: a stock's filings sweep stamps the former, but an ETF's
  filings are stored by the Phase 18 fund-profile sweep, which stamps the
  latter. (3) The `asof` filter renders an absolute clock time (not a relative
  age) for the section captions precisely so they never drift stale on screen
  while live prices tick; only genuinely relative things (a quote age, a sync
  age) use `ago`. No schema change, no new data source, no new network calls.
  Not yet committed or deployed; ships on the next `git push server master`.

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
- An ETF page (`/s/QQQ`) shows a Fund profile (AUM, holdings count, asset mix)
  and a Top holdings list, plus its SEC filing history; a commodity-trust ETF
  (`/s/GLD`) shows AUM and filings only, with no holdings.
- Phone (~360 px) and desktop are both fully usable: no unintended horizontal
  scroll, chart resizes, every feature reachable.
- `/` carries a "Strongest & weakest" pair of panels below the movers, and a
  strong / fair / weak standing badge rides on the movers, the search result
  cards, and above the symbol page's ratio cards (Phase 20).
- A stock page (`/s/AAPL`) shows a Leadership section: the current officer and
  board roster from SEC Form 3/4/5 ownership filings, plus a recent-changes
  list from 8-K item 5.02. An unsynced stock shows a pending note; ETFs and
  indexes show no Leadership section (Phase 14).
- A stock's Financials table (`/s/AAPL`, Quarterly) shows a derived Q4 column
  and per-cell up/down growth cues; a figure the company did not report shows
  an em dash (`—`), not a middle dot (Phase 23 + 24).
- Every data surface carries a quiet freshness caption (Phase 22): each home
  section and the search results show "prices as of ...", the symbol header
  shows the live quote's age, and each symbol-page section title shows its
  last SEC sync or close date. The home Indexes / Commodities captions
  live-update as streamed quotes land; the others are page-load values.
- No automated test suite or linter, matching the sibling projects.
