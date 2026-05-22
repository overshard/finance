# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

`finance` is a self-hosted, real-timeish market watcher for stocks, ETFs, indexes, and futures: live charts, key stats, fundamentals, and SEC filings. Single axum binary with sqlx + SQLite and a Vite frontend.

It is for *watching* the market only. No portfolio, no holdings, no money or cost-basis tracking, no accounts, no auth. Single operator. Deploys at `finance.bythewood.me`.

`PLAN.md` is the living design/resume doc — the phase roadmap, the decisions log, and the data-source policy live there. Read it for the *why* behind anything below.

## Commands

- **Dev server:** `make run` (Vite watch + `cargo run` concurrently on port 8000)
- **Production build:** `make build` (Vite assets + release binary at `target/release/finance`)
- **Run release binary:** `make start`
- **Seed the universe:** `make seed` (curated symbols + bulk daily history; idempotent and resumable). Same as `finance seed`
- **Docker build:** `docker build .`

In this dev container `cargo` is not on `PATH`; use `~/.cargo/bin/cargo`. Run the dev binary with `FINANCE_ROOT=/home/dev/code/finance` so it finds `templates/`, `dist/`, `migrations/`, and `universe/`.

There are no tests or linters configured.

## Architecture

**Backend:** Single-binary axum app. A tiny `src/main.rs` does env init, subcommand dispatch (`seed`), and server boot; `src/app.rs` builds `AppState` + `Config` + the `router()`. Per-feature route modules under `src/routes/`. Async sqlx + SQLite (WAL) for all reads and writes; the schema in `migrations/` is applied on boot via `sqlx::migrate!`.

**Providers (`src/providers/`):** One trait per concern — `HistoryProvider` (Stooq), `QuoteProvider` (Yahoo), `FundamentalsProvider` (SEC EDGAR) — with one struct per source, so a source is swappable without touching callers. `http.rs` builds the shared reqwest clients.

**Endpoint guard (`src/guard.rs`):** A persistent, per-endpoint `EndpointGuard` that every outbound data call passes through: a DB-backed reactive circuit breaker (trips on HTTP 429/503 at once or after a failure streak, exponential backoff, half-open probe recovery), a hard per-hour request budget, and request pacing. State lives in the `endpoint_guard` table, so it survives restarts and is shared by the server and the `seed` subcommand. The user considers never hitting a rate limit critical — see the Anti-spam policy in `PLAN.md`.

**Scheduler (`src/scheduler.rs`):** One long-lived tokio task on a 60s tick. Runs market-hours-aware background jobs — the first-run seed, the ~6-hourly incremental daily-history refresh, demand-driven intraday polling, a once-a-day close snapshot, and a prune — each writing `data_status` and `fetch_log` and pinging the stream hub so `/health` tracks it live.

**Real-time (`src/stream.rs`):** A `tokio::sync::broadcast` hub that also carries a per-ticker viewer-interest registry. The `/stream` axum SSE endpoint registers the tickers a page shows and forwards quote / market / health events; the browser uses `EventSource` and patches the DOM in place. The registry makes intraday polling demand-driven: only the symbols a browser is currently viewing get fetched.

**Market clock (`src/market.rs`):** The US equity session (Closed/Pre/Regular/Post) in `America/New_York` via `chrono-tz`. No exchange-holiday calendar (deliberate — see the decisions log).

**Compute (`src/compute.rs`):** Pure numeric code — indicator maths (`sma`, `ema`, `rsi`), graded fundamental ratios, range-meter marker positions, and the home-page sparkline SVG. The maths lives here, not in SQL or JS.

**Templates:** Jinja2 templates in `templates/` rendered by minijinja with a Jinja2-faithful HTML formatter so `/` is not escaped to `&#x2f;` (matches the sibling Rust apps). `vite_asset` resolves hashed asset names from `dist/.vite/manifest.json`.

**Frontend pipeline:** Vite (run from `frontend/`, built with bun) compiles `frontend/static_src/` into `dist/`, served at `/static/` with content-hashed filenames. Five entry points: `base` (shared shell + SSE client), `home`, `symbol`, `health`, `search`.

**Design — "Paper Ledger":** An old-school accounting ledger reimagined futuristic and modern: warm-paper background, ink-dark text, hairline rules, monospace ledger figures, restrained serif headings. Color is semantic and sparing — green/amber/red mean good/ok/bad (price moves, fundamental ratios, data-health states), never decoration. Chart indicator lines are a deliberate exception (a muted non-semantic palette). Tokens are CSS custom properties in `base.scss :root`. Built mobile-first; phone and desktop are both first-class.

