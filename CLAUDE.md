# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

`finance` is a self-hosted, real-timeish market watcher for stocks, ETFs, indexes, and futures: live charts, key stats, fundamentals, and SEC filings. Single axum binary with sqlx + SQLite and a Vite frontend.

It is for *watching* the market only. No portfolio, no holdings, no money or cost-basis tracking, no accounts, no auth. Single operator. Deploys at `finance.bythewood.me`.

## Commands

- **Dev server:** `make run` (Vite watch + `cargo run` concurrently on port 8000)
- **Production build:** `make build` (Vite assets + release binary at `target/release/finance`)
- **Run release binary:** `make start`
- **Seed the universe:** `make seed` (curated symbols + bulk daily history; idempotent and resumable). Same as `finance seed`
- **Docker build:** `docker build .`

In this dev container `cargo` is not on `PATH`; use `~/.cargo/bin/cargo`. Run the dev binary with `FINANCE_ROOT=/home/dev/code/finance` so it finds `templates/`, `dist/`, `migrations/`, and `universe/`.

There are no tests or linters configured.

## The demand-only model (read this first)

Nothing is fetched on a timer. With nobody on the site, the app makes **zero** outbound calls. Data is pulled **on demand** the first time a symbol is viewed (or when its stored copy is stale), and live intraday quotes are polled **only** for the symbols a browser is currently watching. This shapes the whole architecture: the scheduler does no network sweeps, history fills in lazily, and every figure on the page carries an honest data-age read ("live", "2m ago", "stale, refreshing", "as of Fri close") — a stale figure is never shown as if fresh.

The user considers **never hitting a rate limit** critical. Every outbound call passes through the persistent `EndpointGuard` (see Architecture). Treat the rate-limit-critical path with care; don't add eager fetching.

## Architecture

**Backend:** Single-binary axum app. A tiny `src/main.rs` does env init, subcommand dispatch (`seed`), and server boot; `src/app.rs` builds `AppState` + `Config` + the `router()`. Per-feature route modules under `src/routes/`. Async sqlx + SQLite (WAL) for all reads and writes; the schema in `migrations/` (0001–0015) is applied on boot via `sqlx::migrate!`.

**Providers (`src/providers/`):** One trait per concern — `QuoteProvider` + `HistoryProvider` (both Yahoo) and `FundamentalsProvider` (SEC EDGAR) — with one struct per source, so a source is swappable without touching callers. `http.rs` builds the shared reqwest clients. (Stooq was the original history source; dropped 2026-05-30 in favor of Yahoo-only.)

**Endpoint guard (`src/guard.rs`):** A persistent, per-endpoint `EndpointGuard` that every outbound data call passes through: a DB-backed reactive circuit breaker (trips on HTTP 429/503 at once or after a failure streak, exponential backoff, half-open probe recovery), a hard per-hour request budget, and request pacing. State lives in the `endpoint_guard` table, so it survives restarts and is shared by the server and the `seed` subcommand. Budgets: 1000 req/hr on the `yahoo` guard, 600 req/hr on `sec`.

