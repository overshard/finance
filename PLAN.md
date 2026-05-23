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

_Last updated: 2026-05-23_

**Current phase: Phase 17 (stock health read) complete, verified,
and deployed to production 2026-05-23 (commit `8a16b14`).** A single non-advice "health" read
per stock layered over the Phase 20 strength + trajectory composite: it
folds in a leadership-stability signal read off the recent 8-K item-5.02
change count (Phase 14). Industry context (Phase 15) was intentionally
dropped from this phase — the read ships without it and a later pass can
layer it on. Two surfaces. On the symbol page a new "Stock health" panel
sits between key stats and Fundamentals (stocks only): an overall
Healthy / Mixed / Concerning verdict plus three sub-readings —
fundamentals (Strong / Fair / Weak), trajectory (Climbing / Steady /
Slipping), leadership (Stable / Normal / Churning) with the actual
recent-change count. On the home dashboard a new "Stock health" pair of
panels (Healthiest / Most concerning) ranks the curated large-caps by the
composite, mirroring the Phase 20 strongest/weakest layout but with the
three sub-component chips visible on each row. Both surfaces carry a
quiet "for fun and reading at a glance, not investment advice"
disclaimer. Pure derivation from data already stored — no new network
calls, no schema change, no new `EndpointGuard` row. **Phase 16
(per-ticker anomaly feed) complete, verified,
and deployed to production 2026-05-23 (commit `a839737`).** A new symbol-page section between
Leadership and Recent SEC filings, surfacing notable recent events: large
daily price moves, drawdowns (new 6-month lows), large YoY fundamentals
changes, and 8-K item-5.02 leadership changes (reused from Phase 14). One
line per event — date, glyph, headline — newest first, capped to ~20 over
the past year. All instruments: stocks get all four event types; ETFs and
indexes and futures get price-only events. Selective thresholds (>5% AND
>2σ on price; ±25% YoY on fundamentals; new 6-month low on drawdown) so the
feed stays signal-dense. Pure derivation from data already stored — no new
network calls, no schema change, no new endpoint guard.

