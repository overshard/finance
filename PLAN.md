# finance — Project Plan and Resume Doc

`finance` is a self-hosted, real-timeish market-watching web app for stocks,
ETFs, indexes, and commodities: live charts, key stats, fundamentals, SEC
filings, and at-a-glance health reads. A single Rust + axum binary backed by
SQLite, with a Vite frontend. Deploys at `finance.bythewood.me`, published on
GitHub as `finance`.

It is for *watching* the market only. No portfolio, no holdings, no money or
cost-basis tracking, no accounts, no auth. Single operator. **Not investment
advice** — every synthesized read carries that disclaimer.

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

_Last updated: 2026-05-30 (Phase 2 complete on dev)_

**Major refactor in progress (the "distill + ETF-first" rewrite).** This plan
was fully rewritten 2026-05-30 from a sprawling 3,700-line resume doc into this
focused roadmap. The decisions driving it are in the Decisions log under
2026-05-30; the short version:

- **Short-horizon prediction is being dropped.** Next-day / next-week "picks"
  are a coin-flip gamble (the backtest's own ~50% win rates prove it) and they
  drove most of the live-data demand that tripped the rate limits. Day/Week go;
  Month/Quarter are reframed as a non-advice *quality leaderboard*.
- **Data is going Yahoo-only.** Stooq is being removed entirely (its bulk
  download is CAPTCHA-gated and unscriptable; its per-symbol API has an
  undocumented daily-hit cap that kept blocking us). Yahoo serves both deep
  history and daily updates. See **Data-source policy**.
- **ETFs become true first-class citizens** with their own identity and an ETF
  "quality" read, clearly separated from stocks.
- **Everything gets distilled** into a fast-scannable, dual-first (mobile +
  desktop) design while keeping the futuristic-clean "Paper Ledger" look.

**Current work:** Phase 2 (universe curation) is **complete and verified on the
dev box**; commit + deploy pending. Next: **Phase 3 (drop short-horizon
prediction → quality leaderboard)**.

Phase 2 outcome:
- **Stocks reconciled to the current S&P 500.** Fetched the live Wikipedia
  constituent list (503) and diffed against ours: an exact match, zero changes
  needed — the stock list was already current.
- **ETF roster curated to iShares + Vanguard (43 ETFs).** Dropped the
  other-issuer thematic/sector funds (SMH, ARKK, SCHD, XLK/XLF/XLE/XLV); kept
  the SPY/QQQ/DIA/GLD/SLV proxies; added the most-common Vanguard + iShares
  funds incl. the core holdings IAU, IBIT, VBIL. Issuer/category tags come from
  the existing Yahoo `fund_metadata` (no new schema) per the user's choice.
- **Seed now reconciles on every boot.** `seed::sync_universe` (upsert + prune)
  runs each boot via `run_boot_seed`, so a CSV edit (symbols added or dropped)
  takes effect on deploy without a manual re-seed. Pruning is `is_seeded = 1 AND
  ticker NOT IN (csv)`, cascading cleanly; user-added symbols (`is_seeded = 0`)
  are never touched. This also swept up 7 stale rows an older non-pruning seed
  had left behind (MSTR, NET, RIVN, SHOP, SNOW, SOFI, MRVL — popular names from
  a pre-S&P-narrowing universe; none are current S&P 500 members). **If the user
  wants those back, that's a separate "popular non-S&P watchlist" decision (see
  backlog).**
- **Fixed a Phase-1 data-quality bug: Yahoo `range=max` downsampling.** Yahoo
  silently returns weekly/monthly bars for `interval=1d&range=max` on symbols
  with long histories (all the futures, ^RUT/^VIX, and every freshly-backfilled
  multi-year ETF were monthly; some were 1-bar). Two-part fix: the provider
  refetches a bounded `range=10y` window (which Yahoo serves at true daily
  granularity) when it detects a downsampled response; and `run_history`
  self-heals stored coarse/single-bar series (recent-density test) by replacing
  them with a clean daily re-fetch — so **prod self-heals on deploy**. Verified:
  every seeded symbol now holds genuine daily bars (median gap 1 day), no
  symbol is 1-bar, ^SPX's deep daily history (back to 1789) is preserved, and
  the density test flags zero false positives across all 562 symbols.