**Scheduler (`src/scheduler.rs`):** One long-lived tokio task on a 60s tick. Since the demand-only refocus it does **no** timed network fetching. Each tick it only: broadcasts market-session changes (and re-pushes the dashboard summary, a local DB read, on a session flip), runs the demand-driven intraday quote poll (Yahoo quotes for just the symbols a browser is watching, via the stream hub's interest registry, at a ~5-minute per-symbol cadence), and prunes aged `intraday_bars` / `fetch_log` rows. All the old timed sweeps (daily-close, SEC, dividends, fund metadata, NAV, earnings, asset profile, periodic history) were removed; their data is now pulled on demand. The boot seed only reconciles the universe rows from the curated CSV (local, no network).

**On-demand pull (`backfill_symbol`):** The synchronous per-symbol pull used by the add-symbol route and the on-demand refresh — fetches a viewed symbol's stale/missing fast Yahoo data (quote / intraday / daily history) live behind a loading bar, and its slow SEC data (fundamentals / filings / holdings / NAV) only when missing or stale, each carrying a data-age read. Manual refresh re-pulls everything.

**Real-time (`src/stream.rs`):** A `tokio::sync::broadcast` hub that also carries a per-ticker viewer-interest registry. The `/stream` axum SSE endpoint registers the tickers a page shows and forwards quote / market / health events; the browser uses `EventSource` and patches the DOM in place. The registry is what makes intraday polling demand-driven: only the symbols a browser is currently viewing get fetched.

**Watchlist (`src/watchlist.rs`):** Session-scoped dashboard watchlists with no accounts. A browser is identified by an opaque `fin_sid` cookie; its symbols live in the `watchlist` table keyed on that sid (migration 0015). A brand-new browser is minted a sid and seeded with `STARTERS` (VTI, VXUS, BND, IAU, IBIT); an existing cookie's list is used as-is even when empty, so a user who clears it is not re-seeded. The S&P 500 baseline is *not* a row — the dashboard always shows it as the comparison baseline. Clearing cookies loses the list, by design.

**Market clock (`src/market.rs`):** The US equity session (Closed/Pre/Regular/Post) in `America/New_York` via `chrono-tz`. No exchange-holiday calendar (deliberate).

**Compute (`src/compute.rs`):** Pure numeric code — indicator maths (`sma`, `ema`, `rsi`, `supertrend`), graded fundamental ratios, range-meter marker positions, and the home-page sparkline SVG. The maths lives here, not in SQL or JS.

**Templates:** Jinja2 templates in `templates/` rendered by minijinja with a Jinja2-faithful HTML formatter so `/` is not escaped to `&#x2f;` (matches the sibling Rust apps). `vite_asset` resolves hashed asset names from `dist/.vite/manifest.json`.

**Frontend pipeline:** Vite (run from `frontend/`, built with bun) compiles `frontend/static_src/` into `dist/`, served at `/static/` with content-hashed filenames. Five entry points: `base` (shared shell + SSE client), `home`, `symbol`, `health`, `search`.

**Request logging:** `src/middleware.rs` prints `time METHOD STATUS latency path` per request with ANSI-colored status codes, and serves the themed 404.

## Design — "Paper Ledger"

An old-school accounting ledger reimagined futuristic and modern: warm-paper background, ink-dark text, hairline rules, monospace ledger figures, restrained serif headings. Tokens are CSS custom properties in `base.scss :root`. Built dual-first: desktop is information-dense and *uses* its space, mobile distills to the key signals; neither is an afterthought.

**Color is semantic and sparing.** Green / amber / red mean good / ok / bad (price moves, fundamental ratios, data-age states, data-health states), never decoration. Chart indicator lines are a deliberate exception (a muted non-semantic palette). **Supertrend is a second deliberate exception** (a user call): its trend colour *is* the signal, and up=green/down=red matches the price-move semantics, so it reuses the candle green/red.

**Scannability is the bar.** Land on the dashboard and read how your watchlist is doing against the market today in one glance; land on a symbol and read its price, trend, and key figures (with ages) without hunting.

**Polish last.** Features land first; one focused UI polish pass closes the work rather than nibbling polish mid-build.

## Layout

```
finance/
├── Cargo.toml, Cargo.lock        # rust deps
├── Makefile, README.md           # README has the full config-env table
├── migrations/                   # sqlx migrations 0001-0015, applied on boot
├── universe/starter.csv          # curated seed list (~144 symbols)
├── src/
│   ├── main.rs        # entry: env init, `seed` subcommand, server boot
│   ├── app.rs         # AppState + Config + router()
│   ├── db.rs          # SqlitePool init, migrate, now_ms, meta helpers
│   ├── render.rs      # template render helper
│   ├── middleware.rs  # request log + themed 404
│   ├── templates.rs   # minijinja env, vite_asset, jinja2-compat formatter
│   ├── models.rs      # row structs + the shared ticker `Card`
│   ├── compute.rs     # indicator maths, graded ratios, sparkline SVG
│   ├── seed.rs        # universe reconcile + on-demand backfill_symbol
│   ├── scheduler.rs   # demand-only background loop (no timed sweeps)
│   ├── market.rs      # US market-session clock
│   ├── stream.rs      # SSE pub/sub hub + viewer-interest registry
│   ├── guard.rs       # persistent per-endpoint EndpointGuard
│   ├── watchlist.rs   # session (fin_sid cookie) watchlist storage
│   ├── providers/     # mod.rs (traits), http.rs, yahoo.rs, sec.rs
│   └── routes/        # home, symbols, watchlist, search, stream, health, seo
├── templates/         # base.html, includes/, pages/
├── frontend/static_src/   # base/ home/ symbol/ health/ search/ (Vite entries)
├── dist/              # vite build output (gitignored, served at /static/)
├── data/              # sqlite db at runtime (gitignored)
├── target/            # cargo build output (gitignored)
├── Dockerfile, docker-compose.yml, .dockerignore
└── samplefiles/       # env.sample, Caddyfile.sample, post-receive.sample
```

The binary reads `templates/`, `dist/`, `migrations/`, and `universe/` from cwd by default; override with `FINANCE_ROOT`. The SQLite db lives in `FINANCE_DATA_DIR` (default `./data`, production `/data`). Full config table in `README.md`.

**Key tables:** `symbols` (universe + denormalized latest price/snapshot + per-source `*_synced_at` staleness columns), `daily_prices` (deep OHLCV), `intraday_bars` (15m, pruned 14d), `quotes`, `fundamentals`, `filings`, `dividends`, `fund_profiles` + `fund_holdings`, `fund_metadata`, `leadership`, `watchlist` (sid → tickers), `endpoint_guard`, `data_status`, `fetch_log`. `kind` values: `stock`, `etf`, `index`, `future`, `crypto`.

## Key Routes

- `/`: home dashboard — a session-aware market-overview strip (one interactive chart per instrument: S&P, Dow, Nasdaq-100, gold, crude, BTC, plus VIX/volume/SMA reads) over the user's editable watchlist
- `/s/{ticker}`: symbol page — candlestick chart with indicators, key stats; a stock also shows fundamentals and a leadership roster, an ETF a fund profile (holdings, AUM), both show SEC filings
- `/api/dashboard`, `/api/dashboard/refresh`: dashboard data JSON + manual refresh
- `/api/watchlist` (POST), `/api/watchlist/remove` (POST): edit the session watchlist
- `/api/symbols/{ticker}/history`: candle + indicator series JSON for the chart
- `/api/symbols/{ticker}/growth`, `/api/symbols/{ticker}/refresh`: growth series + on-demand refresh stream
- `/api/symbols` (POST): add a symbol not yet in the universe (validated against Yahoo)
- `/search`: browse and search the whole universe (filter by kind, match ticker and company name)
- `/stream`: Server-Sent Events — live quotes, market session, health nudges
- `/health`, `/api/health`: data-health page and its JSON feed
- `/favicon.ico`, `/robots.txt`, `/sitemap.xml`: static SEO routes
- `/static/*`: Vite assets (1y cache header)

## Data Sources

All free, no account, no API key. Every outbound call goes through the `EndpointGuard` (see Architecture); data is fetched on demand for viewed symbols when stale or absent.

- **Historical daily OHLCV + intraday bars + live quotes — Yahoo Finance.** `v8/finance/chart`; no key, just a browser User-Agent. `interval=1d&range=max` returns a symbol's entire daily history in one call (the per-symbol backfill); `interval=15m&range=1d` serves live quotes + intraday. ETF/fundamentals metadata comes from `v10/finance/quoteSummary` (crumb-gated — see Gotchas). There is no separate history source.
- **Fundamentals, filings, leadership + ETF profiles — SEC EDGAR.** `company_tickers.json` / `companyfacts` / `submissions` for stock fundamentals and filings; Form 3/4/5 ownership XML for the officer/board roster; `company_tickers_mf.json` plus quarterly N-PORT filings for ETF fund profiles (holdings, AUM, asset mix). No key; a contact email (`SEC_CONTACT_EMAIL`) rides in the User-Agent. Indexes do not file with the SEC.

## Gotchas (don't relearn these)

- **Yahoo `range=max` silently downsamples `interval=1d`.** For symbols with long histories (futures, ^RUT/^VIX, multi-year ETFs) the v8 chart endpoint returns weekly/monthly bars even when a daily interval is asked, and for some a single bar. A *bounded* window (`range=10y`, or explicit `period1`/`period2`) is served at true daily granularity. The provider detects a downsampled `range=max` response (median gap > 4 days) and refetches 10y; detect coarseness from *recent* spacing, not whole-span density (^SPX has daily data for decades but a sparse pre-1900 tail).
- **The seed reconciles the universe on every boot, not only first-run.** `sync_universe` (upsert + prune) runs each boot so a `starter.csv` edit takes effect on deploy. Pruning keys on `is_seeded = 1` so user-added symbols are safe. This is a local, no-network reconcile — it does not backfill history (that happens on demand).
- **Yahoo `quoteSummary` is crumb-gated.** Must prime `fc.yahoo.com` + fetch `/v1/test/getcrumb` with cookies, cache the crumb, rotate on 401/403. Already handled in `src/providers/yahoo.rs`.
- **SEC's `fy` field tags the *filing's* fiscal year, not the period's.** Derive fiscal year from period-end + the company's fiscal-year-end month. Keep only clean full-year/discrete-quarter durations; skip quarterly balance-sheet figures.
- **"No data" / historyless symbols are a clean empty, not a failure** — never feed the breaker for a symbol Yahoo legitimately has no history for.
- **Guard state is shared across server + `seed` subcommand** via SQLite, and survives restarts. A boot-time breaker trip is normal after a deploy; it recovers via the half-open probe.
- **Supertrend is drawn as a single line series coloured per data point** (one value/one colour per bar), so the two trend colours can never render at the same x; the band jumps sides at a flip. **Do not** use two whitespace-gapped series — lightweight-charts connects the line straight across the gaps and draws both colours at once (the "constantly green" bug).
- **Yahoo NAV is only as fresh as you keep it.** Comparing a live price to a weeks-old NAV yields a meaningless premium/discount. The ETF quality read's tracking factor gates on `nav_synced_at` freshness and drops to `—` rather than assert a bogus premium. NAV is fetched on demand when an ETF page is viewed and stale.
- **The index overview slots are a hybrid by design.** The chart *line* is the E-mini future (so pre/regular/after-hours all move), while the headline value + % are the **cash index** quote (`regularMarketPrice` vs `chartPreviousClose`) so the number matches everyone (e.g. S&P 500 +0.41% = `^GSPC`). On big-basis days the line (futures) can diverge from the headline (cash) after 4pm — that's expected, not a bug. Each chart normally frames exactly one Schwab day (regular + extended hours, 7:00 AM–8:00 PM ET) of the most recent session.
- **The dashboard switches to a full-week frame after Friday's close.** From Friday 4:00 PM ET through the weekend until Monday 7:00 AM ET (`week_window` in `routes/home.rs`), every overview + watchlist chart spans the whole trading week (Mon 7 AM → Fri 8 PM ET) and the headline % is the full-week move (prior Friday's daily close → Friday's close), not the one-day move. Because the routine intraday poll only stores one day of 15-minute bars at a time, a one-off guarded `range=5d` backfill (`scheduler::backfill_intraday_week`, fired detached from `/api/dashboard/refresh`) fills any week-days the dashboard was not open for; it skips symbols already covering the week, so it is a no-op once filled.
- **`cargo` isn't on PATH in this dev container** — use `~/.cargo/bin/cargo`, and run the dev binary with `FINANCE_ROOT=/home/dev/code/finance` (or from the project dir so paths resolve from cwd).

## Tooling

- **Rust deps:** `cargo` (`Cargo.toml`, `Cargo.lock`)
- **JS deps:** `bun`, run from `frontend/` (`frontend/package.json`, `frontend/bun.lock`)
- **Production:** Docker (`rust:alpine` builder + `alpine:3.23` runtime). Deployed via `git push server master` triggering a post-receive hook that runs `docker compose up --build --detach`. Data persists to `/srv/data/finance/`.