**Phase 30 (top picks + backtest) is complete, verified, and
deployed to production 2026-05-23 (commit `8ea9048`); a follow-up rework
shipped same day (see the decisions log) — not yet deployed.** Home page now carries a
"Top picks" panel — four columns (Day / Week / Month / Quarter), 5 ranked
stocks each, every row a verdict badge over a headline figure. A new
`/backtest` page replays the picker over historical prices and shows
strategy vs `^SPX` equity, total return, CAGR, per-pick and
per-period win rates, and a per-rebalance history table; horizon tabs
swap the rendered horizon in place. Stocks-only across all four
horizons (the user's design call); both win-rate definitions shown
side-by-side; one chart with horizon tabs (vs four side-by-side).
Migration `0009` adds the `picks` table; the scheduler's new `picks`
section snapshots forward each day right after `daily_close`. The
backtest is genuinely out-of-sample: at each rebalance the picker
grades a stock only against fundamentals that would actually have
been filed by then (latest annual whose period_end is at least 90
days before the rebalance — `models::FILING_LAG_DAYS`) and only
against closes up to that date — so a stock strong today but weak
in 2022 will grade weak in a 2022 rebalance. Migration `0010` cleared
stale `year`-horizon snapshot rows during the rework. The user is
explicit this is for fun, not financial advice. **Phase 28 (ETFs as first-class
citizens) is complete, verified, and deployed to production (commit
`2ae81d5`).** The new fund_metadata + sector/geography + ETF-distributions
data populates async via the scheduler's first sec / fund_metadata /
dividends cycles after boot. Phases 0
through 12 (the MVP) plus Phase 14 (company leadership), Phase 18 (ETF
profiles), Phase 20 (strongest & weakest home panels), Phase 21 (home &
search refinements), Phase 23 + 24 (financials table) and Phase 22 (data-age
captions) are complete, verified, and **live in production at
https://finance.bythewood.me**. Phase 22 adds a consistent, quiet data-age
caption across the whole app — the home dashboard, search, and every
symbol-page data section, not just `/health` — see the Done list and the
decisions log. Phase 26 (the user picked it as the next backlog phase)
adds a stock Dividends section on the symbol page — inferred cadence,
prior-year and YTD totals, a count-tempered on-track projection, and a per-
event payout history — backed by a new weekly Yahoo `dividends` scheduler
job. Deployed to production 2026-05-22 (commit `7608b06`). Remaining
post-MVP work: the loose-ordered Phase 13, 15, 19, 25, 27, 29
backlog (Phases 16, 17 and 28 are now done; 28 is shipped, 16 is
shipped, 17 is verified locally and pending deploy). Phase 28 (ETFs as first-class
citizens) was picked as the next backlog phase and scoped 2026-05-22 (see
the decisions log): one big phase covering all seven pieces (distributions,
expense ratio + yield, NAV / premium-discount, sector + geography, full
trailing returns, growth-of-$10k chart, strategy summary, benchmark
comparison). Phase 29 (issuer-direct ETF data feeds: iShares/BlackRock,
Vanguard, ...) was captured 2026-05-22 from a vibe-coding side note
mid-Phase-28; see its Phases-list entry.

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
- **Phase 17 stock health read.** Complete, verified, and deployed to
  production 2026-05-23 (commit `8a16b14`). A non-advice synthesis of the data this
  app already carries (fundamentals + price/growth trajectory + leadership
  stability), surfaced two ways: a "Stock health" panel on the stock
  symbol page and a "Healthiest / Most concerning" pair on the home
  dashboard. Industry context was intentionally dropped from this phase
  (Phase 15 is not built; see decisions log) — the read ships without it
  and a later pass can layer it on.
  - **Compute** added `HealthRead` and `health_read(ratios, closes,
    recent_changes)` in `compute.rs`, plus a `stability_grade` helper.
    HealthRead carries the overall grade + verdict, the composite score
    (for ranking), and three sub-component grades + labels — fundamentals
    (Strong / Fair / Weak, from `graded_mean` over the nine ratios),
    trajectory (Climbing / Steady / Slipping, from `trajectory_score`),
    and leadership stability (Stable / Normal / Churning, from a discrete
    band over the change count). Composite weighting: 0.55 strength /
    0.30 trajectory / 0.15 stability, renormalised over the components
    that landed so an unsynced leadership stat does not penalise the
    stock. Stability bands: 0-1 changes in the last 730 days reads
    Stable / +1.0; 2-3 Normal / 0.0; 4+ Churning / -1.0 — deliberately
    lenient since big companies routinely file ~one planned-succession
    5.02 a year. Each piece is pure; `LEADERSHIP_STABILITY_DAYS` and
    `MIN_GRADED` from Phase 20 stay the same gates.
  - **Symbol page wiring** in `routes/symbols.rs`. Hoisted the daily-close
    series out of the standing block so the new health read can share it.
    Added one small SELECT for the 8-K item-5.02 count over the last
    730 days (only fired once leadership has synced; `None` otherwise so
    the composite drops the stability term cleanly). `build_anomalies` and
    the existing Phase 20 standing computation were left intact — the new
    health read is additive, not a replacement.
  - **Symbol template** got a new `{% if health %}` panel between the key
    stats and Fundamentals (stocks only), introducing the section with a
    quiet section note + a `.disclaimer` line. Three `.health-row` rows
    carry the sub-component label / value / note; the per-row left
    border + value colour are semantically green / amber / red. The
    Phase 20 standing badge stays in the Fundamentals section above the
    ratio cards — it remains the per-ratio rollup; the new panel is the
    broader synthesis.
  - **Home page wiring** in `routes/home.rs`. `StockRow` gained
    `health: Option<HealthRead>` and a `leadership_synced_at` field.
    `load_stocks` got a fourth bulk query — one `GROUP BY ticker` of the
    8-K item-5.02 filings inside `LEADERSHIP_STABILITY_DAYS`, joined to
    `is_seeded` stocks — that feeds each stock's HealthRead. A new
    `health_panels()` ranks by composite score, takes the top and bottom 8,
    and scales each row's magnitude tint to the largest absolute score
    across both panels (mirroring the movers / standing tint maths).
  - **Home template** picked up a "Stock health" section above the
    existing "Strongest & weakest" section: section note + disclaimer +
    a Healthiest / Most concerning panel pair. Each row uses a new
    `health_row` macro and a new `.hrow` style: ticker, then the company
    name with three sub-component pills underneath (the same green /
    amber / red, smaller eyebrow type), then the overall verdict badge.
  - **SCSS.** New `.health` / `.health-row` block in `symbol.scss` (a
    three-column desktop grid that collapses to two-column on phones
    via a 480px media query, so the note wraps under the value); new
    `.hrow` / `.hrow__sub` / `.hrow__chip` block in `home.scss`. The
    shared `.section-note` style moved from `home.scss` to `base.scss`
    so the symbol page can use it too.
  - Verified: `cargo check` + `bun run build` clean. `/s/AAPL` reads
    `Mixed` overall — Fair fundamentals, Climbing trajectory, Churning
    leadership at 7 reported changes in the last 2 years; the
    Fundamentals section below still carries its own `FAIR` standing
    badge. `/s/SPY` (ETF) and `/s/^SPX` (index) hide the section cleanly.
    `/` renders the Stock health panel above Strongest & weakest with
    GOOGL, GOOG and MU leading the healthiest list. Desktop (1280px)
    and phone (390px) both render with no horizontal overflow; zero
    console errors on the home page or any symbol page checked.
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

- **Phase 16 per-ticker anomaly feed.** Complete, verified, and deployed
  to production 2026-05-23 (commit `a839737`). The same `git push server
  master` shipped the Phase 30 rework (`year` → `quarter`, true
  out-of-sample backtest) and the S&P 500 universe expansion alongside. A new "Notable recent events" section on
  the symbol page between Leadership and Recent SEC filings, surfacing four
  kinds of dated events in one merged list — one line per event, date ·
  glyph · headline, newest first, capped at 20 over the past year. Pure
  derivation from data already stored: no new network calls, no migration,
  no new `EndpointGuard` row.
  - **Compute** added two new pure helpers in `compute.rs`. `price_anomalies(closes, dates)`
    walks the trailing year and emits an event for every bar whose
    close-to-close return is both `>5%` in magnitude AND `>2σ` against
    the prior 90-day daily-return σ — the dual threshold keeps a low-vol
    name's modest move and a high-vol name's normal daily wobble out of
    the feed. `drawdown_anomalies(closes, dates)` emits one event each
    time the close prints a fresh 6-month low, with a 30-bar cooldown so
    a long slide does not stream daily; the headline carries the drop
    from the trailing window's peak. Both share a new `AnomalyEvent`
    struct (date, glyph, headline, optional url, severity).
  - **Models** added `fundamentals_anomalies(facts)` in `models.rs`,
    matching the existing `latest_annual_inputs` pattern (this helper
    walks `FundFact` directly so it lives in models, not compute).
    Emits one event per (annual fiscal year, metric ∈ {revenue,
    net_income}) whose YoY change exceeds ±25%, dated at the fiscal
    year's `period_end`. Headlines read "FY2026 revenue +65% YoY".
  - **Symbol route** hoists the stock `fundamentals` SELECT one level
    so both the Fundamentals section and the new `build_anomalies`
    aggregator share the same fact slice. The aggregator merges the
    price + drawdown + fundamentals streams, plus a small 8-K item-5.02
    SELECT (reused from Phase 14's leadership feed but constrained to
    the past 365 days), trims to the past-year window, sorts newest
    first with severity as the tiebreaker, and caps at 20. Returns
    `None` when no events qualify so the template hides the section.
  - **Template** in `symbol.html` renders one panel between Leadership
    and the ETF block (which sits before Filings on a stock page);
    `{% if anomalies %}` gates the whole section. Each row is a date,
    a glyph mapped per `e.glyph` (↑ ↓ for price moves, ↡ for drawdown,
    + − for fundamentals, ❖ for leadership), and a headline; a
    leadership row links to its EDGAR url. A quiet italic provenance
    line below the list says where the events come from and labels the
    section as not investment advice.
  - **SCSS** added a small `.anomalies` block matching the
    `.lead-change` row style — semantic-only colour on the glyph
    (using the existing `--up`/`--down` tokens), monospace date in
    `--ink-faint`, headline in `--ink`.
  - Coverage: all four event types for stocks; ETFs and indexes get
    price + drawdown only (no SEC fundamentals/leadership data to
    derive from). Futures (with no `daily_prices`) and historyless
    indexes like `^VIX` (also no `daily_prices`) get no events at all
    and the section hides for them.
  - Verified: cargo + bun build clean; the four new compute unit
    tests pass (spike-triggers-on-5%-AND-2σ, ignore-1%-even-at-2σ,
    fresh-6mo-low-flags, slide-dedupes-to-≤5). `/s/NVDA` renders 10
    events in the past year (a balanced mix: +5.8%/+5.6%/+7.9%/+5.8%
    one-day moves, a -5.5% downside move, a -19% new 6-month low,
    and FY2026 revenue + net income +65% YoY). `/s/AAPL` shows 5
    leadership-change rows and no price/drawdown/fundamentals events
    (a fair read: AAPL has been calm, single-digit growth). `/s/SPY`
    (ETF) shows 1 drawdown; `/s/^SPX` (index) shows 1 drawdown;
    `/s/GC=F` (future, no `daily_prices`) hides the section entirely.
    Order on `/s/NVDA` is Leadership → Anomalies → Filings (verified
    via DOM offsets). Desktop (1280px) and phone (390px) both render
    with no horizontal overflow and zero console errors.

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

- **Phase 22 show data age everywhere.** Complete, verified, and deployed to
  production. A consistent, quiet data-freshness caption now rides
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

- **Phase 28 ETFs as first-class citizens.** Complete, verified, and
  deployed to production (commit `2ae81d5`). ETFs now read as densely as a stock page: a new "About
  this fund" panel (expense ratio, distribution yield, NAV with live
  premium / discount, inception, category, fund family, strategy
  paragraph), a trailing-returns table (1m / 3m / YTD / 1y / 3y / 5y /
  10y / since-inception, annualised past one year), a growth-of-$10,000
  area chart over the longest available range, sector and geography
  exposure panels alongside the existing asset mix, a relative-performance
  benchmark line on the price chart for the broad-market ETFs, and the
  Phase 26 Dividends section lifted from stocks-only to also cover ETF
  distributions.
  - **Migration `0008`** adds `symbols.fund_metadata_synced_at` and
    `symbols.benchmark`, `fund_profiles.sector_mix` and `geography_mix`
    JSON columns, and a new `fund_metadata` table (expense ratio, yield,
    NAV, inception, category, fund family, strategy summary).
  - **Yahoo `quoteSummary`** is now the source for a third concern beyond
    quotes and dividends: a new `YahooProvider::fund_metadata` calls
    `v10/finance/quoteSummary` with the `fundProfile + defaultKeyStatistics
    + summaryDetail + price + assetProfile` modules and parses each into
    optional fields, so a small ETF that Yahoo only partly covers still
    populates as much as it can. 429 / 503 / 401 / 403 all surface as the
    typed `RateLimited` so the `yahoo` `EndpointGuard` trips at once if
    Yahoo crumb-gates the v10 endpoint for our box.
  - **N-PORT parser extended** to capture each holding's `<issuerCat>`
    and `<invCountry>` while it streams, then aggregate into sector and
    geography mixes in the same `[[label, percent], ...]` JSON shape as
    the existing `asset_mix`. A small bucket map turns the bare codes
    into human labels (`CORP` -> Corporate, `US` -> United States, ...).
    A degenerate mix (a pure-equity ETF rolls everything up to
    Corporate) hides itself in the template.
  - **Scheduler `run_fund_metadata`** is a new section on the existing
    `yahoo` `EndpointGuard` — one request per ETF, monthly staleness on
    `fund_metadata_synced_at`, brought forward to the first tick on boot
    the same way `sec` and `dividends` are, so a deploy backfills the
    universe within a tick rather than the daily interval. The Phase 26
    `dividends` job dropped its `kind = 'stock'` filter so ETF
    distributions ride the same code path. `scheduler::backfill_symbol`
    pulls the new ETF's fund_metadata and distributions inside the
    add-symbol request, mirroring Phase 21's intent.
  - **Compute** added `trailing_returns(bars, today)` (eight periods,
    annualised past 1y; "since inception" uses actual days / 365.25 for
    leap-year drift), `growth_of_10k(bars)` (scale the close series so
    the anchor bar reads as $10k), `premium_discount_pct(price, nav)`
    and a small `premium_grade` band (±0.25 / ±1.00). Unit-tested.
  - **Routes** added a new `/api/symbols/{ticker}/growth` endpoint
    returning the fund's full-history growth series plus a benchmark
    growth series anchored to the fund's first bar (separately
    re-scaled to $10k from there). The existing
    `/api/symbols/{ticker}/history` gained a `benchmark` series scaled
    to the fund's first visible close so the two lines start together
    on the main chart. Trailing returns pull the full daily history
    rather than the 400-bar window the chart uses.
  - **Frontend** added a dashed benchmark line series to the main
    lightweight-chart, a benchmark toggle button that only renders
    when one is configured, and a new `growth.js` driving an
    AreaSeries + dashed LineSeries growth-of-$10k chart against its
    own y-axis (compact-USD formatter).
  - **Template** restructured: the Dividends block moved out of the
    stocks-only conditional (its label flips to "Distributions" on an
    ETF), and the ETF block grew About / Trailing returns / Growth-of-$10k
    panels above the existing Fund profile and Top holdings. Sector and
    geography mixes ride alongside the asset mix inside Fund profile.
  - **Universe `starter.csv`** added a fifth `benchmark` column.
    Curated for 18 broad-market ETFs (SPY/VOO/IVV/VTI/VUG/VTV/SCHD/VIG/
    VYM/XLE/XLF/XLK/XLV -> `^SPX`; QQQ/SMH/ARKK -> `^NDX`; DIA ->
    `^DJI`; IWM -> `^RUT`). Sector SPDRs map to `^SPX`. International,
    bond and commodity ETFs leave it blank (no clean equity-index
    benchmark). The `seed` parser threads the column into
    `symbols.benchmark` on every seed pass.
  - **Health page** lists the new `fund_metadata` job between `sec` and
    `dividends` (job_meta + job_rank); the `dividends` job's description
    now reads "stock and ETF" rather than "stock".
  - Verified locally: cargo + bun build clean; the 3 Phase 28 compute
    unit tests pass; migration `0008` applied cleanly on the seeded
    dev DB; `/s/SPY` renders the new sections (with an injected
    synthetic `fund_metadata` row for the dev box, since Yahoo
    rate-limits this WSL2 IP); `/api/symbols/SPY/history?range=1Y`
    returns a 252-point benchmark series scaled to the fund's first
    visible close; `/api/symbols/SPY/growth` returns a 5,342-point
    fund growth series ($10k -> $79,275 over 21 years, ~10.3%/yr) plus
    the same length benchmark series; `/s/GLD` (commodity trust)
    shows the About / Returns / Growth panels with no holdings (the
    grantor-trust note carries); `/s/AAPL` (stock) shows none of the
    ETF panels; `/s/^SPX` (index) and `/s/GC=F` (future) are
    unchanged. SPY trailing returns sanity-checked against deep
    history: 10y +320.6% / +15.45%/yr, since-inception (from the
    Feb 2005 first stored bar) +692.8% / +10.24%/yr.

