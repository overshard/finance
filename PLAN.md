# finance — Project Plan and Resume Doc

`finance` is a self-hosted, demand-driven market watcher for stocks, ETFs,
indexes, and commodities: live charts, key stats, fundamentals, and SEC filings.
A single Rust + axum binary backed by SQLite, with a Vite frontend. Deploys at
`finance.bythewood.me`, published on GitHub as `finance`.

It is for *watching* the market only. No portfolio, no holdings, no money or
cost-basis tracking, no accounts, no auth. **Not investment advice.**

---

## How to use this file

This is the **living resume document**. The user periodically clears the AI
context to save tokens and resumes work from this file alone, so it must always
be accurate enough that a fresh context can continue with nothing else.

**Keep it current.** After every phase, decision, or change of direction:
update **Status**, tick the **Roadmap**, and append to the **Decisions log**.
Treat updating this file as part of finishing any unit of work.

**This is vibe-coded: the user riffs ideas.** When the user floats an idea,
budget it into this file right away (into the relevant phase or the Design
section) rather than acting on it immediately. Keep doing the current work.

**Phases are deliberately small, self-contained cutoffs** so the user can clear
the AI context between them and resume cleanly. Each phase ends with a verify +
commit + auto-deploy (`git push server master`) and a clean breakpoint.

---

## Status

_Last updated: 2026-06-04 (Phases A-D all built + verified on dev; roadmap
complete; everything uncommitted, holding per the user's call to batch the
commit/deploy)._

**Major refocus in progress (the "demand-only" rewrite).** The previous roadmap
(the "distill + ETF-first" rewrite, Phases 1-7, all deployed at `645b351`) shipped
a broad always-on dashboard. The user has now steered a focus shift: the app
could not get enough live data to show what they actually wanted, and the timed
background sweeps were spending API budget around the clock for symbols nobody was
looking at. The new principle:

> **Nothing is fetched unless a human is actively looking at it.** With nobody on
> the site, the app makes zero outbound calls.

The new shape (decided 2026-06-03, see the Decisions log for the full Q&A):

- **Drop every timed network sweep.** Remove the scheduler's `daily_close`, `sec`,
  `dividends`, `fund_metadata`, `fund_nav`, `earnings_calendar`, `asset_profile`,
  and the periodic `history` jobs. Keep only the local `prune` (no network) and
  the demand-driven intraday poll (already gated by the viewer-interest registry).
  Symbol data, including daily history, is fetched **on page load when stale** and
  on an explicit manual refresh, never on a timer.
- **Symbol pages become pull-on-demand.** Landing on a symbol (or hitting its
  manual refresh button) pulls the latest data with a **clear loading bar** over
  every piece being fetched and a **clear data-age indicator** on everything. Fast
  Yahoo data (quote / intraday / daily history) refreshes live on load; slow,
  rate-limited SEC data (fundamentals, filings, holdings, NAV) is pulled only when
  missing or stale. Manual refresh re-pulls everything.
