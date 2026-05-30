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

_Last updated: 2026-05-30 (Phase 5 done on dev — commit + deploy pending)_

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

**Current work:** Phases 1–4 are **done, committed, and deployed** (Phase 4 is
live at `19d0b14`). **Phase 5 (dashboard redesign) is done on dev, pending commit
+ deploy.** Next after deploy: **Phase 6 (symbol-page distillation + live
intraday on chart).**

Phase 5 outcome (dashboard redesign → "how is the market doing TODAY"):
- **Hero verdict.** A two-line plain read at the top blending the broad index
  move, breadth, and the VIX risk tone into a lead + clause (e.g. "Higher, but
  narrow." / "Markets higher with narrow participation."), over a compact
  index-chip strip, with the headline figures (S&P %, % green, VIX tone) and a
  non-advice note. `build_hero` + `market_verdict` + `vix_tone` in
  `src/routes/home.rs`, fed by the already-loaded index/commodity cards + breadth
  (zero extra queries). **The verdict's direction tracks the broad index's sign**
  so it never contradicts the "S&P +x%" shown beside it; breadth only sets
  direction when no index price exists. A near-flat index reads "mixed" even when
  breadth skews. (Found + fixed during review: a +0.12% S&P with weak breadth had
  read "Broadly lower" — a direct contradiction; the deadband was tightened from
  ±0.15% to ±0.05% and the breadth fallback narrowed.)
- **Breadth band.** Advancers / decliners counts + % green over a single
  up/flat/down proportion bar. `breadth()` reuses the `load_stocks` scan (no
  extra query); a stock without a computable change is excluded so a missing
  quote never reads as "flat".
- **ETF band (the Phase-4 deferral, now built).** Five curated quality cards
  (`VOO`, `VTI`, `QQQ`, `BND`, `GLD`) — intraday sparkline + day move + the
  Phase-4 quality verdict pill — over a compact gainers/losers strip across the
  whole curated ETF set, each pill dotted by quality grade. `load_etfs` rolls
  every seeded ETF (price + Yahoo metadata + SEC AUM + top-10 concentration) into
  the `etf_quality` read, reusing the same NAV-freshness gate as the symbol page.
- **Band order:** Hero · Indexes · Breadth · ETFs · Stock movers · Industries ·
  Risk & commodities · Quality leaderboard. The existing Industries (sector
  up/down) panel was **kept as its own band** (user's call), not folded into
  breadth.
- **Verified on dev:** `cargo build` + `vite build` clean; home renders all bands
  at desktop (1280) and mobile (390) with no console errors; hero reads
  consistently with the figures beside it; breadth counts reconcile
  (197 adv / 306 dec = 39% green); ETF cards show quality pills and the movers
  strip ranks correctly (screenshots reviewed, then deleted). Because today is a
  weekend the index cards correctly resolve to futures (ES=F, …) and breadth
  dates to the prior close.
- **Known limitation (Phase 7 polish candidate):** the hero verdict and breadth
  are page-load snapshots — the sparkline cards still stream live, but the
  verdict sentence + breadth counts do not recompute intraday. Live breadth would
  need a server-pushed breadth event on the stream hub.

Phase 3 outcome (drop short-horizon prediction → quality leaderboard):
- **The whole picker is gone.** Removed the four horizon rankers
  (`pick_day/week/month/quarter` + `PickInput`), `src/picks.rs`, the `/backtest`
  route + page + JS, the scheduler's once-a-day snapshot job, and the
  backtest-only `models::latest_annual_inputs_as_of` / `FILING_LAG_DAYS`.
  Migration `0013_drop_picks.sql` drops the `picks` table and sweeps its stale
  `data_status` / `fetch_log` / `meta` rows so `/health` doesn't show a frozen
  "picks" job.
- **Three overlapping home panels merged into one Quality leaderboard.** Top
  picks, Strongest & weakest, and Stock health all collapsed into a single
  non-advice "Healthiest / Most concerning" surface driven by the existing
  `compute::health_read` composite (fundamentals + trajectory + leadership
  stability). The old strongest/weakest panel's trailing-year return is folded
  into each leaderboard row as a quiet price anchor; the non-advice disclaimer
  rides on the section.
- **`compute::standing` kept** — the movers panel still uses it for each row's
  strength badge, and the symbol + search pages use it too. Only the home
  *strongest/weakest panel* was removed, not the standing read itself.
- **Verified:** `cargo build` + `vite build` clean (no `backtest` entry);
  migration applied on boot (picks table dropped, zero stale picks rows,
  `data_status` job list no longer lists picks); `/backtest` + `/api/backtest`
  → 404; `/api/health` → 200 with no picks job; home renders the leaderboard
  with trailing returns and none of the removed panels (screenshot reviewed).

Phase 2 outcome (universe curation, deployed in `b016b25`):
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

### Phase 3 — Drop short-horizon prediction → quality leaderboard + home de-dup  ✅ DONE (deployed 2026-05-30)
- ✅ Removed all four pick rankers (Day/Week *and* Month/Quarter), the backtest
  machinery, the scheduler snapshot job, and the backtest-only models helpers.
- ✅ Merged Top picks + Stock health + Strongest & weakest into a single
  non-advice **Quality leaderboard** (Healthiest / Most concerning) driven by
  `compute::health_read`, trailing-year return folded into each row.
- ✅ Dropped the `picks` table via migration `0013_drop_picks.sql` (+ swept
  stale status rows). `compute::standing` retained for the movers badge +
  symbol/search pages.
- Note: the leaderboard's home-page placement is intentionally provisional —
  Phase 5 rebuilds the dashboard (hero + bands) around it.

### Phase 4 — ETFs as true first-class citizens  ✅ DONE (deployed 2026-05-30, `19d0b14`)
- ✅ Blended ETF **quality score** (`compute::etf_quality`): cost-weighted blend
  of cost (40%) / tracking (25%) / diversification (20%) / size (15%), composite
  −1..1 → percent, four sub-reading chips — structurally mirrors the stock
  `health_read` donut. Renormalises over gradeable factors (commodity trusts
  with no holdings drop diversification cleanly); shows only with ≥2 factors.
- ✅ Distinct ETF symbol-page identity: the quality donut anchors the header
  top-right (reusing the `health-badge` styling), hover reveals the four
  sub-readings + non-advice note. The ETF already shows fund sections (holdings,
  expense/yield, NAV/premium, sector/geo, trailing returns vs benchmark) and no
  stock fundamentals — so the badge + `ETF` tag complete the distinct identity.
- ✅ **Tracking now backed by a real daily NAV.** Discovered the 30-day metadata
  cadence made premium/discount unreliable; added a dedicated daily `fund_nav`
  scheduler job + `nav_synced_at` column (migration `0014`) so NAV is current,
  and a freshness gate that drops the tracking factor to `—` when NAV is stale
  rather than assert a bogus premium. (See Decisions log for the full story.)
- ⏸ **Distinct ETF band on the dashboard — deferred to Phase 5** (user's call):
  the dashboard is fully redesigned in P5, so the ETF band is built there once
  rather than built twice.
- **Verified on dev:** `cargo build` clean; migration `0014` applies on boot;
  `/health` lists the new "ETF NAV" job; every ETF page renders the quality
  donut (VOO 89% Strong, GLD 64% with diversification correctly `—`, BND 100%
  on its two graded factors); the freshness gate correctly drops tracking to
  `—` while NAV is stale (screenshot reviewed). **One runtime path still
  unverified live:** the daily `fund_nav` Yahoo fetch — the dev `yahoo` guard's
  breaker was open (normal post-deploy back-off) so the job stopped early 0/43;
  the fetch reuses the proven `quoteSummary` NAV parse, and prod exercises it on
  first deploy once its guard is healthy.

### Phase 5 — Dashboard redesign: "how is the market doing TODAY"  ✅ DONE on dev (commit + deploy pending)
- ✅ Hero: two-line plain-language verdict (blended index move + breadth + VIX
  tone) + compact index strip + headline figures + non-advice note. Direction
  tracks the broad index sign so it never contradicts the figure shown.
- ✅ Breadth band: advancers/decliners + % S&P green + a proportion bar. (Sector
  leaders/laggards stay in their **own** Industries band, per the user's call —
  not folded into breadth.)
- ✅ ETF band (the Phase-4 deferral): 5 curated quality cards + a gainers/losers
  strip, each carrying the Phase-4 quality verdict.
- ✅ Clearly labeled bands, dual-first density: Hero · Indexes · Breadth · ETFs ·
  Stock movers · Industries · Risk & commodities · Quality leaderboard.

### Phase 6 — Symbol-page distillation + live intraday on chart
- Mobile above-the-fold order: price/change → health verdict → mini chart →
  trajectory. Desktop denser; health read is the clear hero.
- A viewed fund during market hours shows today's real-time intraday on its
  chart (current day), stitched onto the daily series.

### Phase 7 — Health/systems page distillation + final polish pass
- Distill `/health` and overall cross-page cohesion; one closing UI polish pass.
- **Expand the footer to match the sibling-project pattern.** Finance currently
  has a single-line footer; the user wants it grown "a lot" to match how the
  other apps do footers. The house pattern (analytics, status, blog, repos) is a
  **two-tier footer**: a multi-column upper `<footer>` (columns like About /
  Pages / Links — nav links, cross-project links, Portfolio/GitHub/LinkedIn, and
  a "Source" link to `github.com/overshard/finance`) over a slim `.footer-bar`
  with `© {{ now.year }} Isaac Bythewood · Some rights reserved` + a GitHub icon
  link. `repos`'s `footer__grid` with `// LABEL` column headers is the closest
  fit for the Paper Ledger aesthetic — model finance's on that. Fold the data
  attribution below into one of the columns.
- **Data-attribution (partially done early):** the stale **Stooq** credit was a
  factual bug (Stooq removed Phase 1), so the one-line footer was corrected
  on 2026-05-30 to "Market data via Yahoo Finance · Fundamentals via SEC EDGAR ·
  not investment advice" ahead of schedule. The full footer build above still
  belongs to this polish pass. Yahoo's chart endpoint is unofficial (no
  published ToS/attribution requirement), but a tasteful credit is honest and
  suits the professional face the user wants.

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
- **Scrub Claude/Anthropic trailers from git history (cross-repo, force-push).**
  User wants every `Co-Authored-By: Claude`, `🤖 Generated with [Claude Code]`,
  and any Anthropic ad line removed from *all* commit messages in *all* repos.
  **Survey 2026-05-30: history is already clean** — a strict scan across all 9
  `~/code` repos (analytics, blog, darkfurrow, finance, isaacbythewood, repos,
  status, taproot, timelite) found **zero** such trailers; the only "claude"
  hits are legitimate references to the `CLAUDE.md` *filename* in commit
  messages, which must NOT be scrubbed. So this is a no-op today and only
  matters if a trailer ever slips in. Procedure if needed (do it as its own
  focused session — it is irreversible):
  1. Per repo, rewrite history dropping the offending trailer lines, e.g.
     `git filter-repo --message-callback` (preferred) or a `filter-branch`
     fallback, stripping only `Co-Authored-By: Claude*`, `🤖 Generated with*`,
     and `Generated with [Claude Code]*` lines — never the `CLAUDE.md` mentions.
  2. `git push --force-with-lease` to **every** remote (GitHub `origin` *and*
     the deploy remote `server`).
  3. **Server fixup:** the deploy is a bare repo + post-receive hook that builds
     into the project dir. After a history rewrite the server's checkout will be
     on an orphaned commit, so SSH to the alpine host (`taproot` manages it) and
     reset the bare repo's `master` + re-run the deploy (`docker compose up
     --build --detach`) so `/srv` tracks the rewritten history; verify the
     container is healthy and reattached to `bythewood-edge`. Coordinate via the
     `taproot` repo (its CLAUDE.md is off-limits to auto-edits per user rule).

---

## Decisions log

**2026-05-30 — Phase 5 (dashboard redesign).** Answered 4 design forks before
building:
1. **Hero verdict = blended + a touch more.** A two-line read: a punchy lead
   ("Higher, but narrow.") plus a clause ("Markets higher with narrow
   participation."), blending the broad index move, breadth (% green), and the
   VIX risk tone, with the headline figures and a non-advice note beneath.
2. **Sectors stay separate.** Breadth band = advancers/decliners + % green only;
   the existing Industries (sector up/down) panel keeps its own band rather than
   folding "sector leaders/laggards" into breadth.
3. **ETF band = both.** Curated quality cards (VOO/VTI/QQQ/BND/GLD) *and* a
   compact ETF gainers/losers strip — this is the Phase-4 deferral, now built.
4. **Breadth viz = stat strip + proportion bar** (advancers/decliners counts, %
   green, one up/flat/down bar), not a distribution histogram.
Implemented entirely in `src/routes/home.rs` + `home.html`/`macros.html` +
`home.scss`, reusing the already-loaded card/stock scans so the hero and breadth
add **zero** extra queries (only the ETF band adds two: the ETF roll-up and a
top-10-holdings window query). Found + fixed during the screenshot review: the
verdict could contradict the figure beside it — a +0.12% S&P with weak breadth
read "Broadly lower" because the ±0.15% direction deadband swallowed the move and
let breadth flip the sign. Fixed by tightening the deadband to ±0.05% and making
direction track the broad index's sign, with breadth breaking ties only when no
index price exists. Deferred live updates for the hero/breadth (page-load
snapshots) to the Phase 7 polish pass.

**2026-05-30 — Phase 4 build + the NAV-staleness discovery.** Built the ETF
quality read (`compute::etf_quality`, mirroring `health_read`) and the
symbol-page quality donut. **Mid-build discovery:** the tracking factor
(premium/discount to NAV, the option the user picked) was reading "Wide gap" on
funds that track perfectly (VOO, IVV) because Yahoo's NAV is only refreshed
every 30 days (`FUND_METADATA_STALE_SECS`) — so a live price compared to a
weeks-old NAV showed a bogus premium. This broke the premise behind the user's
tracking choice. Surfaced it and asked; user chose **refresh NAV daily** so
tracking becomes a true daily read. Implemented as:
- a dedicated daily `fund_nav` scheduler job (separate from the 30-day static
  metadata sweep so the slow fields keep their cadence) that fetches NAV only
  (`YahooProvider::fund_nav`, two `quoteSummary` modules) through the `yahoo`
  guard — ~43 req/day, negligible vs the 1000/hr budget;
- a `nav_synced_at` column (migration `0014`) the daily job stamps;
- a **freshness gate** on the symbol page: the tracking factor is graded only
  when NAV was synced ≤3 days ago, else it drops to `—` and the cost/div/size
  blend renormalises — so a stale NAV never drives a false tracking verdict.
Also deferred the **dashboard ETF band to Phase 5** (it rebuilds the dashboard
anyway) and corrected the footer's stale Stooq credit early (a factual bug).
The `fund_nav` live fetch wasn't exercised on dev (the guard's breaker was open
post-deploy), but it reuses the proven NAV parse and runs on first prod deploy.

**2026-05-30 — Phase 4 (ETFs as first-class citizens) design forks resolved.**
Answered 3 clarifying questions before building:
1. **ETF quality score = cost-weighted, all four factors.** Cost (expense ratio)
   ~40%, Tracking ~25%, Diversification ~20%, Size (AUM) ~15%. Composite in
   [-1,1] → percent, with four sub-reading chips — structurally mirrors the
   stock `health_read` donut. Weights renormalize over whichever factors are
   gradeable (commodity trusts like GLD/SLV/IBIT have no holdings, so
   diversification drops out and the rest reweight). Show the badge only when
   ≥2 factors are gradeable.
2. **Tracking factor = premium/discount to NAV**, reusing the existing
   `compute::premium_discount_pct` + `premium_grade` (price vs Yahoo NAV). No
   new compute, no benchmark-alignment work. (True tracking error vs benchmark
   was considered and parked.)
3. **Dashboard ETF band deferred to Phase 5.** Phase 4 ships the quality score +
   the distinct ETF symbol-page identity (own quality donut + sub-readings in
   the header, alongside the fund sections that already exist). The dashboard
   ETF band gets built into the Phase 5 dashboard redesign rather than built
   twice.
Grading bands (initial, tune later): cost cheap ≤0.10% / pricey >0.50%;
tracking via `premium_grade` (±0.25% tight, ±1% wide); diversification on top-10
holdings concentration (≤~25% broad, >~50% concentrated); size on log10(AUM)
centered ~ $2B ok, ≥ $20B large.

**2026-05-30 — Phase 3 (drop short-horizon prediction → quality leaderboard).**
Executed the kickoff decision to drop prediction. Resolved the one open design
fork (what the unified leaderboard ranks by) without blocking: used the existing
`health_read` composite — it is the "healthiest" read the plan named and already
blends strength + trajectory + leadership stability — rather than inventing a new
score or reusing `standing`. Folded the old strongest/weakest panel's
trailing-year return into each leaderboard row so no useful signal was lost.
Kept the home-page treatment deliberately light (a straight three-into-one
de-dup) because Phase 5 redesigns the dashboard around this band anyway. Found
during the work: `movers` reuses `compute::standing` for its strength badge, so
`standing` stays (only the strongest/weakest *panel* was removed); and the
running dev server had to be restarted to pick up the new binary + run the
drop-picks migration.

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
- **Yahoo NAV is only as fresh as you keep it.** The 30-day `fund_metadata`
  sweep (`FUND_METADATA_STALE_SECS`) carries a NAV, but comparing a live price
  to a weeks-old NAV yields a meaningless premium/discount — it reads as a huge
  fake "premium" on funds that actually track to the basis point. NAV is struck
  once per trading day; anything that reads a price-vs-NAV premium (the ETF
  quality read's tracking factor) must run off the daily `fund_nav` refresh and
  gate on `nav_synced_at` freshness. Don't trust the metadata sweep's NAV for
  any real-time comparison.
- **`cargo` isn't on PATH in this dev container** — use `~/.cargo/bin/cargo`,
  and run the dev binary with `FINANCE_ROOT=/home/dev/code/finance` (or from the
  project dir so paths resolve from cwd).