- **Phase 30 top picks + backtest.** Complete, verified, and deployed
  to production 2026-05-23 (commit `8ea9048`); reworked same day to
  replace the `year` horizon with `quarter` and to compute per-
  rebalance historical standings (see the decisions log). A home-page
  "Top picks" panel of 5 forecast-horizon picks per horizon
  (Day / Week / Month / Quarter), and a new `/backtest` page that
  replays the picker over historical prices.
  Stocks-only across all four horizons per the user's design call; one
  chart with horizon tabs on the backtest page; both per-pick and
  per-period win rates surfaced. For fun and testing — explicitly not
  financial advice, a quiet disclaimer rides on both surfaces.
  - **Pick math (`compute.rs`)** — four pure `pick_*` rankers, each
    taking the shared `PickInput` (last price, prev close, daily closes,
    Phase 20 standing) and returning an `Option<f64>` (the headline
    figure that justified the pick, `None` when disqualified):
    - **Day:** today's intraday % move + a bias for sitting near the
      52-week high; skips stocks the standing rates `Weak`.
    - **Week:** trailing 5-day % return, gated on RSI(14) being in
      `[30, 70]` and the close being above SMA50; same `Weak` filter.
    - **Month:** trailing 20-day % return, gated on the close being
      above SMA200; same `Weak` filter.
    - **Quarter:** trailing ~63-day (one earnings cycle) % return,
      gated on the close being above SMA200; same `Weak` filter.
      (Originally **Year**, a pass-through of the Phase 20 standing —
      replaced same day; see the decisions log.)
  - **Picks module (`src/picks.rs`)** glues the rankers to the DB:
    `HORIZONS` const, `compute_picks(bundles)` runs each ranker against
    every stock and returns one `PickSlate` (the 5 top per horizon),
    `load_bundles(pool)` builds the per-stock bundles in 3 queries (the
    same shape as `routes::home::load_stocks`), `snapshot_today(pool,
    date)` writes the result into the `picks` table.
  - **Migration `0009`** adds the `picks` table:
    `(snapshot_date, horizon, rank, ticker, score, price_at_pick)`, PK
    on the first three. One row per pick, replaced wholesale on each
    snapshot date (idempotent reruns). Frozen forward from the first
    deploy so the backtest reads immutable history, not today's algo
    replayed over old data (which an algo tweak would silently rewrite).
  - **Scheduler** gained a `picks` section right after
    `run_daily_close_if_due`. Keyed in `meta` on `picks_snapshot_date`
    so it fires exactly once per ET trading date, and gated on
    `daily_close_date` already being set so the picks are scored off
    fresh closes. Logs to `fetch_log` + flips `data_status` like every
    other job; surfaces on `/health` for free.
  - **Home page** (`routes/home.rs` + `home.html`) carries the "Top
    picks" section between Today's movers and Strongest & weakest.
    Computed live every render (cheap: the new pick scan + the existing
    standings scan together still come in under 600ms warm); a fixed
    page-load snapshot like the movers and standings panels, so the
    stream client does not stall on it.
  - **`/backtest` page** (`routes/backtest.rs` + `backtest.html` +
    `frontend/static_src/backtest/`): one page, four horizon tabs that
    swap content in place via `GET /api/backtest?horizon=…`. The JSON
    feed runs `picks::run_backtest`, which walks back from today by
    horizon stride (1/5/20/63 trading days), at each rebalance picks
    the top 5 with the same rankers, equal-weights them for one stride,
    rebalances at the next stride, and tracks both the strategy's and
    `^SPX`'s equity from a $10k anchor. Renders an equity curve area
    chart (lightweight-charts, mirroring the Phase 28 growth chart),
    four stat cards (strategy total + CAGR, benchmark total + CAGR,
    per-pick win rate, per-period win rate), and a per-rebalance
    history table with each period's picks color-tinted by their own
    return.
  - **Out-of-sample standings.** At each rebalance date the backtest
    grades a stock against the latest annual whose `period_end + 90
    days ≤ rebalance` (a conservative SEC filing-lag cushion —
    `models::FILING_LAG_DAYS`) and against closes sliced to that date.
    `HistBundle` carries raw `FundFact`s; `rank_at` calls
    `models::latest_annual_inputs_as_of` then `compute::standing` per
    rebalance, so a stock weak in 2022 grades weak in a 2022
    rebalance and the year-over-year picks genuinely diverge.
  - **Disclaimer style** in `base.scss`: a quiet ink-faint italic line,
    smaller than body text, never carries semantic color. Used on both
    the home Top picks panel and the backtest page header.
  - **Performance**: the backtest load query was capped at the trailing
    7 years (`HIST_LOOKBACK_DAYS`) so deep histories (`^SPX` back to
    1789) do not pull a million-row scan; cold-cache `/api/backtest`
    runs ~1-2s per horizon, warm ~100ms.
  - Verified: cargo + bun build clean; `/` renders the four-column
    Top picks section on desktop (4-wide) and phone (2x2), every row
    carrying a verdict badge over a percent return; `/backtest` loads
    the Month horizon by default with the equity curve drawing the
    strategy and `^SPX` from $10k. After the same-day rework the
    Quarter horizon's local replay (2021-08 → 2026-02, 19 rebalances)
    surfaced genuinely varying picks per period: 2021 H2 was tech
    (NET, DDOG, NVDA, AMD, MRVL); 2022 H1 rotated to defensives
    (CVX, LMT, PM, MO); MU appeared in exactly one 2022 period (where
    its trailing-60d momentum and as-of fundamentals warranted entry)
    and the backtest honestly recorded the -19.8% exit — a stark
    contrast to the pre-rework behaviour where MU was top-5 in every
    year by construction. Phone view stacks the stat cards 2x2 and
    the rebalance table scrolls inside its card. Tabs swap horizons
    in place without a navigation. Zero console errors.