- **Dashboard is fully reworked into a TradingView-style view:** a big real-time
  **normalized %-vs-SPX overlay** graph for the day, very clear about market hours
  (pre / regular / after-hours / closed), over a **session-scoped editable
  watchlist** of stocks/ETFs that refresh every ~5 minutes while the dashboard is
  open, with data-age indicators throughout. SPX is the fixed baseline. Starters:
  VTI, VXUS, BND, IAU, IBIT. Polls only while someone has the dashboard open. Also
  keeps three always-on market reads (user's call): overall market volume, ^VIX
  (risk tone), and SMA 50/200 as general-interest overlays.
- **Remove the Industries page entirely** (not useful without real-time sector
  data).
- **Search stays** as the way to jump directly to any symbol for its on-demand
  deep data. The full universe CSV remains a *searchable catalog* (metadata only;
  each symbol's data is fetched only when it is first viewed).

**Current work:** the demand-only roadmap (Phases A-D) is **complete and verified
on dev**. Everything is uncommitted — the user chose to hold and batch one
commit + deploy. The old surface is still live in prod until that deploy.

Phase D outcome (cohesion + polish):
- **Removed the now-dead `StreamEvent::Summary` machinery** (the old dashboard's
  live breadth/verdict push, unused since Phase C). Deleted `src/summary.rs`
  (`market_summary` / `market_verdict` / breadth) and the `summary` module; the one
  survivor, `vix_tone`, moved into `compute.rs`. Dropped the `Summary` stream
  variant, the scheduler's session-flip + intraday publishes, `sse_summary`, and the
  `finance:summary` re-broadcast in `stream.js`.
- **Trimmed `/health` to the demand-only reality.** `job_meta` / `job_rank` now carry
  only the surviving `intraday` (and `prune`) jobs instead of arms for the eight
  removed sweeps; the page lede was rewritten to "fetches market data on demand …
  the only timed job is the live intraday poll." Fixed a "1 jobs" pluralization in
  the systems verdict.
- **Data-age on the dashboard reads.** `market_reads` now carries an `asof` (freshest
  quote across ^SPX/^VIX/SPY); the reads strip shows "Prices as of {time}", kept live
  by `hero.js` — matching the symbol page's per-section ages.
- **Verified on dev:** `cargo build` + `vite build` clean, zero warnings; `/api/health`
  lists only the `intraday` job with the on-demand guards (sec/yahoo) healthy and the
  Yahoo budget reflecting on-demand calls; the dashboard chart + reads + "Prices as of
  …" caption render with zero console errors and no Summary references left in the
  running app; `/health` renders the new copy.

Phase D follow-up (dashboard freshness + graph legibility, from user feedback after
the close): the dashboard felt stale — "prices as of 7:40pm" while it was 8:25pm —
because it only updated via the market-hours intraday poll (which skips non-futures
when the market is closed, since stocks/ETFs don't trade then) and had **no on-open
refresh** like the symbol pages. Fixes:
- **On-open refresh.** New `GET /api/dashboard/refresh` → `scheduler::refresh_quotes`
  pulls fresh quotes for the watchlist + baseline (^SPX/^VIX/SPY) once when the page
  opens, regardless of session, skipping anything quoted in the last ~5min (so a reload
  doesn't re-hit Yahoo). `hero.js` calls it on init, then redraws — so opening the
  dashboard always shows current data and a current "as of" time. Verified: the caption
  jumped 7:40pm → 8:32pm on open (`refreshed: 7`) with the market closed.
- **Closed-state clarity.** The banner sub now reads "Prices update during market hours"
  when closed (was a static "all times ET"), so a frozen price reads as expected, not
  broken.
- **Graph legibility.** Each line now carries a `title` (its ticker / "S&P 500") that
  lightweight-charts labels at the line's last value on the price axis, and the palette
  was swapped for eight well-separated hues — so each line is identifiable by name +
  colour, not colour alone. Verified at the close: IAU / BND / S&P 500 / VTI / VXUS /
  IBIT each labelled on the axis, zero console errors.

Phase C outcome (dashboard rework: session watchlist + %-vs-S&P-500 hero graph):
- **Session-scoped watchlist.** New `watchlist` table (migration `0015`) keyed on
  an opaque `fin_sid` cookie (no accounts; minted via SQLite `randomblob`, no new
  crate). `src/watchlist.rs` owns the session resolve/seed/list/add/remove; a first
  visit seeds the starters (VTI, VXUS, BND, IAU, IBIT) and sets the cookie, an
  existing cookie's list is used as-is (even empty — a user who clears it isn't
  re-seeded). `routes/watchlist.rs` exposes `POST /api/watchlist` (add) +
  `/remove`. Add reuses a new `ensure_symbol` (refactored out of `add_symbol`) so a
  brand-new ticker is validated + backfilled into the universe first.
- **Normalized %-vs-S&P-500 hero graph.** `GET /api/dashboard` returns the day's
  series (S&P baseline + each watchlist symbol, each as % change from the session's
  first bar) plus the market reads. `home/scripts/hero.js` draws them on one
  lightweight-charts axis (ink baseline + a muted non-semantic palette), with a
  legend, re-fetched every 60s and on tab focus. Symbols with no intraday bars are
  cleanly dropped from the graph (their card still shows "no intraday data").
- **Market reads kept (user's call):** S&P level/move, VIX + tone (reuses
  `summary::vix_tone`), market **volume** (proxied off SPY — cash indexes carry no
  share volume on Yahoo — today vs its 65-day average → Heavy/Normal/Light), and the
  S&P's **50/200-day** stance (`compute::sma`). SMA 50/200 stay on symbol charts; the
  open sub-decision (a daily SPX chart on the dashboard) was resolved as **not now** —
  the dashboard carries the 50/200 read as a one-line trend stat instead.
- **Market-hours banner** (Regular / Pre-market / After hours / Market closed) with a
  coloured session dot, kept live by hero.js.
- **Watchlist cards** reuse the `spark-card` markup (so the base stream client
  live-ticks price + sparkline) with a hover remove button; an add box sits in the
  header. Add/remove reload the page so the cards, the stream tickers, and the graph
  re-sync.
- **~5-minute poll throttle** added to `run_intraday`: a viewed symbol quoted within
  the last ~4m45s is skipped, so a dashboard left open polls each symbol about once
  every five minutes (light on budget, plenty for delayed data). The baseline reads
  (^SPX/^VIX/SPY) carry `data-ticker` so they stay polled while the dashboard is open.
- **Stripped the old dashboard** entirely: home.rs's breadth / movers / ETF band /
  quality leaderboard / hero verdict code and structs, the old `home.js` (+ its
  `finance:summary` patcher), and the dead `compute::trailing_return`.
- **Verified on dev (curl + Playwright):** migration applies on boot; first visit
  sets `fin_sid` and seeds the 5 starters; `/api/dashboard` returns the baseline +
  watchlist series (with real intraday point counts) and all four reads (S&P +0.22%,
  VIX 16.06 Steady, volume 54.4M Light, "Above its 50- and 200-day"); add (TSLA) and
  remove (via both API and the UI buttons) work and persist across reloads; the hero
  overlay draws all five lines as % from open with the S&P baseline; the banner reads
  "Market closed"; clean at desktop (1280, 4-up reads) and mobile (390); zero console
  errors; `cargo build` + `vite build` clean with zero warnings.
- **Known limitations (Phase D / backlog):** (1) because symbols are polled on the
  ~5-min cadence and were last touched at different times in dev, the graph's series
  can span slightly different day-windows, showing a small x-axis gap; with all
  dashboard symbols polled together during live hours they align. (2) The dead
  `StreamEvent::Summary` machinery was **removed in Phase D** (`vix_tone` moved to
  `compute.rs`). (3) Pre/regular/after-hours **shading** on the graph itself was not
  built — the banner carries the session; shading is a backlog polish candidate.

Phase B outcome (on-demand symbol data + loading bar + data age + manual refresh):
- **New SSE refresh pipeline.** `scheduler::refresh_plan` decides which steps a
  symbol's refresh runs (the two price steps — live quote, daily history — always;
  the slow SEC / metadata steps only when their `*_synced_at` is stale, or on
  `force`). `scheduler::refresh_step` runs each, reusing the existing guarded
  `backfill_*` helpers. A new SSE route `GET /api/symbols/{ticker}/refresh?force=`
  (`routes::symbols::refresh_stream`) streams a `plan` event, a `step` event before
  and after each step, and a final `done {reload}`.
- **Step set:** stock → Live quote · Daily history · Fundamentals/filings/leadership
  (SEC) · Earnings · Sector & industry · Dividends. ETF → Live quote · Daily history
  · Holdings & filings (SEC) · Fund details & NAV · Distributions. Index/future →
  Live quote · Daily history. History uses an **incremental** since-last-bar fetch
  (deep `range=max` only when no history exists). The quote step publishes to the
  hub so an open page patches its price live; the ETF NAV step re-adds a small
  `store_fund_nav` and uses the (now un-`allow(dead_code)`-ed) `yahoo.fund_nav`.
- **Frontend (`symbol/scripts/refresh.js` + markup + `symbol.scss`):** a header
  Refresh button + status text + a thin progress bar. On load the page auto-runs a
  refresh (`force=0`); the button runs `force=1` (everything). The bar fills per
  step and names the current one. On `done`: if a **deep** (server-rendered) section
  was refreshed it reloads to show the new data + age; otherwise the live price was
  already patched and it just settles. A one-shot `sessionStorage` skip flag set
  before our own reload prevents that reload from re-triggering the auto-refresh —
  **no reload loop**.
- **Data age everywhere (already present, now honest):** every section heading
  already showed "synced … ago" off its `*_synced_at`; the reload refreshes them.
  The stale "lands on the next sweep" empty-state copy was rewritten to "pulled on
  demand — hit Refresh".
- **Staleness windows:** SEC fundamentals/filings 7d, leadership 30d, Yahoo
  metadata (earnings/profile/dividends/fund metadata) 7d. `force` ignores them.
- **Verified on dev (Playwright + curl):** the SSE emits plan → per-step running/ok
  → done; AAPL `force=0` ran quote+history+earnings+profile (the stale ones) and
  left fundamentals/dividends (4d) and leadership (11d, inside its 30d window)
  untouched, each showing its true age. A fresh symbol (NVDA) did a single reload
  then settled ("Updated just now", no loop, zero console errors). The manual
  Refresh button on VTI ran the full ETF step set with the bar reading "Updating:
  Distributions", reloaded once, and settled. (Live in-market price tick couldn't be
  exercised — after-hours during the check — but the quote step stores + publishes
  exactly as the intraday job does.)

Phase A outcome (strip background sweeps + remove Industries):
- **Scheduler gutted to demand-only.** Removed `run_history`, `run_sec`,
  `run_dividends`, `run_fund_metadata`, `run_fund_nav`, `run_earnings_calendar`,
  `run_asset_profile`, `run_daily_close_if_due` (and the `is_due` / `schedule_next`
  due-check helpers + their constants, ~1340 lines). The loop now only broadcasts
  market-session changes (+ a local-DB summary on a session flip), runs the
  demand-driven `run_intraday` (viewed symbols only), and the local `run_prune`.
  The per-symbol on-demand pull (`backfill_symbol` + its `store_*` / `backfill_*`
  helpers) is **kept** — it is the on-demand path Phases B/C build on.
- **Boot seed is metadata-only.** `run_boot_seed` now just calls
  `seed::sync_universe` (local upsert + prune, no network); the history backfill is
  gone from boot. `make seed` (the `seed` subcommand) keeps `seed::run` for an
  explicit, user-invoked bulk backfill.
- **Removed jobs swept from `/health`.** `register_endpoints` now also
  `DELETE FROM data_status WHERE job NOT IN ('intraday','prune')` so a prod DB's
  old job rows don't linger as stale.
- **Industries removed entirely:** deleted `src/routes/industries.rs`, the
  `industries` frontend entry + its Vite input, both `industries_*` templates, the
  home "Today's industries" band (`IndustryRow` / `industry_panels` + context vars +
  the now-dead `StockRow.sector` / `asset_profile_synced_at` fields), the
  `compute::industry_returns` / `seasonality` block + its tests, and every
  `/industries` nav/footer link. The symbol-page sector/industry tags are now plain
  `<span>`s (no link). The sector/industry *symbol data* (Yahoo assetProfile) is
  kept.
- **Retained-for-next-phase (marked `#[allow(dead_code)]`):** `yahoo.fund_nav`
  (Phase B on-demand NAV), `market::{et_date,is_et_weekday,after_close}` (Phase C
  market-hours), `db::get_meta`.
- **Verified on dev:** `cargo build` + `vite build` clean, zero warnings; boot does
  only local work (universe sync 562 + prune), and an 8s idle window makes **zero**
  outbound calls; `/` → 200 with no "industr" remnants, `/industries` → 404,
  `/api/health` lists only `intraday`. (Live intraday couldn't be exercised — no
  open market / viewer during the check; the path is unchanged from before.)

---

## Design principles

**"Paper Ledger" look (keep it).** An old-school accounting ledger reimagined
futuristic and clean: warm-paper background, ink-dark text, hairline rules,
monospace ledger figures, restrained serif headings. Tokens are CSS custom
properties in `base.scss :root`.

**Color is semantic and sparing.** Green / amber / red mean good / ok / bad
(price moves, data-age states, data-health states), never decoration. Chart
indicator lines are the one deliberate exception (a muted non-semantic palette).

**Data age is always visible and honest.** Because data is now fetched on demand
rather than on a timer, every figure on the site carries a clear, plain age read
("live", "2m ago", "stale, refreshing", "as of Fri close"). A stale figure is
never shown as if fresh. A refresh in flight shows a loading bar, not a spinner
guessing-game.

**Market hours are explicit.** The dashboard makes the current session
(pre-market / regular / after-hours / closed) unmistakable, and the day graph
delineates those periods rather than drawing one undifferentiated line.

**Scannability is the bar.** Land on the dashboard and read how your watchlist is
doing against the market today in one glance. Land on a symbol and read its
price, trend, and key figures (with their ages) without hunting.

**Dual-first, not mobile-first-only.** Desktop is information-dense and should
*use* its space; mobile distills to the key signals. Neither is an afterthought.

**Polish last.** Features land first; one focused UI polish pass closes the work
rather than nibbling polish mid-build.

---

## Data-source policy (the important reference)

All data is **free, no account, no API key.** The user considers *never hitting a
rate limit* critical: every outbound call goes through a persistent
`EndpointGuard` (DB-backed reactive circuit breaker + hard per-hour budget +
request pacing; survives restarts; see `src/guard.rs`). The demand-only refocus
*reduces* outbound traffic sharply, since idle time now means zero calls.

**Yahoo Finance is the only price source.**
- **Deep daily history:** `v8/finance/chart?interval=1d&range=max` returns a
  symbol's entire daily OHLCV in one call. Fetched on demand when a viewed symbol's
  stored history is stale or absent (no more periodic sweep).
- **Intraday + live quotes:** `v8/finance/chart?interval=15m&range=1d`. Polled only
  for symbols a browser is currently viewing.
- **ETF / fundamentals metadata:** `v10/finance/quoteSummary` (crumb-gated; the
  provider does the `fc.yahoo.com` primer + `getcrumb` dance, caches the crumb,
  rotates on 401/403). Modules: `fundProfile`, `calendarEvents`, `assetProfile`.
  Fetched on demand for a viewed symbol when stale.
- Budget: 1000 req/hr on the `yahoo` guard.

**SEC EDGAR** (no key, contact email in User-Agent; 600/hr guard): stock
fundamentals (`companyfacts`), filings (`submissions`), leadership (Form 3/4/5),
ETF holdings/AUM (N-PORT, `company_tickers_mf.json`). Fetched on demand for a
viewed symbol when its SEC data is missing or stale. Indexes don't file.

**Freshness model (demand-only):**
- **Live intraday (SSE-polled):** ONLY the symbols a browser is currently showing,
  via the viewer-interest registry in `src/stream.rs`. A dashboard watchlist symbol
  refreshes ~every 5 minutes while watched; an open symbol page refreshes faster.
  Nothing is polled when nobody's watching.
- **History + deep data:** fetched on page load when stale, and on manual refresh.
  There is no background sweep keeping idle symbols warm; a symbol nobody visits
  simply holds whatever it last had, and is refreshed the next time it is viewed.

---

## Architecture as built (condensed)

Single-binary axum app. `src/main.rs` (env init, `seed` subcommand, boot) →
`src/app.rs` (`AppState` + `Config` + `router()`). Async sqlx + SQLite (WAL);
migrations in `migrations/` applied on boot.

- **`src/providers/`** — one trait per concern: `QuoteProvider` /
  `HistoryProvider` (Yahoo), `FundamentalsProvider` (SEC). `http.rs` builds the
  shared reqwest clients.
- **`src/guard.rs`** — the persistent per-endpoint `EndpointGuard` (see policy).
- **`src/scheduler.rs`** — the 60s-tick tokio task. **Shrinking in Phase A** to
  just: broadcast market-session changes, run the demand-driven intraday poll
  (viewed symbols only), and run the local prune. All timed network sweeps are
  being removed.
- **`src/stream.rs`** — `tokio::broadcast` hub + per-ticker viewer-interest
  registry; `/stream` SSE forwards quote / market / health events. This registry
  is the heart of the demand-only model and stays central.
- **`src/market.rs`** — US session clock (Closed/Pre/Regular/Post) via
  `chrono-tz`. No holiday calendar (deliberate).
- **`src/compute.rs`** — pure numeric code: indicators (sma/ema/rsi), graded
  fundamental ratios, range-meter positions, sparkline SVG.
- **Templates** — minijinja in `templates/` with a Jinja2-faithful HTML
  formatter. **Frontend** — Vite from `frontend/static_src/` (entries: base, home,
  symbol, health, search; the `industries` entry is being removed), built with bun,
  served hashed at `/static/`.

**Key tables:** `symbols` (universe + denormalized latest price/snapshot +
per-source `*_synced_at` staleness columns), `daily_prices` (deep OHLCV),
`intraday_bars` (15m, pruned 14d), `quotes`, `fundamentals`, `filings`,
`dividends`, `fund_profiles` + `fund_holdings`, `fund_metadata`, `leadership`,
`endpoint_guard`, `data_status`, `fetch_log`. **New in Phase C:** a session
watchlist table (sid -> tickers).

`kind` values: `stock`, `etf`, `index`, `future` (commodities/futures).

---

## Roadmap

Phases are ordered but reorderable; each is a context-clearing breakpoint that
ends verified + committed + deployed.

### Phase A — Strip background sweeps + remove Industries  ✅ DONE on dev (commit + deploy pending)
Make the app demand-only and delete the dead surface, before building the new one.
See the Status block above for the full outcome.
- **Gut the scheduler.** Remove the `daily_close`, `sec`, `dividends`,
  `fund_metadata`, `fund_nav`, `earnings_calendar`, `asset_profile`, and periodic
  `history` jobs and their bring-forward calls. The loop keeps: market-session
  broadcast, `run_intraday` (demand-driven), and `run_prune_if_due`. The
  `EndpointGuard` and `data_status` / `fetch_log` plumbing stay (the on-demand
  fetches in Phases B/C still record through them).
- **Stop the seed's automatic history backfill.** First-run seed still creates the
  universe rows (metadata only, local) via `sync_universe`, but no longer fetches
  any history over the network. A symbol's history is filled the first time it is
  viewed (Phase B).
- **Remove Industries entirely:** the `industries` route + `/api/industries`, the
  `industries` frontend entry, the `industries_*` templates, the home Industries
  band, and the nav link. Drop the industries-only compute if unused elsewhere.
- **Verify:** with no browser connected, the app makes zero outbound calls over a
  several-minute idle window (watch `fetch_log` / guard counters); `/industries`
  → 404; home still renders (old bands, trimmed of Industries) until Phase C
  replaces it; `/health` no longer lists the removed jobs.

### Phase B — On-demand symbol data (loading bar + data age + manual refresh)  ✅ DONE on dev (commit + deploy pending)
Turn the symbol page into a pull-on-demand surface. See the Status block above for
the full outcome.
- **On load:** if the symbol's quote / intraday / daily history is stale or absent,
  trigger a fresh Yahoo pull. Slow SEC data (fundamentals, filings, holdings, NAV)
  is pulled only when missing or past its staleness window. A manual **Refresh**
  button re-pulls everything.
- **Loading bar over the whole pull.** A clear progress indicator that names each
  step as it runs (prices → history → fundamentals → filings → ...), driven by the
  existing SSE stream or a dedicated per-symbol refresh endpoint that reports
  progress. Guard-denied steps surface plainly ("rate-limited, showing cached").
- **Data-age on every block.** Each figure/section shows a plain age read derived
  from its `*_synced_at` timestamp ("live" / "3m ago" / "as of Fri close" /
  "stale").
- **Respect the guard.** All pulls route through the `yahoo` / `sec` guards; an
  open breaker degrades gracefully to cached + a clear note, never a hammer.
- **Verify:** loading a cold symbol fills its chart + stats live with the bar
  advancing; data-age reads correctly; manual refresh re-pulls; a tripped guard
  shows cached-with-note; no console errors at mobile + desktop.

### Phase C — Dashboard rework: session watchlist + %-vs-SPX hero graph  ✅ DONE on dev (commit + deploy pending)
See the Status block above for the full outcome.
Rebuild home around the watchlist and the day graph.
- **Session watchlist.** An opaque `fin_sid` cookie identifies a browser session
  (no accounts); a `watchlist(sid, ticker, position, added_at)` table holds its
  symbols, seeded with VTI, VXUS, BND, IAU, IBIT on first visit. Add/remove UI on
  the dashboard. Clearing cookies loses the list (acceptable, by design). SPX is a
  fixed, non-removable baseline.
- **Hero graph = normalized %-vs-SPX overlay.** All watchlist symbols + SPX on one
  intraday chart, each as % change from today's open (TradingView/Google compare
  style), SPX as the visual baseline. The graph delineates pre / regular /
  after-hours and shows "closed" state clearly. Falls back to the most recent
  trading day on weekends/holidays.