**Dev server:** kept running in the background via `make` during sessions so
the user can review progress live.

---

## Design principles

**"Paper Ledger" look (keep it).** An old-school accounting ledger reimagined
futuristic and clean: warm-paper background, ink-dark text, hairline rules,
monospace ledger figures, restrained serif headings. Tokens are CSS custom
properties in `base.scss :root`.

**Color is semantic and sparing.** Green / amber / red mean good / ok / bad
(price moves, health reads, data-health states) — never decoration. Chart
indicator lines are the one deliberate exception (a muted non-semantic palette).

**Scannability is the bar.** The user must be able to:
- Land on the **dashboard** and tell *how the market is doing TODAY* in one
  glance — a one-line plain verdict + the index strip + market breadth.
- Land on a **stock** and immediately read its health, trajectory, and key
  figures without hunting.
- Land on an **ETF** and immediately read what it holds, what it costs, and how
  it's trending — with a clear visual separation from stocks.

**Dual-first, not mobile-first-only.** Desktop is information-dense and should
*use* its space; mobile distills to the key signals (clear hierarchy, nothing
important below a second scroll). Neither is an afterthought.

**Polish last.** Features land first; one focused UI polish pass closes each
visual phase rather than nibbling polish mid-build.

---

## Data-source policy (the important reference)

All data is **free, no account, no API key.** The user considers *never hitting
a rate limit* critical: every outbound call goes through a persistent
`EndpointGuard` (DB-backed reactive circuit breaker + hard per-hour budget +
request pacing; survives restarts; see `src/guard.rs`).

**Yahoo Finance is the only price source (as of 2026-05-30; Stooq removed).**
- **Deep daily history:** `v8/finance/chart?interval=1d&range=max` returns a
  symbol's entire daily OHLCV in one call. Used once per symbol to backfill
  `daily_prices`.
- **Daily updates:** the once-a-day `daily_close` job already touches every
  symbol; it appends that day's bar to `daily_prices` (no extra requests).
- **Intraday + live quotes:** `v8/finance/chart?interval=15m&range=1d`.
- **ETF / fundamentals metadata:** `v10/finance/quoteSummary` (crumb-gated; the
  provider does the `fc.yahoo.com` primer + `getcrumb` dance, caches the crumb,
  rotates on 401/403). Modules: `fundProfile`, `calendarEvents`, `assetProfile`.
- Budget: 1000 req/hr on the `yahoo` guard. 429/503/401/403 surface as the
  typed `RateLimited` the guard trips on.

**SEC EDGAR** (no key, contact email in User-Agent; 600/hr guard): stock
fundamentals (`companyfacts`), filings (`submissions`), leadership (Form 3/4/5),
ETF holdings/AUM (N-PORT, `company_tickers_mf.json`). Indexes don't file.

**Freshness tiers (deliberate, to stay on-budget):**
- **Live intraday (SSE-polled):** ONLY the dashboard's headline indexes and the
  single symbol whose page is currently open. Demand-driven via the viewer-
  interest registry in `src/stream.rs` — nothing is polled when nobody's
  watching.
- **Daily close:** the entire rest of the universe. One snapshot per trading day.
- A viewed fund during live market hours shows today's real-time intraday on its
  chart (Phase 6).

---

## Architecture as built (condensed)

Single-binary axum app. `src/main.rs` (env init, `seed` subcommand, boot) →
`src/app.rs` (`AppState` + `Config` + `router()`). Async sqlx + SQLite (WAL);
migrations in `migrations/` applied on boot.

- **`src/providers/`** — one trait per concern: `QuoteProvider` /
  `HistoryProvider` (Yahoo), `FundamentalsProvider` (SEC). `http.rs` builds the
  shared reqwest clients. (Stooq provider removed in Phase 1.)
- **`src/guard.rs`** — the persistent per-endpoint `EndpointGuard` (see policy).
- **`src/scheduler.rs`** — one long-lived 60s-tick tokio task running
  market-hours-aware jobs (history backfill, daily_close, demand-driven
  intraday, SEC sweep, dividends, fund_metadata, earnings, asset_profile,
  prune). Each writes `data_status` + `fetch_log` and pings the stream hub.