- **Phase 26 dividend payouts.** Complete, verified, and deployed to
  production (commit `7608b06`). Per-payout dividend history sourced from Yahoo's chart endpoint
  via `events=div`, surfaced as a new Dividends section on the symbol page
  between Fundamentals and Leadership. Stocks only — ETFs, indexes and
  futures show none of it.
  - **Migration `0007`** adds the `dividends` table keyed `(ticker, ex_date)`
    with a per-share `amount`, plus `symbols.dividends_synced_at`. Five-year
    history pulled per stock (the user's pick over `max` or `5y` shorter
    windows: enough for prior-year + YTD pace and a long visible list, while
    keeping the per-call payload modest).
  - **Yahoo provider.** A new `YahooProvider::dividends(ticker)` inherent
    method calls the same v8 chart endpoint with `interval=1d&range=5y&events=div`
    and parses the `events.dividends` map. Like `lookup`, a 429/503 surfaces
    as the typed `RateLimited` (the `yahoo` `EndpointGuard` trips at once);
    a 404 or `chart.error` returns an empty list (a clean "no dividends
    history" answer, not a guard failure). Non-positive amounts and unparsable
    timestamps are filtered.
  - **Scheduler.** A new `dividends` job sweeps every stock whose
    `dividends_synced_at` is older than a week, paced through the existing
    `yahoo` guard alongside intraday and daily-close. Daily due-check;
    skipped wholesale on a no-stale run. Brought forward to the first tick
    on boot, like `sec`, so a deploy adding the table backfills the universe
    within a tick rather than the daily interval. Resumable: each stock's
    `dividends_synced_at` is stamped only on success.
  - **Pace math (`compute.rs`).** `infer_cadence` reads the median gap
    between the last up-to-8 payouts to classify a stock as monthly /
    quarterly / semi-annual / annual / irregular. `dividend_pace` builds a
    `DividendPace` carrying prior-year + YTD totals, the inferred cadence's
    caption, and a count-tempered projection: `YTD × expected_n /
    declared_n_so_far` (the user picked this over an elapsed-fraction-of-year
    projection, since it does not misread a quarterly payer just after a
    payout). The on-track grade is `Good` / `Ok` / `Bad` on a ±2% flat band,
    rise-is-good (matching the Phase 24 trend reading).
  - **Symbol page.** A new "Dividends" section between Fundamentals and
    Leadership (the user's slot pick). Header: the cadence caption + the
    on-track verdict pill. Below: a 3-card pace row — prior-year total, YTD
    so far (with a payment count), and the count-tempered projection (with a
    coloured `+x.x% vs <prior year>` sub). A provenance note labels the
    projection as an estimate between payouts. Below that, a per-event
    history list (date + per-share amount, newest first). A stock that has
    not been swept yet shows a pending note; a swept stock with no payouts
    in the past five years hides the section entirely (it pays no dividend).
  - **Add-symbol backfill** (`scheduler::backfill_symbol`) now also pulls a
    new stock's dividend history before responding, so a user-added stock's
    Dividends section is complete the moment the add returns.
  - **Health page** lists the new `dividends` job between `sec` and
    `intraday` (job_meta + job_rank). The job runs on the existing `yahoo`
    endpoint guard, so no new guard row was needed.
  - Currency formatting: per-share figures show as `$0.24` to the cent
    normally and widen to `$0.0625` for sub-cent payouts (the monthly REIT
    case), so a small payment is not lost to rounding.
  - Verified: cargo + bun build clean; the boot sweep ran and stamped every
    curated stock; `/s/AAPL` rendered the Dividends section with the
    quarterly cadence caption, prior-year + YTD totals, the projection with
    its on-track badge, and the per-event history list; a non-paying stock
    hid the section; ETFs / indexes / futures showed no section.

**Resuming, next action**
**Phase 28 (ETFs as first-class citizens) is complete and deployed**
(commit `2ae81d5`, 2026-05-22). The new data populates async via the
scheduler's first sec / fund_metadata / dividends cycles after boot:
SEC re-parses N-PORT to fill `fund_profiles.sector_mix` +
`.geography_mix` for the 28 ETFs; the new `fund_metadata` Yahoo job
populates the `fund_metadata` table for the same 28; the lifted
`dividends` sweep now covers ETF distributions alongside stock
dividends. Watch `/health` for any job that goes red. Scope settled
2026-05-22 (see the decisions log): one big phase covering all seven
pieces (distributions for ETFs, expense ratio + yield via Yahoo
`quoteSummary`, NAV / premium-discount, sector + geography exposure from
N-PORT, full trailing returns 1m/3m/YTD/1y/3y/5y/10y/since-inception,
growth-of-$10k chart, strategy summary + inception, and a benchmark
comparison off a hand-curated `benchmark` column for the curated universe).
A new migration `0008` will add fund-metadata columns to `fund_profiles`
(expense ratio, yield, NAV, inception, category, fund family, strategy
summary, sector_mix and geography_mix JSON) and a `benchmark` column on
`symbols`; a new Yahoo `quoteSummary` provider path adds
`YahooProvider::fund_metadata`; the N-PORT parser is extended to retain
each holding's `industryCode` and country; a new scheduler `fund_metadata`
section sweeps stale ETFs on the existing `yahoo` guard (monthly cadence).
Phase 26's `kind = 'stock'` dividends filter is dropped so ETF
distributions ride the same code path. Phase 29 (issuer-direct ETF data
feeds: iShares/BlackRock, Vanguard, ...) was captured 2026-05-22 from a
vibe-coding side note mid-Phase-28; see the decisions log.

Phase 30 (top picks + backtest) is complete, verified, and deployed to
production 2026-05-23 (commit `8ea9048`). Remaining post-MVP work is
the loose-ordered Phase 13, 15, 16, 17, 19, 25, 27, 29 backlog. Phase 26 (dividend payouts)
is complete and deployed (commit `7608b06`); the MVP plus Phase 14,
Phase 18, Phase 20, Phase 21, Phase 23 + 24, Phase 22 and Phase 26 are all
live at https://finance.bythewood.me. There is still no GitHub repo for
finance: the user deferred that; if one is created later, add it as
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
- [x] **Phase 16: Per-ticker anomaly feed.** Complete, verified, and
  deployed to production 2026-05-23 (commit `a839737`). See the Phase 16
  Done entry in Status and the decisions log. (Picked as the next backlog phase and
  scoped 2026-05-23 — see decisions log.) On the symbol page, a
  feed of notable recent events for that one ticker: large changes in its
  fundamentals, leadership changes, and unusually large price moves or
  drawdowns. Builds on Phases 7 and 14.
  Pieces:
  (1) **Three compute helpers in `compute.rs`**, each pure over data
  already stored. `price_anomalies(closes, dates)` walks the trailing 1y
  bars, computes a trailing 90-day rolling standard deviation of daily
  returns, and emits an event when `|move| > 5%` and `|move| > 2σ`.
  `drawdown_events(closes, dates)` flags each day a stock prints a new
  6-month low (no event while still in drawdown, to avoid a daily stream
  in a long slide). `fundamentals_events(facts)` walks the latest two
  annual figures for revenue and net income and emits an event when the
  YoY change exceeds ±25%.
  (2) **Leadership events** ride the 8-K item-5.02 SELECT Phase 14 already
  runs in `build_leadership` — `routes/symbols.rs` reads them once,
  reshapes each as an `AnomalyEvent` with the existing EDGAR url + a "8-K
  item 5.02 leadership change" headline. No new SQL.
  (3) **`AnomalyView` aggregator** in `routes/symbols.rs`: merge the four
  feeds, sort newest-first, cap at ~20 over the past 1y. Stocks get all
  four; ETFs / indexes / futures get only piece (1) + (2-price-side).
  (4) **Template section in `templates/pages/symbol.html`** between
  Leadership and Recent SEC filings. One row per event: date glyph
  headline. Match the Phase 14 lead-changes feed visually so the page
  reads consistently. Section hides cleanly on an empty feed.
  Pure derivation: no schema change, no new network calls, no new
  `EndpointGuard` row.
- [x] **Phase 17: Stock health read.** Complete, verified and deployed
  to production 2026-05-23 (commit `8a16b14`) — see the Phase 17 Done
  entry in Status and the decisions log. Synthesizes fundamentals + price/growth
  trajectory + leadership stability into a single non-advice "health"
  read, on the stock symbol page and as a Healthiest / Most concerning
  pair on the home dashboard. Industry context (Phase 15) was
  intentionally dropped from this phase since Phase 15 is not built; the
  read ships without it and a later pass can layer it on. The leadership
  signal is a stability score read off the count of recent 8-K item-5.02
  filings (the same Phase 14 data already on the page); the user-picked
  scope was "stability via churn count" over a qualitative note. Builds
  on Phases 7, 14 and 20.
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

- [x] **Phase 22: Show data age everywhere.** Complete, verified and deployed
  to production 2026-05-22. See the Phase 22 Done entry in Status and the
  decisions log. (Captured 2026-05-22 from a
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

- [ ] **Phase 25: Earnings dates.** (Captured 2026-05-22 as a vibe-coding
  side note.) Surface a stock's earnings rhythm on the symbol page and the
  chart. Pieces:
  (1) A small section showing the most recent earnings date with "N days
  ago", the next expected earnings date with "N days from now", and a short
  list of recent past earnings dates. Past dates come from the `filings`
  table for free: 8-K item 2.02 (Results of Operations and Financial
  Condition) is the SEC earnings press release, and Phase 14 already stores
  8-K item codes in `filings.items`.
  (2) The next expected date needs a forward source. Two options to settle
  when built: Yahoo's `quoteSummary` calendarEvents module (a new path on
  the yahoo provider, gated by the existing `yahoo` `EndpointGuard`), or
  estimate it from the prior year's 8-K 2.02 cadence (no new endpoint, less
  reliable when a company moves its reporting date).
  (3) Earnings markers on the candlestick chart: a small pip or vertical
  guide drawn on the bar for each past earnings date, so a large move that
  followed an earnings print is explained at a glance. Uses
  lightweight-charts' series-markers API; the dates come from the same 8-K
  2.02 filings. Pip styling stays in the Paper Ledger ink palette, outside
  the semantic green / amber / red set.
  Stocks only (ETFs, indexes and futures have no earnings). A stock without
  an SEC sync yet shows nothing extra. No schema change for pieces (1) and
  (3); piece (2) may want a small `next_earnings_at` column on `symbols`
  if it takes the Yahoo calendar path.

- [x] **Phase 26: Dividend payout history and pace.** Complete, verified,
  and deployed to production 2026-05-22 (commit `7608b06`); see the
  Phase 26 Done entry in
  Status and the decisions log. Per-payout dividend history from Yahoo
  chart `events.dividends`, new `dividends` table (migration `0007`),
  a weekly `dividends` scheduler job on the existing `yahoo` guard, and a
  symbol-page Dividends section between Fundamentals and Leadership with
  inferred cadence, prior-year + YTD totals, a count-tempered on-track
  projection, and a per-event history list. Ex-div pips on the candlestick
  chart deferred to Phase 25 per the design Q&A. Stocks only.

  (Captured 2026-05-22
  as a vibe-coding side note alongside Phase 25.) On the symbol page,
  surface the dividend cadence and how the current year is tracking against
  the last. Pieces:
  (1) A dividend history list: the per-share amount of each payout and its
  ex-dividend or payment date, newest first. Source: Yahoo's chart endpoint
  carries an `events.dividends` series alongside the price bars, with an
  ex-div timestamp and an `amount` per event. The existing yahoo provider
  fetches the chart for quotes already; this adds the `events` query
  parameter and a small parser. SEC XBRL's quarterly `DividendsPerShare`
  facts (already stored in `fundamentals`) are per fiscal period, not per
  payout date, so they do not stand in.
  (2) Calendar-year totals: total paid per share in the previous calendar
  year, and total YTD in the current year, both summed from the dividend
  events.
  (3) An "on track" pace read: project the current year's total by scaling
  YTD by the elapsed fraction of the year, compare to last year's total,
  and report the % change with a semantic green / amber / red badge (rise
  is good for dividends, matching the Phase 24 trend reading). Caveat: a
  company with quarterly or semi-annual cadence looks ahead or behind pace
  between payouts; either temper the projection by counting declared
  payments vs the prior year's count, or label it conservatively.
  (4) Ex-div pips on the candlestick chart, reusing the Phase 25 marker
  layer with a different glyph or color so an earnings event and a dividend
  event are distinguishable.
  Stocks only. Schema: a `dividends` table keyed by (ticker, ex_date) with
  the per-share amount, populated by a new scheduler section that pulls
  each stock's dividend history through the `yahoo` `EndpointGuard` on a
  slow cadence (weekly is enough; the data rarely changes).

- [x] **Phase 28: ETFs as first-class citizens.** Complete, verified, and
  deployed to production 2026-05-22 (commit `2ae81d5`). See the Phase 28 Done entry in
  Status and the decisions log. (Captured 2026-05-22 from a vibe-coding
  side note immediately after Phase 26 shipped. Picked as the next backlog
  phase and scoped 2026-05-22.) The user's
  steer, verbatim: *"I really need ETFs treated as first class citizens
  like with as much data as possible, AUM, cashflow maybe, dividends,
  everything — right now we are ignoring a lot of stats for ETFs and
  treating them as second class."* The goal: an ETF symbol page that
  reads as densely and informatively as a stock's, not the smaller card
  it currently is.

  **What an ETF page carries today (the gap):** Phase 18 ships the fund
  profile (AUM via N-PORT, holdings count, top-25 holdings, asset-class
  mix) and the SEC filings list. Phase 26's Dividends section was stocks
  only and skips ETFs. There is no expense ratio, no distribution yield,
  no NAV / premium-discount, no sector or geography breakdown, no return
  metrics, no inception date or strategy summary, no benchmark comparison.

  **Scope settled 2026-05-22 (see the decisions log).** One big phase
  covering all seven pieces below in a single ship. Open questions answered:
  expense ratio + yield come from Yahoo `quoteSummary` (path 2a), not
  hand-curated; benchmark comparison uses a hand-curated `benchmark`
  column on `symbols` (path 7's pragmatic option), so user-added ETFs
  simply omit the overlay; the full trailing-returns set, the growth-of-
  $10k chart, NAV / premium-discount, and sector + geography panels are
  all in. Phase 29 (issuer-direct ETF feeds for iShares / Vanguard / ...)
  was budgeted as a separate later phase, not folded in here.

  **Planned pieces:**

  (1) **ETF distributions.** Lift Phase 26 to cover ETFs — Yahoo's
  `events.dividends` series carries an ETF's distributions the same way
  it carries a stock's dividend, so the existing `YahooProvider::dividends`
  and the `dividends` scheduler job already work for ETF tickers. Drop
  the `kind = 'stock'` filter in `run_dividends` and `backfill_symbol`'s
  dividend step, and surface the same Dividends section on the ETF page.
  Cheapest piece; ships almost for free.

  (2) **Expense ratio and yield.** The two figures most consumers ask of
  an ETF. Not in SEC structured data (the wall Phase 18 hit). Source
  options: (a) Yahoo's `quoteSummary` endpoint, modules
  `fundProfile` + `defaultKeyStatistics` + `summaryDetail`, which carry
  `annualReportExpenseRatio`, `yield`, `trailingAnnualDividendYield`, fund
  family, category, inception date and the strategy summary in one
  request — a new path on the `yahoo` provider behind the existing guard.
  (b) Hand-curated values in `universe/starter.csv`. Phase 18 considered
  (a) and dropped it; revisit it here, because hand-curation does not
  scale to user-added ETFs.

  (3) **NAV and premium / discount.** An ETF's market price drifts from
  its true net-asset-value intraday — a fresh metric the app does not
  carry. Yahoo serves `navPrice` in the chart `meta` (or via the
  `quoteSummary` price module); compute `(price - nav) / nav * 100` and
  display the day's premium / discount with a quiet good / ok / bad band
  (a persistent large premium is a yellow flag, a small discount can be
  a buying opportunity). A small chart of historical premium / discount
  is a stretch goal.

  (4) **Sector and geography exposure.** N-PORT carries each holding's
  `industryCode` (the standard SOC / GICS-ish code field) and country of
  issuer; aggregate them into "Sector exposure" and "Geographic exposure"
  panels alongside the existing asset-class mix bar. Pure derivation
  from the per-holding data the Phase 18 parser already streams past
  (right now it keeps only the top-25 issue + amount); no new network
  call, but the N-PORT parser needs to keep these fields as it streams.

  (5) **Return metrics.** Trailing returns 1m / 3m / YTD / 1y / 3y / 5y /
  10y / since inception, computed from `daily_prices` which we already
  have for ETFs. Pure `compute.rs` work, lots of small displays. Annualize
  for periods over a year. A "growth of $10,000" chart over the longest
  available range is a strong skim metric.

  (6) **Strategy summary + inception date + category.** From Yahoo's
  `quoteSummary.fundProfile.longBusinessSummary` (a paragraph the issuer
  writes) plus `firstTradeDateEpochUtc` in chart `meta`. A small "About
  this fund" panel below the profile.

  (7) **Benchmark comparison.** Most ETFs track an index (`SPY`/`VOO`
  -> `^SPX`, `QQQ` -> `^NDX`, etc.). A small relative-performance line
  on the chart over the visible range would read at a glance whether the
  fund is tracking or drifting. Hard part: knowing the benchmark — Yahoo
  occasionally carries it but inconsistently; an optional `benchmark`
  column on `symbols` filled by hand for the curated universe is the
  pragmatic path.

  **Schema implications:** likely new `etf_stats` table (or extra columns
  on `symbols`) for the slow-moving figures — expense ratio, yield,
  inception date, category, fund family, benchmark, strategy summary —
  populated by a new scheduler section on the `yahoo` guard, monthly
  cadence (these change rarely). Sector / geography mixes can ride in
  JSON columns on `fund_profiles` like `asset_mix` already does.

  **Dependencies:** none hard. Piece (1) is a small detour on Phase 26's
  code. Pieces (2)+(6) need the `quoteSummary` endpoint added to
  `YahooProvider`. The user did not specify ordering — pick at build
  time, or split into 28a (distributions + expense ratio + yield, the
  table-stakes ETF figures) and 28b (NAV / sector / returns / benchmark,
  the richer analytics).

  **Anti-spam:** every new fetch rides the existing `yahoo` /
  `sec` `EndpointGuard`. No new endpoint guards required.

- [ ] **Phase 27: Backup providers for redundancy.** (Captured 2026-05-22
  from a vibe-coding side note while Phase 26 was mid-build.) For each
  data concern (history / live quotes / fundamentals / dividends),
  configure one or more *backup* upstreams behind the existing provider
  traits, and switch over to the backup whenever the primary is unhappy:
  its `EndpointGuard` breaker has opened, its hourly budget is spent, or
  the primary returned a transport error. The user considers this critical
  for keeping the app up when an upstream blocks, rate-limits, or simply
  has an outage — "as much redundancy as possible".
  Open design points to settle when building:
  (1) **Trait composition.** A small `MultiProvider<T: Provider>` wrapper
  that holds a primary + ordered fallbacks; `acquire()` tries the primary's
  guard first, falls through on `Permit::Denied` / typed error, and records
  which source succeeded for the health page. Likely lives in
  `src/providers/mod.rs`.
  (2) **Candidate sources.** Likely free/free-tier: Tiingo, Alpha Vantage,
  Polygon (with caveats), or Stooq's CSV mirror for daily history; a second
  Yahoo path or Marketstack for quotes; a fixed snapshot from `companyfacts`
  for fundamentals where SEC is already the canonical source. Each gets
  its own `EndpointGuard` row and budget (per PLAN.md's anti-spam policy).
  (3) **Routing rules.** Stickiness — once a fallback is in use, when does
  the system retry the primary? Likely a probe at the next due-check tick
  once the primary's breaker closes. Per-symbol overrides for upstreams
  that cover a different sub-universe (e.g. some sources don't carry
  futures or non-US stocks).
  (4) **Health page surfacing.** The page should show which source is
  currently in use per concern, when a fallback last took over, and the
  full list of configured backups with their own breaker / budget state.
  No new core feature, but a meaningful change to the provider layer; it
  goes beyond a small refinement, so the user can pick the ordering after
  the current backlog.

- [ ] **Phase 29: Issuer-direct ETF data feeds.** (Captured 2026-05-22
  from a vibe-coding side note mid-Phase-28.) The user's steer: *"as a
  future plan we tie into iShares/blackrock and Vanguard directly for
  even more data for their ETFs (and others if available/popular)"*.
  Phase 28 builds the ETF page off SEC N-PORT and Yahoo `quoteSummary`,
  the broadly-available free sources; Phase 29 layers richer
  issuer-direct data on top wherever the issuer publishes it. Issuers
  often publish their own fund factsheets and full daily holdings as
  CSV / JSON feeds that go beyond N-PORT's quarterly snapshot
  (daily refresh, full position list rather than just top-25,
  effective duration / SEC yield / option-adjusted spread for bond
  funds, sector and country breakdowns done by the issuer's own
  methodology, distribution schedules, factsheets, prospectus PDFs,
  premium / discount history, and so on).

  **Likely issuer sources to investigate at build time:**
  - **iShares / BlackRock** — `ishares.com` carries per-fund product
    pages with downloadable CSV holdings, distributions, factsheet
    PDFs and JSON behind the page; needs research on the stable
    endpoint shape.
  - **Vanguard** — `vanguard.com` advisor product feeds and per-fund
    JSON for holdings, characteristics, distributions.
  - **State Street / SPDR** — `ssga.com` for SPY, SPDR sector funds.
  - **Invesco** — for QQQ and the BLDR / DBA / DBC commodity funds.
  - **Schwab / Fidelity / WisdomTree / VanEck / First Trust** — pick
    based on what the user holds enough interest in.

  **Open design points to settle when building:**

  (1) **Trait composition.** Likely a `FundExtrasProvider` trait per
  issuer, dispatched by `symbols.issuer` (a new column or derived from
  the fund name), with a `merge_into` step that overlays issuer fields
  onto the Phase 28 `fund_profiles` row only where the issuer feed has
  fresher / richer data than N-PORT. SEC stays the canonical "every
  fund has it" baseline; issuer data is the cherry on top.

  (2) **Endpoint guards per issuer.** A new `EndpointGuard` row per
  issuer domain, each with its own conservative budget per the
  anti-spam policy. None of these are documented public APIs; treat
  them carefully.

  (3) **Coverage and graceful degradation.** Only a few large issuers
  are worth implementing; the long tail (small ETF sponsors, foreign
  issuers) stays SEC + Yahoo only. The page must not look broken on
  an unsupported issuer's fund — extras simply do not render.

  (4) **Scheduler placement.** Likely a new `fund_extras` job on its
  own daily / weekly cadence, separate from the Phase 28 `fund_metadata`
  job, so an issuer outage does not stall the Yahoo path.

  (5) **Health page surfacing.** Each issuer guard surfaces on
  `/health` like the existing four; the symbol page may want a small
  "enriched by <issuer>" provenance line where issuer data is in use.

  No order or schedule attached; the user can pick after the current
  backlog. Depends on Phase 28 (the schema and the fund-profile
  scaffolding to overlay onto).

- [x] **Phase 30: Top picks + backtest.** Complete and verified locally
  2026-05-23; see the Phase 30 entry in Status and the decisions log.
  (Captured 2026-05-23 from a vibe-coding session right after Phase 28
  deployed. Picked as the next phase to build on 2026-05-23.) The user's steer, verbatim:
  *"on the home page i'd like to use all the fundamentals and trajectory
  and stats we have in the system to show a new panel on the home page —
  this panel would be top 5 picks for day / week / month / year — this
  would be our best guess given time frames relevant to this information
  and all the information we have as to what we should invest in — i'd
  also like a feature tied to this in guess testing the results so this
  would be a separate page you go to that shows the results of our guess
  with a win rate and % gains if $ invested was done kind of thing as
  like a stress test to show how solid our pick rate is — also i know
  you are not a financial advisor i am not either this is just for fun
  and testing"*. Two surfaces, one shared picking engine.

  **Scope settled 2026-05-23 (see the decisions log).** Picks are
  *forecast-horizon* picks: each tier predicts who will do best over
  that forward horizon (day = movers / intraday momentum; year = the
  Phase 20 fundamentals + trajectory composite). The holding period is
  implicit in the horizon. A "for fun and testing — not financial
  advice" disclaimer rides quietly on both surfaces.

  **Planned pieces:**

  (1) **Per-horizon ranking math** in `compute.rs`. Four pure functions,
  each taking the same per-stock input bundle (latest price, full daily
  history, current-day intraday bars, Phase 7 fundamentals, Phase 20
  standing) and returning a per-symbol score. Suggested signals — settle
  at build time:
    - **Day:** today's intraday return + a small "near 52-week high"
      bias + a fundamental-strength filter (no weak-graded stocks).
      Conceptually the movers panel sharpened by quality.
    - **Week (~5 trading days):** 5-day return + RSI not extreme
      (skip > 70 / < 30) + above SMA50. Short-momentum read.
    - **Month (~20 trading days):** 20-day return + above SMA200 +
      Phase 20 standing not weak. Medium-momentum read filtered by
      fundamental quality.
    - **Year (~252 trading days):** reuse Phase 20's combined score
      directly — fundamentals 2:1 + trajectory — which is already the
      year-horizon answer the app computes.

  (2) **Home-page panel** alongside movers and strongest / weakest. A
  "Top picks" section showing four small lists of 5 stocks each
  (Day / Week / Month / Year), each row a `verdict_badge` + ticker +
  the score's headline figure (the relevant return or composite).
  Either a 4-column row on desktop and stacked on phone, or a tabbed
  segment — design pass at build time. A quiet disclaimer line under
  the section header ("for fun and testing, not financial advice"). The
  panel is a fixed page-load snapshot, like movers and strongest /
  weakest (per Phase 11 / Phase 20), so the home render stays cheap.

  (3) **Daily pick snapshot** to a new `picks` table (migration `0009`),
  one row per (snapshot_date, horizon, rank 1-5, ticker, score,
  price_at_pick). Snapshotted by a new scheduler section right after
  the daily-close job (a known once-a-day market-close moment when
  every stock has a fresh close). This is the load-bearing piece for
  the backtest: without it the backtest can only replay today's algo
  over old data, which means *every algo tweak invalidates history*.
  With it the picks the app actually made on every past day are immutable
  and the backtest is honest. The first day the feature ships seeds
  day 1 of the table; the table grows from there. (No retroactive
  backfill — see the decisions log when it's settled.)

  (4) **Backtest page** at `/backtest`. A simulation: starting capital
  (default $10k), pick a horizon, rebalance into the day's 5 picks
  equal-weight every period (day picks = daily rebalance, year picks =
  yearly), accrue returns through the snapshot history. Outputs:
    - **Equity curve** (lightweight-charts area), $X simulated capital
      over time, with `^SPX` benchmark dashed on the same axes.
    - **Win rate.** Settle definition at build time — either per-pick
      (% of picks that gained over their horizon) or per-period (% of
      horizons where the basket beat the benchmark). The user's phrasing
      "win rate" suggests per-pick; the per-period view is sharper for
      strategy quality. Likely show both.
    - **% gain vs benchmark.** Total return of the strategy minus
      `^SPX` total return over the same window. Compounded annualised
      growth (CAGR) once the history is long enough.
    - **Tabular history.** Each snapshot's picks and how each fared.
  A "for fun and testing — not financial advice" disclaimer rides at
  the top of the page.

  (5) **Disclaimer.** A new shared `.disclaimer` style (base.scss) — a
  quiet ink-faint line, smaller than body text, never green / amber /
  red. Used on both the home panel and the backtest page.

  **Schema.** Migration `0009` adds `picks (snapshot_date TEXT,
  horizon TEXT, rank INTEGER, ticker TEXT, score REAL, price_at_pick
  REAL, PRIMARY KEY (snapshot_date, horizon, rank))`. No change to
  `symbols`. Compute is pure (no new network calls) — reuses the data
  the home and symbol routes already read.

  **Open design questions (settle at build time):**
  - **Universe per horizon.** Stocks-only on all four? Or include
    curated ETFs on the year horizon (a steadily growing fund is a
    legitimate one-year pick)? Probably stocks-only on day / week /
    month (fundamentals filter excludes ETFs anyway), open on year.
  - **Win-rate definition** — per-pick vs per-period (see piece 4).
  - **Backtest UI** — four separate charts (one per horizon) or one
    chart with a horizon toggle. The latter is tidier; the former
    enables side-by-side reads.
  - **Rebalance cost.** Skip transaction-cost modelling in v1 (single
    operator, no real money) and label the backtest as "frictionless"
    in its provenance line. A later refinement could add a flat bps
    drag.
  - **Retroactive backfill.** Skip in v1 — the snapshot table grows
    forward from the first deploy and the backtest is honest about
    "history since 2026-Mm-DD". Backfilling by replaying today's algo
    over past `daily_prices` would be tempting but lies about what
    the app actually said at the time, so it stays out.
  - **Disclaimer wording** — design pass with the user.

  **Anti-spam.** Zero new network calls. Compute is pure over data
  already stored (`daily_prices`, `intraday_bars`, `fundamentals`,
  `quotes`). No new `EndpointGuard` row.

  **Dependencies.** Phase 20 (its standing score is the year horizon
  directly, the strength filter on the shorter horizons). Phase 11 /
  Phase 20 home dashboard scaffolding to hang the panel on. No hard
  dependency on Phase 14 / 15 / 16 / 17 (could later layer leadership
  and industry signals into a future v2 of the picks).

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
  Deployed to production on 2026-05-22 via `git push server master`
  (commit `39a863e`).
- **2026-05-22: search "Add" affordance fixed for short tickers.** Bug: the
  Search page offered to add an untracked symbol only when the query returned
  zero results, but search matches `ticker LIKE '%q%'` (a substring), so a one
  or two letter ticker like `W` (Wayfair) partially matched a crowd of other
  tickers and the offer never showed, leaving `W` unaddable. Fix
  (`routes/search.rs`): the offer is now gated on an exact-ticker `EXISTS`
  check against `symbols`, not on `results.is_empty()`, so a query that is a
  valid, untracked ticker is offered even when the substring search found
  other symbols. The "do not nag a company-name search" intent is kept: the
  offer still shows only when the query matched nothing, or matched at least
  one result as a ticker substring (a name-only search such as `Inc` does not
  trigger it). The add panel can now render above a populated results grid.
- **2026-05-22 — Phase 26 picked next; design Q&A settled four points.**
  Asked which loose-ordered backlog phase to take next, the user chose
  Phase 26 (dividend payouts). A short design pass settled the open
  questions the plan had flagged. (1) **Chart pips deferred to Phase 25.**
  Phase 26 ships the page section only; Phase 25 will build the
  lightweight-charts marker layer for earnings + ex-div together, which
  keeps Phase 26 tight and avoids writing marker plumbing without an
  earnings caller to compare against. (2) **Yahoo dividend-history depth:
  range=5y, interval=1d, events=div** — enough for prior-year + YTD pace
  and a long visible history list, with a modest per-call payload; the
  candle stream itself is discarded by the new method. (3) **Count-tempered
  projection.** Project YTD up by the ratio of expected payments this year
  to declared so far (so a quarterly payer at end-of-Q1 projects ×4, not
  ×~4 by elapsed days). Reads more honest around the cadence than an
  elapsed-fraction-of-year approach. (4) **Symbol-page slot.** The new
  Dividends section sits between Fundamentals and Leadership, so the page
  reads as key stats → fundamentals → financials → dividends → leadership
  → filings.
- **2026-05-22 — Phase 26 dividend payouts shipped (local).** Yahoo is now
  the source for a third concern beyond quotes and intraday bars. Design
  calls made during the build: (1) the `dividends(ticker)` method is
  inherent to `YahooProvider`, not behind a new trait — the data lives on
  the same v8 chart endpoint as the existing quote / lookup methods, and
  the Phase 27 backlog (provider redundancy) is the right place to lift it
  to a trait if it gains a second source. (2) The new `dividends`
  scheduler job rides the existing `yahoo` `EndpointGuard` (no new row,
  budget shared with intraday + daily-close); declared dividends drift
  slowly, so a weekly staleness window keeps the steady-state cost tiny.
  (3) A stock that has not been swept yet shows a "not synced yet" pending
  note in place; a swept stock with no payouts in the past five years
  hides the section entirely (it pays no dividend — no heading over an
  empty table). (4) Cadence is inferred from the *median* gap between the
  last up-to-8 payouts, with comfortable bands (≤45d monthly, ≤130d
  quarterly, ≤220d semi-annual, ≤450d annual) so a single irregular gap
  does not throw the classification. (5) The on-track grade uses a small
  ±2% flat band to keep a rounding-grade payment change from reading as
  "growing" or "shrinking" (PACE_FLAT_BAND). (6) The add-symbol backfill
  (`scheduler::backfill_symbol`) was extended to pull a new stock's
  dividend history before responding, mirroring Phase 21's intent that a
  user-added symbol's page is complete the moment the add returns.
  Per-share figures display to the cent normally (`$0.24`) and widen to
  4dp for sub-cent payouts (`$0.0625`, the monthly REIT case) so a small
  payment is not lost to rounding. Deployed to production on 2026-05-22
  via `git push server master` (commit `7608b06`); migration `0007`
  applied cleanly on the box, and the new `dividends` scheduler job
  backfilled the universe on its first tick post-boot.
- **2026-05-22 — Phase 27 captured: backup providers for redundancy.**
  While Phase 26 was mid-build the user floated a wish for additional
  *backup* providers per data concern (history / quotes / fundamentals /
  dividends), so the app can switch over when the primary's
  `EndpointGuard` is unhappy — breaker open, hourly budget spent, or any
  transport error — and so we have "as much redundancy as possible". Per
  the vibe-coding rule the idea was budgeted into the plan rather than
  acted on mid-phase: see the new Phase 27 entry in the Phases list, which
  also enumerates the four open design points to settle when building (a
  `MultiProvider<T>` wrapper, candidate sources, routing rules and
  stickiness, health-page surfacing of the active source).
- **2026-05-22 — Phase 28 captured: ETFs as first-class citizens.** Right
  after Phase 26 deployed, the user flagged a strong wish: ETFs are
  currently treated as second-class — they get a small fund profile
  (AUM, holdings, asset mix) and the SEC filings list, but no
  distributions (Phase 26 was stocks-only), no expense ratio, no
  distribution yield, no NAV / premium-discount, no sector or geography
  breakdown, no trailing returns, no benchmark comparison, no strategy
  summary. The user wants ETFs to read as densely as a stock page. Per
  the vibe-coding rule the idea was budgeted into the plan rather than
  acted on at once — Phase 28 in the Phases list enumerates seven
  planned pieces (distributions, expense ratio + yield via Yahoo
  `quoteSummary`, NAV / premium-discount, sector + geography exposure
  from N-PORT, trailing returns, strategy summary + inception, benchmark
  comparison) with their data sources and the schema implications. The
  user did not specify order; built as one phase or split 28a / 28b at
  build time. No new endpoint guards required.
- **2026-05-22: two side notes captured as Phases 25 and 26.** While waiting
  on the search-Add fix to deploy, the user floated two ideas: (1) earnings
  dates on the symbol page (last and next, with days-to / days-since), and
  a pip on the chart at each earnings date so a large move that followed
  an earnings print is explained at a glance; (2) dividend payouts on the
  symbol page (per-event date and amount, previous calendar year total and
  current YTD total, plus an "on track" pace read against last year), with
  ex-div pips on the chart sharing the Phase 25 marker layer. Past
  earnings dates come for free from 8-K item 2.02 filings already stored
  by Phase 14; the next-expected date needs a forward source (Yahoo
  `quoteSummary` calendarEvents, or estimation from prior-year cadence)
  and is left to settle at build time. Dividend payouts come from Yahoo
  chart `events.dividends`; SEC XBRL's quarterly `DividendsPerShare` is
  per fiscal period not per payout, so it does not stand in. Both are
  stocks-only and budgeted as Phase 25 and Phase 26 in the post-MVP
  backlog; no order or schedule attached.
- **2026-05-22 — Phase 28 picked next; design Q&A settled four points.**
  Asked which loose-ordered backlog phase to take next, the user chose
  Phase 28 (ETFs as first-class citizens). A short design pass settled
  every open question the phase's plan entry had flagged. (1) **Scope:
  one big phase, all seven pieces.** Not split 28a / 28b — the user
  wants ETFs to read as densely as a stock page in one ship. (2)
  **Expense ratio and yield come from Yahoo `quoteSummary`,** modules
  `fundProfile + defaultKeyStatistics + summaryDetail + price`, in a
  single request behind the existing `yahoo` `EndpointGuard` — not
  hand-curated in `starter.csv`. Phase 18 had once considered Yahoo
  here and dropped it, but hand-curation does not scale to user-added
  ETFs (Phase 9), which settled it. (3) **Benchmark comparison uses a
  hand-curated `benchmark` column on `symbols`** (the plan's pragmatic
  path), populated for the curated universe (SPY/VOO->^SPX, QQQ->^NDX,
  DIA->^DJI, IWM->^RUT, sector SPDRs, etc.). A user-added ETF simply
  omits the overlay rather than guessing wrong from a flaky auto-detect.
  (4) **All four extras are in:** the full trailing-returns set
  (1m/3m/YTD/1y/3y/5y/10y/since-inception, annualized past 1y), the
  growth-of-$10k area chart over the longest available range, the live
  NAV / premium-discount badge, and N-PORT sector + geography panels.
  The Phase 28 entry in the Phases list was updated to read "scoped, in
  progress" with these answers folded in.
- **2026-05-22 — Phase 28 ETFs as first-class citizens shipped (local).**
  Yahoo is now the source for a third concern beyond quotes and
  dividends, and SEC N-PORT carries two more aggregations (sector,
  geography) past the asset mix. Design calls made during the build:
  (1) **Yahoo `fund_metadata` is an inherent `YahooProvider` method,
  not a new trait** — the data lives on `v10/finance/quoteSummary`, a
  single-source concern, same call as `lookup` / `dividends`; Phase 27
  (provider redundancy) is the right place to lift it if a second
  source ever joins. (2) **Sector mix uses N-PORT's `<issuerCat>`,
  geography uses `<invCountry>`** — N-PORT does NOT carry GICS sector
  codes (issuer-computed metadata, issuer-direct only), so the sector
  panel is honest about what SEC provides: CORP / GOVT / MUN /
  Registered fund / etc. Meaningful for bond and multi-sector funds,
  degenerate (single bucket) on a pure-equity ETF — the template
  hides the panel in that case rather than render a flat bar carrying
  no information. Finer GICS sectors are budgeted as a Phase 29
  cherry-on-top. (3) **Yahoo `quoteSummary` defensively handles
  401 / 403 alongside 429 / 503 as `RateLimited`,** because Yahoo's
  v10 endpoint is occasionally crumb-gated and surfaces auth failures
  inconsistently; treating all four as the guard's rate-limit signal
  keeps a broken upstream from spinning while letting the breaker
  recover cleanly when Yahoo flips back. The WSL2 dev box is
  blanket-429'd by Yahoo today (a known cloud-IP issue), so the
  endpoint was verified by parser-level reasoning and unit tests; the
  production alpine server has been hitting Yahoo cleanly for months
  (Phase 5, 18, 26), so the deploy is the source of truth. (4) **A
  separate full-history query for trailing returns,** not the
  400-bar window the price chart's key stats reuse — without it, the
  3y / 5y / 10y / since-inception rows always read as missing.
  (5) **Benchmark column is hand-curated in `starter.csv`** (per the
  scope Q&A): the 18 broad-market ETFs that have a clean curated
  benchmark in our index universe get one (SPY / VOO / ... -> ^SPX,
  QQQ / SMH / ARKK -> ^NDX, DIA -> ^DJI, IWM -> ^RUT); sector SPDRs
  map to ^SPX too, international / bond / commodity ETFs leave it
  blank, so the relative-performance overlay hides itself rather than
  guess wrong. A user-added ETF (Phase 9) simply omits the overlay.
  (6) **One growth-of-$10k chart in lightweight-charts**, separate
  from the price chart so the longest available range (since
  inception) is shown without coupling to the price chart's range
  buttons; benchmark rides on the same panel as a dashed line
  re-scaled to $10k from the fund's first bar (not the benchmark's
  earliest bar), so the two lines start together. (7) **Dividends
  template lifted out of the stocks-only conditional** rather than
  duplicated; the section title flips to "Distributions" when the
  symbol is an ETF. (8) **The Phase 28 build accidentally surfaced a
  latent verification gap:** trailing returns first read from the
  chart's truncated 400-bar window, which capped them at ~1.5 years
  and made every long-horizon row miss; caught in a render check on
  SPY ("3y/5y/10y all show —"), fixed with the separate full-history
  query in symbols.rs. No deploy yet — the local verification used a
  synthetic `fund_metadata` row injected via the python `sqlite3`
  module since Yahoo refuses this WSL2 IP.
- **2026-05-23 — Phase 30 follow-up: `pick_day` now uses the last
  completed daily bar, not live intraday.** Mid-verification the user
  flagged the Day picks as reading "after the fact": a stock up 12%
  intraday today was being top-ranked for "today's pick", which is
  exactly the post-hoc trap (if the move happened during the session,
  buying on it means chasing what already happened). Two fixes shipped
  together: (1) `pick_day` now reads `closes[-1] / closes[-2] - 1`
  (most recent completed daily bar's close-to-close return), not
  `last_price / prev_close - 1`. After-hours both forms give the same
  number; during regular hours the new form returns yesterday's
  full-day move instead of today's intraday-so-far — a legitimate
  forward-looking momentum signal for tomorrow's continuation. The
  `52-week-high bias` was also refactored to read off the same
  `closes` series. `PickInput` dropped `last_price` / `prev_close`
  entirely, so every ranker now reads the same single-source slice
  and no signal can leak from intraday. (2) Horizon labels reframed
  forward-looking — `Tomorrow / Next week / Next month / Next year`
  (the `key` strings stay `day / week / month / year`, since the
  picks table stores them). The home panel's section note and
  disclaimer now both call the displayed figure "the signal that
  earned the pick" rather than implying it is a prediction of
  return. The picker is honest about doing momentum + quality, and
  honest that it does not know what tomorrow holds.
- **2026-05-23 — Phase 30 top picks + backtest shipped (local).** Design
  Q&A settled three points before the build: stocks-only across all
  four horizons (the user's pick over including ETFs on the year
  horizon; the short rankers all filter on Phase 20 strength, which
  only stocks carry), both win-rate definitions shown side-by-side
  (per-pick = % of picks that gained over their horizon, per-period
  = % of horizons where the basket beat `^SPX`), and one chart with
  horizon tabs over four side-by-side charts (tidier, deeper detail
  per horizon). Build calls made during the work: (1) **acknowledged
  look-ahead bias from today's standing.** Per-period fundamentals are
  not stored, so the backtest applies today's `Strong/Fair/Weak`
  verdict to every historical date. Surfaced explicitly in the page
  disclaimer; a future phase could store point-in-time fundamentals
  if we ever want a clean out-of-sample run, but for v1 this matches
  the user's "for fun and testing" framing. (2) **`HIST_LOOKBACK_DAYS`
  cap of 7 years on the backtest load query.** A first run pulled
  ~948k rows from `daily_prices` (some indexes go back to the 1700s)
  and the API took 7s; the cap drops it to ~1-2s without losing any
  realistic backtest depth (the year horizon's 5 rebalances × 252
  bars ≈ 5 years). (3) **The pick rankers' headline figures are the
  raw score**, so the home panel and the backtest both display the
  same number per row without a separate display pass — `+3.2%`,
  `+12.0%`, etc. (4) **`pick_year` returns Phase 20's combined score
  on a `× 100` scale** so the four lines of the home panel read on
  roughly the same magnitude; the year column shows the raw composite
  rather than a percent, which is honest about what the year ranker is
  ranking on. (5) **The snapshot job fires right after `daily_close`,
  keyed in `meta` on `picks_snapshot_date`**, and only proceeds once
  `daily_close_date` is also today's — so the picks are never scored
  off stale prices, and the job is idempotent across restarts.
- **2026-05-23 — Phase 30 captured + picked next: top picks + backtest.**
  Resuming from a cleared context, the user floated a new feature mid-
  resume: a home-page panel of 5 forecast-horizon picks for day / week /
  month / year using the fundamentals / trajectory / stats the app
  already carries, plus a separate `/backtest` page that simulates
  following the picks and reports a win rate and $ gain. Two design
  questions settled the scope before write-up: (1) "pick" means
  *forecast-horizon* — each tier predicts who will do best over that
  forward horizon, with different signals per horizon (day = movers /
  momentum, year = Phase 20's combined fundamentals + trajectory score
  unchanged), holding period implicit; (2) picked as the next phase to
  build, ahead of the loose 13/15/16/17/19/25/27/29 queue. Budgeted as
  Phase 30 (Phase 29 already holds the issuer-direct ETF feeds backlog).
  The phase's most load-bearing call is the new `picks` table
  (migration `0009`): daily snapshot of each horizon's 5 picks right
  after the daily-close job, so the backtest runs against immutable
  historical picks rather than replaying today's algo over old data
  (every algo tweak would otherwise rewrite history). v1 grows the
  history forward from first deploy — no retroactive backfill, the
  backtest is honest about "history since". Anti-spam clean: zero new
  network calls, pure compute over data already stored. The user
  explicitly said "i know you are not a financial advisor, i am not
  either, this is just for fun and testing" — captured as a quiet
  disclaimer line on both surfaces. Open design questions noted in
  the phase entry (universe per horizon, win-rate definition, backtest
  UI layout, retroactive backfill, disclaimer wording) settle at build
  time per the vibe-coding norm.
- **2026-05-22 — Phase 29 captured: issuer-direct ETF data feeds.**
  Mid-Phase-28 the user floated: *"as a future plan we tie into
  iShares/blackrock and Vanguard directly for even more data for their
  ETFs (and others if available/popular)"*. Per the vibe-coding rule
  budgeted into the plan rather than acted on mid-phase: see the new
  Phase 29 entry in the Phases list, which enumerates the candidate
  issuer sources (iShares, Vanguard, State Street / SPDR, Invesco, and
  others) and five open design points (trait composition, per-issuer
  endpoint guards, coverage and graceful degradation on unsupported
  issuers, scheduler placement, and `/health` surfacing). Depends on
  Phase 28 to overlay onto.
- **2026-05-23 — Stock universe expanded to the full S&P 500.** The
  hand-curated ~110-stock list in `universe/starter.csv` was replaced
  with all 503 current S&P 500 constituents (dual-class names like
  GOOGL/GOOG and BRK.B counted separately). Indexes (6), ETFs (28) and
  futures (10) are unchanged, so the file now holds 547 symbols. The
  list was scraped from Wikipedia's `List_of_S%26P_500_companies`
  constituents table on 2026-05-23; it is a dated snapshot, not an
  auto-refreshing feed (membership churns a few times a year, refresh
  manually when noticed — auto-refresh job left as a follow-up). The
  `exchange` column is empty for the new rows (Wikipedia does not
  carry it); the existing `Option<String>` schema and downstream code
  already tolerate that. `benchmark` stays empty for stocks per the
  existing convention. Side effects: the home page's strongest/weakest
  panels and the Phase 30 top-picks selector both draw from
  `is_seeded = 1 AND kind = 'stock'`, so their candidate pool widens
  from ~110 to 503 — which is the point, the user picked the S&P 500
  as "most of the large US companies most people care about". Cold-
  start cost: Stooq history backfill for the ~390 new tickers takes
  ~2 hours of guard budget (200/hour) and resumes across scheduler
  cycles; SEC EDGAR fundamentals and Yahoo dividends backfill the
  same way over their own daily cadences. No new endpoints, no new
  burst risk: every call still routes through `EndpointGuard`. Phase
  1's "144 symbols" note and the Phase 3 decisions-log line about the
  200/hour budget sitting "comfortably above one full universe
  refresh (~144 calls)" are now historical — a full refresh today
  spans multiple hours by design; the seed and incremental jobs are
  built to stop and resume against the guard.
- **2026-05-23 — Phase 30 rework: year horizon → quarter, and a true
  out-of-sample backtest.** The user flagged that the Day/Week/Month
  tabs varied year-over-year but the Year tab showed the same top-5
  in every period — MU in 2022 despite a flat-downward trajectory.
  Diagnosed: `pick_year` was a pass-through of today's `Standing`,
  and `HistBundle::standing` was computed once from today's
  fundamentals and today's full close history, then reused at every
  historical rebalance. So the year horizon was constructed to be
  identical year-over-year; the short rankers also carried a softer
  today-bias via their `Bad`-grade filter (a stock weak now never got
  picked historically). Two fixes shipped together:
  - **Year → Quarter.** `pick_year` dropped; new `pick_quarter`
    mirrors `pick_month`'s shape on a 63-bar window (one earnings
    cycle), gated on close > SMA200, same `Weak` filter. A year
    forecast that is honestly "today's standing rank" was not pulling
    its weight; quarter sits cleanly between month and "no horizon"
    and produces a real momentum read. `HORIZONS`, `stride_for`,
    `max_bars_for`, and the match arms in `compute_picks` / `rank_at`
    all updated. Migration `0010_quarter_horizon.sql` deletes any
    stale `year`-keyed rows in the `picks` snapshot table; the next
    `daily_close` writes a fresh `quarter` snapshot.
  - **Per-rebalance standings.** `models::FundFact` now carries
    `period_end`; new `models::latest_annual_inputs_as_of(facts,
    price, as_of)` picks the latest annual whose `period_end +
    FILING_LAG_DAYS (90) ≤ as_of` — a conservative SEC 10-K filing-
    lag cushion (10-Ks are due 60–90 days after fiscal year end
    depending on filer category), so the backtest never grades a
    stock with figures that did not yet exist on its as-of date.
    `HistBundle` carries raw `FundFact`s (not a precomputed
    `Standing`); `rank_at` computes the standing per rebalance from
    the closes-so-far slice and the as-of-filed inputs. Every
    `FundFact` SELECT (4 sites: routes/home.rs, routes/search.rs,
    routes/symbols.rs, picks.rs ×2) was extended to fetch the new
    `period_end` column.
  - **Disclaimers.** The "acknowledged look-ahead bias" copy on the
    `/backtest` page and in the `routes/backtest.rs` + `picks.rs`
    module docs is replaced with the honest "genuinely out-of-sample"
    description. Home-page Top-picks copy reframed from "next year"
    to "next quarter".
  - Verified: cargo + bun build clean; `/backtest` Quarter tab runs
    19 rebalances over 2021-08 → 2026-02 with genuinely varying picks
    (2021 H2 tech, 2022 H1 defensives, MU appearing in exactly one
    2022 period and the backtest honestly recording the -19.8%
    exit). Home page renders all four columns including "Next
    quarter" with the new description text. No deploy yet.
- **2026-05-23 — Phase 16 picked next; design Q&A settled four points.**
  Asked which of the remaining backlog phases (13, 15, 16, 17, 19, 25, 27,
  29) to build next; the user picked Phase 16 (per-ticker anomaly feed).
  Four design questions then resolved the scope:
  - **Event types** (multi-select, all chosen): large daily price moves;
    drawdowns / new multi-month lows; large YoY fundamentals changes;
    leadership changes from 8-K item 5.02 (already filtered in
    `filings.items` by Phase 14).
  - **Coverage**: all instruments. Stocks get all four event types; ETFs
    and indexes and futures get price-only events (the chart's own move is
    still legible, but a dated bullet list of "−7.2% on 2024-08-05"
    captions stand-alone reading). Fundamentals + leadership events stay
    stocks-only by construction.
  - **Placement**: between Leadership and Recent SEC filings — sits with
    the other event-shaped sections.
  - **Item style**: one line per event — date · glyph · headline, newest
    first, capped to ~20 over the past year. Matches the Phase 14
    leadership-changes feed.
  Two follow-ups settled with sensible defaults (user picked the
  recommended option on both): window is past 1 year; thresholds are
  selective (price |move| > 2σ vs trailing 90-day vol AND > 5%;
  fundamentals ±25% YoY on revenue or net income; drawdown is a new
  6-month low). Pure derivation from data already stored (`daily_prices`,
  `fundamentals`, `filings.items`): no new network calls, no schema
  change, no new `EndpointGuard` row. The section hides itself when a
  symbol has no qualifying events (the same way the leadership section
  hides on an unsynced stock).
- **2026-05-23 — Phase 16 per-ticker anomaly feed shipped (local).** Two
  pure-compute helpers (`price_anomalies`, `drawdown_anomalies`) plus a
  `models::fundamentals_anomalies` walker plus a small leadership-filings
  SELECT, merged in `routes::symbols::build_anomalies` into one feed
  capped at 20 over the past year. A new section between Leadership and
  the ETF block (which precedes Filings on a stock page) renders the feed
  in a one-line-per-event Paper-Ledger row, with a leadership row
  linking to EDGAR. The two new compute helpers carry four unit tests
  (spike-on-5%-AND-2σ, ignore-1%-at-2σ, fresh-6mo-low-flags,
  slide-dedupes-to-≤5). Verified against the dev DB: `/s/NVDA` rendered
  10 events in a balanced mix (+5.8%/+5.6%/+7.9%/+5.8% one-day moves,
  -5.5% downside move, -19% new 6-month low, FY2026 revenue + net
  income +65% YoY); `/s/AAPL` rendered 5 leadership-change rows only
  (a fair read for a low-vol single-digit-growth name); `/s/SPY` (ETF)
  + `/s/^SPX` (index) each rendered 1 drawdown event; `/s/GC=F` (future,
  no `daily_prices`) hid the section entirely. Desktop (1280px) and
  phone (390px) both rendered with no horizontal overflow and zero
  console errors. Deployed to production 2026-05-23 via `git push server
  master` (commit `a839737`), bundled with the Phase 30 rework and the
  S&P 500 universe expansion that were also pending deploy from earlier
  the same day.
- **2026-05-23 — Phase 17 picked next; scope settled in three Q&A.** The
  user picked Phase 17 (stock health read) over the remaining loose
  backlog (13 heat map, 15 industry trends, 25 earnings dates, 27 backup
  providers, 29 issuer-direct ETF feeds). Three scoping questions settled
  before any code:
  1. **Industry context** — Phase 15 (industry trends) was scheduled as a
     prerequisite in the original Phase 17 scope. The user chose to
     **drop industry from this phase** rather than detour through Phase
     15 first or fold a minimal SIC tag inline. The health read ships
     without industry context; a later pass after Phase 15 can layer it
     on. The Phases-list and Done entries note this explicitly.
  2. **Leadership signal** — Phase 14 ships only a roster + 8-K item-5.02
     change feed (no tenure, no insider/outsider read). The user picked
     a **stability score via recent change count**, not a qualitative
     note alongside the composite. Discrete three-band score (0-1 →
     Stable, 2-3 → Normal, 4+ → Churning) over the count of 8-K item-5.02
     filings in the last 730 days — deliberately lenient because large
     companies routinely file ~one planned-succession 5.02 a year.
  3. **Surfaces** — the user picked **symbol page + a home rank panel**
     (rather than symbol-page-only). The home panels mirror the Phase 20
     strongest / weakest pair, sitting above them; the symbol-page panel
     sits above Fundamentals (the existing Phase 20 standing badge stays
     in place, since it is specifically the per-ratio rollup and the new
     panel is the broader synthesis).
- **2026-05-23 — Phase 17 stock health read shipped (local).** Pure
  derivation on top of data the app already carries, no schema change
  and no new network calls. The composite is fundamentals 0.55 +
  trajectory 0.30 + leadership stability 0.15, renormalised over the
  components that landed so an unsynced stock is not penalised; bands
  use the same `STRONG_CUTOFF` / `WEAK_CUTOFF` as Phase 20 so the overall
  verdict (Healthy / Mixed / Concerning) stays consistent with the
  per-ratio standing (Strong / Fair / Weak). The new "Stock health"
  symbol-page panel renders three sub-rows (fundamentals / trajectory /
  leadership) with the actual numeric reasoning beside each (e.g. "7
  reported officer or director changes in the last 2 years"); the
  Fundamentals section below still carries its own standing badge —
  intentional, since the two read at different levels (per-ratio rollup
  vs broader synthesis) and the user wanted the synthesis at the top.
  The home Stock health pair mirrors strongest / weakest layout with a
  new `health_row` macro showing the three sub-component pills under
  the name and the overall verdict on the right. Verified: cargo + bun
  build clean; `/s/AAPL` reads Mixed (Fair / Climbing / Churning at 7
  changes), `/s/SPY` and `/s/^SPX` hide the panel cleanly; `/` renders
  the Healthiest panel with GOOGL / GOOG / MU leading; desktop and
  390px phone both render with no horizontal overflow and zero console
  errors. Deployed to production 2026-05-23 via `git push server master`
  (commit `8a16b14`); the live `finance.bythewood.me/` renders the
  Healthiest / Most concerning panel pair and `/s/AAPL` the new Stock
  health panel.

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