- **Watchlist rows/cards** beneath: price, day change, a sparkline, and a data-age
  read each. Refresh ~every 5 minutes while the dashboard is open (demand-driven:
  the page registers its watchlist tickers with the interest registry; the
  intraday poll honors a ~5-minute per-symbol cadence for dashboard viewers).
- **Keep these market reads on the dashboard (user's call, 2026-06-03):**
  - **Overall market volume.** An aggregate market-volume read for the day (source
    TBD during build: the lead index's / a broad-ETF's volume, since we no longer
    sweep the whole universe for a true summed figure). Shown with its data-age.
  - **VIX, kept tracking.** ^VIX stays a first-class dashboard read (the risk tone),
    not dropped with the old hero verdict. It is one of the always-on reads even
    though it is not a watchlist symbol.
  - **SMA 50 / SMA 200.** Interesting in general. They stay as chart overlays on
    symbol pages (Phase B, daily ranges). On the dashboard they only make sense on a
    *daily* SPX chart, not on the intraday %-vs-SPX hero overlay; decide during the
    build whether the dashboard carries a small SPX daily chart with the 50/200 SMAs
    or whether 50/200 live only on symbol pages. (Open sub-decision.)
- **Strip the rest of the old home bands** that depended on whole-universe daily
  data we no longer fetch (breadth, stock movers, the quality leaderboard, the hero
  verdict sentence). The new dashboard is: session/hours banner + hero comparison
  graph + the volume/VIX reads + editable watchlist. (Revisit whether any other old
  band is worth keeping during the build.)
- **Verify:** first visit seeds the 5 starters; add/remove persists across reloads
  in the same browser and resets in a fresh one; the hero overlay normalizes all
  lines to % from open with SPX as baseline; market-hours labeling is correct in
  each session; watchlist refreshes on the ~5-minute cadence with live data-age;
  nothing polls once the dashboard tab is closed.

### Phase D — Cohesion + polish pass  ✅ DONE on dev (commit + deploy pending)
Removed the dead Summary machinery, trimmed `/health` to the demand-only job set +
copy, added a "Prices as of" age to the dashboard reads, and fixed a verdict
pluralization. (Industries links were already removed in Phase A; the loading bar,
market-hours banner, and mobile hierarchy landed in Phases B/C.) See the Status
block above for the full outcome. **This completes the demand-only roadmap.**

### Backlog / parked
- Named multiple watchlists (only a single session list is planned for now).
- A "popular non-S&P" quick-add set on the dashboard, if the curated catalog feels
  thin for adding symbols.
- Server-rendered fallback when JS is off (the hero graph + loading bar are JS).
- Whether to keep *any* whole-market read (breadth/movers) as an optional,
  on-demand-only panel rather than deleting it outright.

---

## Decisions log

**2026-06-03 — The "demand-only refocus" kickoff.** The user steered a focus shift
away from the broad always-on dashboard: the app could not source enough live data
to show what they wanted, and the timed sweeps were spending API budget on symbols
nobody was viewing. Answered 4 clarifying questions:
1. **Dashboard = a session-scoped editable watchlist.** Cookie-based, no accounts;
   custom to a browser "session" however we manage it (chosen impl: an opaque
   `fin_sid` cookie + a `watchlist` table). Clearing the browser loses it, which is
   fine. Seeded with VTI, VXUS, BND, IAU, IBIT; SPX is the fixed baseline.
2. **Hero graph = normalized %-vs-SPX overlay** (TradingView/Google compare style):
   every watchlist symbol + SPX as % change from today's open, SPX the baseline.
3. **Symbol pull scope = prices auto, deep data if stale.** On load, auto-pull fast
   Yahoo data (quote / intraday / daily history) live with a loading bar; pull slow
   SEC data (fundamentals / filings / holdings / NAV) only when missing or stale,
   each carrying a data-age read. Manual refresh re-pulls everything.
4. **Drop all timed network sweeps** (including `daily_close`). Nothing happens
   automatically. History is fetched on loading a page whose data is stale. Keep
   only the local prune and demand-driven polling. The universe CSV stays as a
   searchable catalog (metadata only; data fetched on first view).
Then riffed three reads to keep on the dashboard despite stripping the old bands:
**overall market volume**, **^VIX** (keep tracking), and **SMA 50/200** (interesting
in general). Budgeted into Phase C: volume + VIX are always-on dashboard reads; the
50/200 SMAs stay as symbol-page chart overlays, with an open sub-decision on whether
the dashboard also carries a daily SPX chart to host them (the intraday %-vs-SPX hero
overlay can't show daily SMAs meaningfully).
Roadmap rewritten into Phases A-D above. The previous "distill + ETF-first"
roadmap (Phases 1-7) is **superseded** but its outcomes remain live in prod until
each new phase replaces them; that history is condensed below and in git.

**Superseded — the "distill + ETF-first" rewrite (2026-05-30, Phases 1-7, deployed
`645b351`).** Condensed, since the demand-only refocus reworks much of it:
- **P1 Yahoo-only data layer:** removed the Stooq provider/guard/config; Yahoo
  `interval=1d&range=max` serves deep history. Fixed Yahoo `range=max`
  downsampling (provider 10y fallback + `run_history` self-heal). *Yahoo-only
  stays; the periodic history job it added is now being removed in Phase A.*
- **P2 Universe curation:** reconciled stocks to the current S&P 500 (exact match);
  curated ETFs to iShares + Vanguard (43); `seed::sync_universe` reconciles on every
  boot. *Universe stays as the search catalog; the boot history backfill is being
  dropped in Phase A.*
- **P3 Dropped short-horizon prediction → quality leaderboard:** removed the four
  pick rankers, `src/picks.rs`, `/backtest`, the snapshot job; dropped the `picks`
  table (migration `0013`). *The leaderboard band is being removed in Phase C.*
- **P4 ETFs first-class:** `compute::etf_quality` (cost/tracking/diversification/
  size blend), the ETF symbol-page quality donut, a daily `fund_nav` job +
  `nav_synced_at` (migration `0014`) + a NAV-freshness gate. *The quality read +
  ETF page identity stay; the daily `fund_nav` sweep is being removed in Phase A
  (NAV now fetched on demand for a viewed ETF).*
- **P5 Dashboard "how is the market TODAY":** hero verdict + breadth band + ETF
  band + bands layout. *Being replaced by the watchlist + %-vs-SPX dashboard in
  Phase C.*
- **P6 Symbol-page distillation + live intraday:** 1D/1W intraday range buttons
  (15m bars on a minute axis, prev-close reference line), live tick via a
  re-broadcast `finance:quote` event, mobile above-the-fold reorder. *The intraday
  chart + live tick stay and are the base for Phase B's on-demand pull.*
- **P7 Health distillation + footer + live breadth:** `/health` top systems
  verdict, two-tier Paper Ledger footer, `src/summary.rs` + `StreamEvent::Summary`
  live market summary. *The footer + `/health` stay (Phase D trims `/health` to the
  smaller job set); the live market summary/breadth push is being removed with the
  old dashboard.*

**Pre-2026-05-30 history (condensed).** The app shipped the original Phases 0-31:
MVP (universe, Stooq history, scheduler+guard, Paper Ledger redesign, live
quotes+SSE, health page, SEC fundamentals+filings, chart indicators, search+add-
symbol, commodities, home redesign, ship/Docker), then post-MVP work (leadership,
industry trends, ETF profiles, strongest/weakest, data-age captions, financials
table, dividends, earnings dates, anomaly feed, stock health read, ETF first-class
v1, top picks + backtest, UI polish). Blow-by-blow lives in git history.

---

## Hard-won lessons (don't relearn these)

- **Yahoo `range=max` silently downsamples `interval=1d`.** For symbols with long
  histories (futures, ^RUT/^VIX, multi-year ETFs) the v8 chart endpoint returns
  weekly/monthly bars even when a daily interval is asked for, and for some a single
  bar. A *bounded* window (`range=10y`, or explicit `period1`/`period2`) is served
  at true daily granularity. The provider detects a downsampled `range=max` response
  (median gap > 4 days) and refetches 10y; detect coarseness from *recent* spacing,
  not whole-span density (^SPX has daily data for decades but a sparse pre-1900
  tail). The on-demand history fetch in Phase B reuses this provider logic.
- **The seed reconciles the universe on every boot, not only first-run.**
  `sync_universe` (upsert + prune) runs each boot so a `starter.csv` edit takes
  effect on deploy. Pruning keys on `is_seeded = 1` so user-added symbols are safe.
  (Phase A keeps this; it removes only the *history backfill*, not the metadata
  sync.)
- **Yahoo `quoteSummary` is crumb-gated.** Must prime `fc.yahoo.com` + fetch
  `/v1/test/getcrumb` with cookies, cache the crumb, rotate on 401/403. Already in
  `src/providers/yahoo.rs`.
- **SEC's `fy` field tags the *filing's* fiscal year, not the period's.** Derive
  fiscal year from period-end + the company's fiscal-year-end month. Keep only clean
  full-year/discrete-quarter durations; skip quarterly balance-sheet figures.
- **"No data" / historyless symbols are a clean empty, not a failure** — never feed
  the breaker for a symbol Yahoo legitimately has no history for.
- **Guard state is shared across server + `seed` subcommand** via SQLite, and
  survives restarts. A boot-time breaker trip is normal after a deploy; it recovers
  via the half-open probe.
- **Chart indicator lines use a non-semantic palette on purpose** — green/amber/red
  are reserved for good/ok/bad; candles own green/red.
- **Yahoo NAV is only as fresh as you keep it.** Comparing a live price to a
  weeks-old NAV yields a meaningless premium/discount. The ETF quality read's
  tracking factor must gate on `nav_synced_at` freshness. In the demand-only model
  NAV is fetched when an ETF page is viewed and stale, and the freshness gate still
  drops the tracking factor to `—` rather than assert a bogus premium.
- **`cargo` isn't on PATH in this dev container** — use `~/.cargo/bin/cargo`, and
  run the dev binary with `FINANCE_ROOT=/home/dev/code/finance` (or from the project
  dir so paths resolve from cwd).