- **`src/stream.rs`** — `tokio::broadcast` hub + per-ticker viewer-interest
  registry; `/stream` SSE forwards quote / market / health events.
- **`src/market.rs`** — US session clock (Closed/Pre/Regular/Post) via
  `chrono-tz`. No holiday calendar (deliberate).
- **`src/compute.rs`** — pure numeric code: indicators (sma/ema/rsi), graded
  fundamental ratios, health read, range-meter positions, sparkline SVG.
- **Templates** — minijinja in `templates/` with a Jinja2-faithful HTML
  formatter (so `/` isn't escaped in URLs). **Frontend** — Vite from
  `frontend/static_src/` (entries: base, home, symbol, health, search), built
  with bun, served hashed at `/static/`.

**Key tables:** `symbols` (universe + denormalized latest price/snapshot +
per-source `*_synced_at` staleness columns), `daily_prices` (permanent deep
OHLCV), `intraday_bars` (15m, pruned 14d), `quotes`, `fundamentals` (long/narrow
SEC facts), `filings`, `dividends`, `fund_profiles` + `fund_holdings` (ETF
N-PORT), `fund_metadata` (ETF Yahoo data), `leadership`, `picks` (being
reworked), `endpoint_guard`, `data_status`, `fetch_log`.

`kind` values: `stock`, `etf`, `index`, `future` (commodities/futures).

---

## Roadmap

Phases are ordered but reorderable; each is a context-clearing breakpoint that
ends verified + committed + deployed. Pain-point mapping to the user's brief:
data/guardrails → P1; ETFs first-class → P4; distill/cohesion → P3,P5,P7;
drop short-horizon → P3; live intraday for viewed fund → P6.

### Phase 1 — Yahoo-only data layer  ✅ DONE (deployed 2026-05-30, `76f38f4`)
Kill the rate-limit problem at the root.
- Remove the Stooq provider, the `stooq` `EndpointGuard`, `STOOQ_APIKEY`
  config, the per-symbol Stooq history job, and the seed's Stooq backfill path.
- Add `YahooProvider::daily_history` (`interval=1d&range=max`) → full daily
  OHLCV; route through the `yahoo` guard with pacing.
- Seed/backfill `daily_prices` from Yahoo for symbols missing deep history
  (one-time, paced under 1000/hr).
- `daily_close` appends the day's bar to `daily_prices` from data it already
  fetches (no extra requests), so there's no recurring per-symbol history sweep.
- Verify: guard never trips during a full backfill; `daily_prices` populates;
  charts render; `/health` shows no stooq endpoint.

### Phase 2 — Universe curation  ✅ DONE on dev (commit + deploy pending)
- ✅ Reconciled the 503 stocks to the current S&P 500 (fetch + diff: exact match).
- ✅ Curated ETFs to iShares + Vanguard (43 total): dropped other-issuer
  thematic/sector funds, kept SPY/QQQ/DIA/GLD/SLV proxies, added the
  most-common Vanguard + iShares funds incl. core holdings IAU/IBIT/VBIL.
  Issuer/category tags reuse the Yahoo `fund_metadata` (no new schema).
- ✅ Kept the 6 indexes and all 10 futures (4 index-futures + 6 commodities).
- ✅ `seed::sync_universe` now reconciles (upsert + prune) on every boot, so CSV
  edits take effect on deploy. Pruned 7 stale non-S&P leftovers in passing.
- ✅ Fixed Yahoo `range=max` downsampling (provider 10y fallback + `run_history`
  self-heal); every seeded symbol now holds true daily bars.

### Phase 3 — Drop short-horizon prediction → quality leaderboard + home de-dup
- Remove Day/Week picks and the short-horizon backtest machinery.
- Reframe Month/Quarter into a single non-advice **quality leaderboard**
  ("healthiest / strongest right now"), merging the overlapping home panels
  (Top picks, Stock health Healthiest/Concerning, Strongest & weakest) into one
  coherent surface. Slim or drop the `picks` table + `/backtest` accordingly.

### Phase 4 — ETFs as true first-class citizens
- A blended ETF **quality score** (cost / diversification / size / tracking)
  mirroring the stock health donut, with its own sub-readings.
- Distinct ETF symbol-page identity: own header treatment + badge + section set
  (holdings, expense/yield, NAV/premium, sector/geo, trailing returns vs
  benchmark) instead of stock fundamentals.
- Distinct ETF band on the dashboard.

### Phase 5 — Dashboard redesign: "how is the market doing TODAY"
- Hero: one-line plain-language market verdict + index strip + **breadth**
  (advancers/decliners, % of S&P green, sector leaders/laggards).
- Clearly labeled bands: Indexes · Breadth · ETFs · Stock movers · Commodities ·
  Quality leaderboard. Dual-first density.

### Phase 6 — Symbol-page distillation + live intraday on chart
- Mobile above-the-fold order: price/change → health verdict → mini chart →
  trajectory. Desktop denser; health read is the clear hero.
- A viewed fund during market hours shows today's real-time intraday on its
  chart (current day), stitched onto the daily series.

### Phase 7 — Health/systems page distillation + final polish pass
- Distill `/health` and overall cross-page cohesion; one closing UI polish pass.
- Add a discreet data-attribution line ("Market data via Yahoo Finance ·
  Fundamentals via SEC EDGAR") in the footer / on `/health`. Yahoo's chart
  endpoint is unofficial (no published ToS or attribution requirement to
  satisfy), but a tasteful credit is honest, costs nothing, and suits the
  professional face the user wants. (Captured 2026-05-30 from a user note.)

### Backlog / parked
- Watchlists (tables exist, unused — user wants an opinionated no-customization
  view for now).
- **Popular non-S&P watchlist.** Phase 2 narrowed stocks to exactly the S&P 500,
  which dropped some popular names the universe used to carry (MSTR, NET, RIVN,
  SHOP, SNOW, SOFI, MRVL). Add them back as an opt-in "popular / most-watched"
  band if the user wants them.
- Issuer-direct ETF feeds (iShares/Vanguard) if Yahoo/SEC prove thin.
- Deep pre-2000 history (lost with Stooq; revisit only if charts feel thin).
  Note: index/futures daily history via Yahoo caps at ~10y (the `range=10y`
  fallback); ^SPX/^DJI/^NDX still carry deep daily history from before.

---

## Decisions log

**2026-05-30 — Phase 2 (universe curation).** Answered 4 clarifying questions:
1. **ETF roster:** keep iShares + Vanguard + the SPY/QQQ/DIA/GLD/SLV proxies;
   drop other-issuer thematic/sector funds; add the most-common Vanguard +
   iShares funds. Landed at 43 ETFs.
2. **Core holdings:** "get the most common Vanguard and iShares funds" (not just
   the named IAU/IBIT/VBIL) — drove the broadened ETF set above.
3. **ETF tags:** reuse Yahoo `fund_metadata.category` / `fund_family` for the
   dashboard band separation; no new schema/migration.
4. **S&P 500 recon:** fetch the live constituent list and diff to match exactly
   (result: already an exact match).
Plus, surfaced and fixed during verification: the seed never reconciled on
re-boot (now does, via `sync_universe`), and Yahoo `range=max` silently
downsamples `interval=1d` to weekly/monthly for long-history symbols (fixed in
the provider + a `run_history` self-heal). See Status for the full outcome.

**2026-05-30 — The "distill + ETF-first" refactor kickoff.** User steered a
broad refactor; answered 10 clarifying questions. Decisions:
1. **Drop next-day/next-week prediction.** Confirmed it's a gamble and the main
   driver of live-data demand. Day/Week picks removed; Month/Quarter reframed as
   a non-advice quality leaderboard.
2. **Universe = S&P 500 + major indexes + major commodities + iShares/Vanguard
   ETFs.** Commodities stay.
3. **Data goes Yahoo-only; Stooq removed.** Investigated the Stooq bulk
   download: it's CAPTCHA-gated (authorizes the PHP session, not a reusable
   token — verified live), so it can't be cron'd; the per-symbol apikey path is
   reusable but carries the undocumented "Exceeded the daily hits limit" cap
   that was blocking us. Yahoo `interval=1d&range=max` gives full daily history
   in one call and `daily_close` already touches every symbol, so Yahoo covers
   both backfill and updates with no rate-limit exposure. Trade-off: lose
   Stooq's ultra-deep pre-2000 history (a chart curiosity).
4. **Freshness tiers:** live intraday only for dashboard indexes + the open
   symbol; daily close for everything else.
5. **ETF separation:** distinct dashboard bands + distinct ETF symbol-page
   identity.
6. **ETF read:** a blended "quality" score (not a fake "company health").
7. **Dashboard hero:** one-line verdict + index strip + breadth.
8. **Mobile stock order:** price/change → health verdict → mini chart →
   trajectory.
9. **Cadence:** commit + auto-deploy each verified phase.
10. **This session:** rewrite PLAN.md (done), then start Phase 1.

**Pre-2026-05-30 history (condensed).** The app shipped Phases 0–31 of the
original roadmap: MVP (universe, Stooq history, scheduler+guard, Paper Ledger
redesign, live quotes+SSE, health page, SEC fundamentals+filings, chart
indicators, search+add-symbol, commodities, home redesign, ship/Docker), then
post-MVP work (company leadership, industry trends, ETF profiles, strongest/
weakest, data-age captions, financials table, dividends, earnings dates,
per-ticker anomaly feed, stock health read, ETF first-class v1, top picks +
backtest, and a full UI polish pass). It deployed to production at
finance.bythewood.me. This refactor reworks the prediction, data-source, ETF,
and distillation layers on top of that base. The blow-by-blow phase history
from before this rewrite lives in git history; it was intentionally dropped
from this doc to keep it scannable.

---

## Hard-won lessons (don't relearn these)

- **Yahoo `range=max` silently downsamples `interval=1d`.** For symbols with
  long histories (futures, ^RUT/^VIX, multi-year ETFs) the v8 chart endpoint
  returns weekly/monthly bars even when a daily interval is asked for — and for
  some it returned a single bar. A *bounded* window (`range=10y`, or explicit
  `period1`/`period2`) is served at true daily granularity. The provider detects
  a downsampled `range=max` response (median timestamp gap > 4 days) and refetches
  10y; `run_history` self-heals already-stored coarse/single-bar series (it flags
  a symbol with < 30 bars in its trailing 90 days, replacing them with a daily
  re-fetch). Detect coarseness from *recent* spacing, not whole-span density —
  ^SPX has daily data for decades but a sparse pre-1900 tail, and a whole-span
  test wrongly flags it.
- **The seed must reconcile on every boot, not only first-run.** `seed_completed`
  gates the history *backfill*, but `sync_universe` (upsert + prune) runs each
  boot so a `starter.csv` edit (symbols added or dropped) takes effect on deploy.
  Pruning keys on `is_seeded = 1` so user-added symbols are safe.
- **Yahoo `quoteSummary` is crumb-gated.** Must prime `fc.yahoo.com` + fetch
  `/v1/test/getcrumb` with cookies, cache the crumb, rotate on 401/403. Already
  implemented in `src/providers/yahoo.rs`.
- **SEC's `fy` field tags the *filing's* fiscal year, not the period's.**
  Derive fiscal year from period-end + the company's fiscal-year-end month, or
  comparatives shift by years. Keep only clean full-year/discrete-quarter
  durations; skip quarterly balance-sheet figures (10-Q mis-tags prior-year
  comparatives).
- **"No data" / historyless symbols are a clean empty, not a failure** — never
  feed the breaker for a symbol Yahoo legitimately has no history for.
- **Guard state is shared across server + `seed` subcommand** via SQLite, and
  survives restarts. A boot-time breaker trip is normal after a deploy that adds
  a new upstream-backed job; it recovers via the half-open probe.
- **Chart indicator lines use a non-semantic palette on purpose** — green/amber/
  red are reserved for good/ok/bad; candles own green/red.
- **`cargo` isn't on PATH in this dev container** — use `~/.cargo/bin/cargo`,
  and run the dev binary with `FINANCE_ROOT=/home/dev/code/finance` (or from the
  project dir so paths resolve from cwd).