**Request logging:** `src/middleware.rs` prints `time METHOD STATUS latency path` per request with ANSI-colored status codes, and serves the themed 404.

## Layout

```
finance/
├── Cargo.toml, Cargo.lock        # rust deps
├── Makefile, README.md, PLAN.md  # top-level (PLAN.md is the living design doc)
├── migrations/                   # sqlx migrations 0001-0005, applied on boot
├── universe/starter.csv          # curated seed list (~150 symbols)
├── src/
│   ├── main.rs        # entry: env init, `seed` subcommand, server boot
│   ├── app.rs         # AppState + Config + router()
│   ├── db.rs          # SqlitePool init, migrate, now_ms, meta helpers
│   ├── render.rs      # template render helper
│   ├── middleware.rs  # request log + themed 404
│   ├── templates.rs   # minijinja env, vite_asset, jinja2-compat formatter
│   ├── models.rs      # row structs + the shared ticker `Card`
│   ├── compute.rs     # indicator maths, graded ratios, sparkline SVG
│   ├── seed.rs        # first-run universe + history backfill
│   ├── scheduler.rs   # background job loop
│   ├── market.rs      # US market-session clock
│   ├── stream.rs      # SSE pub/sub hub + viewer-interest registry
│   ├── guard.rs       # persistent per-endpoint EndpointGuard
│   ├── providers/     # mod.rs (traits), http.rs, stooq.rs, yahoo.rs, sec.rs
│   └── routes/        # home, symbols, search, stream, health, seo
├── templates/         # base.html, includes/, pages/
├── frontend/static_src/   # base/ home/ symbol/ health/ search/ (Vite entries)
├── dist/              # vite build output (gitignored, served at /static/)
├── data/              # sqlite db at runtime (gitignored)
├── target/            # cargo build output (gitignored)
├── Dockerfile, docker-compose.yml, .dockerignore
└── samplefiles/       # env.sample, Caddyfile.sample, post-receive.sample
```

The binary reads `templates/`, `dist/`, `migrations/`, and `universe/` from cwd by default; override with `FINANCE_ROOT`. The SQLite db lives in `FINANCE_DATA_DIR` (default `./data`, production `/data`). Full config table in `README.md`.

## Key Routes

- `/`: home dashboard — index/commodity sparkline cards over the day's top movers
- `/s/{ticker}`: symbol page — candlestick chart with indicators, key stats; a stock also shows fundamentals, an ETF a fund profile (holdings, AUM), both show SEC filings
- `/api/symbols/{ticker}/history`: candle + indicator series JSON for the chart
- `/api/symbols` (POST): add a symbol not yet in the universe (validated against Yahoo)
- `/search`: browse and search the whole universe (filter by kind, match ticker and company name)
- `/stream`: Server-Sent Events — live quotes, market session, health nudges
- `/health`, `/api/health`: data-health page and its JSON feed
- `/favicon.ico`, `/robots.txt`, `/sitemap.xml`: static SEO routes
- `/static/*`: Vite assets (1y cache header)

## Data Sources

All free, no account. See `PLAN.md` for the full anti-spam / caching policy.

- **Historical daily OHLCV — Stooq.** One call returns a symbol's entire daily history. Gated behind a free apikey (`STOOQ_APIKEY`, in `.env`, gitignored).
- **Intraday bars + live quotes — Yahoo Finance.** `v8/finance/chart`; no key, just a browser User-Agent.
- **Fundamentals, filings + ETF profiles — SEC EDGAR.** `company_tickers.json` / `companyfacts` / `submissions` for stock fundamentals and filings; `company_tickers_mf.json` plus quarterly N-PORT filings for ETF fund profiles (holdings, AUM, asset mix). No key; a contact email (`SEC_CONTACT_EMAIL`) rides in the User-Agent. Indexes do not file with the SEC.

## Tooling

- **Rust deps:** `cargo` (`Cargo.toml`, `Cargo.lock`)
- **JS deps:** `bun`, run from `frontend/` (`frontend/package.json`, `frontend/bun.lock`)
- **Production:** Docker (`rust:alpine` builder + `alpine:3.23` runtime). Deployed via `git push server master` triggering a post-receive hook that runs `docker compose up --build --detach`. Data persists to `/srv/data/finance/`.
