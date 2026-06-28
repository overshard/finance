// The dashboard's market overview + watchlist (redesigned to a sparkline grid).
//
// Goal: a glanceable, Yahoo-Finance-style read of "how are the markets doing"
// at one glance. Each instrument (the major indexes + gold, crude, bitcoin) and
// each watchlist symbol is a small card: its name, its live value, the day's %
// change, and a tiny non-interactive SVG sparkline of the day's path coloured
// green/red vs the previous close. No interactive charts, no session badges, no
// per-card chrome — the calm overview Isaac actually trusts. Everything comes
// from /api/dashboard, re-fetched ~every 20s and on tab focus. The same card
// shape is used for the fixed overview (built here) and the watchlist
// (server-rendered shells we draw the sparkline into).

// Semantic day-direction inks (Paper Ledger up/down) + soft area fills. The
// sparkline is coloured by the day's direction vs the previous close, the
// Google-Finance / Yahoo read.
const UP = "#2f7d4f";
const DOWN = "#b23b32";
const UP_FILL = "rgba(47, 125, 79, 0.13)";
const DOWN_FILL = "rgba(178, 59, 50, 0.13)";
const REF = "rgba(33, 31, 26, 0.28)"; // dashed previous-close baseline
const DASH = "·";

// Arrow + sign + colour together: a colourblind-safe change indicator (WCAG
// 1.4.1 wants a second channel beyond colour). "▲ +1.96%" reads in greyscale.
function fmtPctArrow(n) {
  if (n == null || Number.isNaN(n)) return DASH;
  const a = n > 0 ? "▲ " : n < 0 ? "▼ " : "";
  return a + fmtPct(n);
}

const SESSION_LABELS = {
  regular: "Regular session",
  pre: "Pre-market",
  post: "After hours",
  closed: "Market closed",
};

// ── formatters ─────────────────────────────────────────────────────────────
function fmtValue(n, unit) {
  if (n == null || Number.isNaN(n)) return DASH;
  const s = n.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  return unit === "$" ? "$" + s : s;
}
function fmtPct(n) {
  if (n == null || Number.isNaN(n)) return DASH;
  return (
    n.toLocaleString("en-US", {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
      signDisplay: "exceptZero",
    }) + "%"
  );
}
function fmtCompact(n) {
  if (n == null || Number.isNaN(n)) return DASH;
  const abs = Math.abs(n);
  if (abs >= 1e9) return (n / 1e9).toFixed(1).replace(/\.0$/, "") + "B";
  if (abs >= 1e6) return (n / 1e6).toFixed(1).replace(/\.0$/, "") + "M";
  if (abs >= 1e3) return (n / 1e3).toFixed(1).replace(/\.0$/, "") + "K";
  return String(n);
}
const cap = (s) => (s ? s.charAt(0).toUpperCase() + s.slice(1) : s);

function fmtClock(ms) {
  if (!ms) return null;
  return new Date(ms)
    .toLocaleTimeString("en-US", { timeZone: "America/New_York", hour: "numeric", minute: "2-digit" })
    .replace(/\s/g, "")
    .toLowerCase();
}
function fmtAgo(ms) {
  if (!ms) return "";
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (s < 5) return "just now";
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.round(s / 60)}m ago`;
  return `${Math.round(s / 3600)}h ago`;
}

// Sector-tile background: a green/red wash whose strength scales with the move,
// clamped at ±3% (the de-facto heatmap scale Yahoo/Finviz use). Neutral when
// unknown.
function sectorColor(pct) {
  if (pct == null || Number.isNaN(pct)) return "var(--ink-wash, rgba(33, 31, 26, 0.05))";
  const t = Math.max(-1, Math.min(1, pct / 3));
  const a = (0.1 + 0.62 * Math.abs(t)).toFixed(3);
  return t >= 0 ? `rgba(47, 125, 79, ${a})` : `rgba(178, 59, 50, ${a})`;
}

// ── sparkline ────────────────────────────────────────────────────────────────
// Build a small, non-interactive SVG sparkline of one instrument's day: the
// intraday line over the day's grid, coloured by direction vs the previous
// close (`base`), with a faint dashed baseline at that close and a soft area
// fill. preserveAspectRatio="none" stretches the fixed viewBox to the card; the
// line keeps a crisp 1.5px stroke via vector-effect. Returns "" when there are
// too few points to draw (the card then shows just its value + %).
function sparkSvg(s) {
  const pts = s.points || [];
  if (pts.length < 2) return "";
  const W = 100;
  const H = 34;
  const PAD = 2;
  // Value range, widened to include the baseline so the dashed line always sits
  // inside the frame.
  let lo = Infinity;
  let hi = -Infinity;
  for (const p of pts) {
    if (p.v < lo) lo = p.v;
    if (p.v > hi) hi = p.v;
  }
  if (s.base != null) {
    lo = Math.min(lo, s.base);
    hi = Math.max(hi, s.base);
  }
  const span = hi - lo || 1;
  // x by the bar's position in the day window so a half-day plots from the left
  // rather than stretching across the full width.
  const dt = s.end_t > s.start_t ? s.end_t - s.start_t : 1;
  const x = (t) => (PAD + ((t - s.start_t) / dt) * (W - 2 * PAD)).toFixed(2);
  const y = (v) => (PAD + (1 - (v - lo) / span) * (H - 2 * PAD)).toFixed(2);
  const line = pts.map((p, i) => `${i ? "L" : "M"}${x(p.t)} ${y(p.v)}`).join(" ");
  const first = x(pts[0].t);
  const last = x(pts[pts.length - 1].t);
  const area = `${line} L${last} ${H - PAD} L${first} ${H - PAD} Z`;
  const color = s.up ? UP : DOWN;
  const fill = s.up ? UP_FILL : DOWN_FILL;
  const baseY = s.base != null ? y(s.base) : null;
  const baseline =
    baseY != null
      ? `<line x1="${PAD}" y1="${baseY}" x2="${W - PAD}" y2="${baseY}" stroke="${REF}" stroke-width="0.5" stroke-dasharray="2 2" vector-effect="non-scaling-stroke"/>`
      : "";
  return (
    `<svg class="spark" viewBox="0 0 ${W} ${H}" preserveAspectRatio="none" aria-hidden="true">` +
    `<path class="spark__area" d="${area}" fill="${fill}"/>` +
    baseline +
    `<path class="spark__line" d="${line}" fill="none" stroke="${color}" stroke-width="1.5" stroke-linejoin="round" stroke-linecap="round" vector-effect="non-scaling-stroke"/>` +
    `</svg>`
  );
}

function setText(role, text) {
  const el = document.querySelector(`[data-role="${role}"]`);
  if (el && text != null) el.textContent = text;
}
function setTone(role, tone, prefix) {
  const el = document.querySelector(`[data-role="${role}"]`);
  if (!el || !tone) return;
  [...el.classList].forEach((c) => {
    if (c.startsWith(prefix)) el.classList.remove(c);
  });
  el.classList.add(prefix + tone);
}

// ── session countdown ────────────────────────────────────────────────────────
// "Market closes in 2h 14m" in the banner: the next boundary on the fixed ET
// schedule (no holiday calendar, by design — mirrors market.rs).
const WEEKDAYS = { Sun: 0, Mon: 1, Tue: 2, Wed: 3, Thu: 4, Fri: 5, Sat: 6 };
const PRE_OPEN = 4 * 60;
const REG_OPEN = 9 * 60 + 30;
const REG_CLOSE = 16 * 60;
const POST_CLOSE = 20 * 60;

function etNowParts() {
  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone: "America/New_York",
    weekday: "short",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  }).formatToParts(new Date());
  let wd = 0;
  let h = 0;
  let m = 0;
  for (const p of parts) {
    if (p.type === "weekday") wd = WEEKDAYS[p.value] ?? 0;
    else if (p.type === "hour") h = parseInt(p.value, 10) % 24;
    else if (p.type === "minute") m = parseInt(p.value, 10);
  }
  return { wd, minutes: h * 60 + m };
}

function fmtSpan(mins) {
  const d = Math.floor(mins / 1440);
  const h = Math.floor((mins % 1440) / 60);
  const m = mins % 60;
  if (d > 0) return h > 0 ? `${d}d ${h}h` : `${d}d`;
  if (h > 0) return m > 0 ? `${h}h ${m}m` : `${h}h`;
  return `${Math.max(1, m)}m`;
}

function nextSessionRead() {
  const { wd, minutes } = etNowParts();
  if (wd >= 1 && wd <= 5) {
    const next = [
      [PRE_OPEN, "Pre-market opens"],
      [REG_OPEN, "Market opens"],
      [REG_CLOSE, "Market closes"],
      [POST_CLOSE, "After hours ends"],
    ].find(([at]) => minutes < at);
    if (next) return `${next[1]} in ${fmtSpan(next[0] - minutes)}`;
  }
  const days = wd === 5 ? 3 : wd === 6 ? 2 : 1;
  const span = (days - 1) * 1440 + (1440 - minutes) + PRE_OPEN;
  return `Pre-market opens in ${fmtSpan(span)}`;
}

// Last shown value per ticker, so a card flashes when its number actually moves.
const lastShown = new Map();
// The freshest reads quote time (epoch-ms), so the header "updated Ns ago" ticks.
let readsAsofMs = null;

const escapeHtml = (s) =>
  String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

// Update a card's value + % pill and its sparkline, flashing when the value moves.
function paintCard(root, s) {
  const prev = lastShown.get(s.ticker);

  const v = root.querySelector(".ov-card__value");
  if (v) v.textContent = fmtValue(s.last, s.unit);
  const c = root.querySelector(".ov-card__chg");
  if (c) {
    c.textContent = fmtPctArrow(s.change_pct);
    c.classList.remove("is-up", "is-down", "is-flat");
    c.classList.add(s.change_pct == null ? "is-flat" : s.change_pct >= 0 ? "is-up" : "is-down");
  }
  const chart = root.querySelector(".ov-card__chart");
  if (chart) chart.innerHTML = sparkSvg(s);

  if (prev != null && s.last != null && prev !== s.last) {
    root.classList.remove("ov-flash-up", "ov-flash-down");
    void root.offsetWidth; // reflow so the animation re-triggers
    root.classList.add(s.last >= prev ? "ov-flash-up" : "ov-flash-down");
  }
  if (s.last != null) lastShown.set(s.ticker, s.last);
}

export function initHero() {
  const overviewGrid = document.querySelector('[data-role="overview-grid"]');
  const overviewCards = new Map(); // ticker -> root

  function makeOverviewCard(s) {
    const root = document.createElement("div");
    root.className = "ov-card";
    root.dataset.ticker = s.ticker;
    root.innerHTML =
      `<div class="ov-card__name">${escapeHtml(s.name)}</div>` +
      `<div class="ov-card__nums">` +
      `<span class="ov-card__value num"></span>` +
      `<span class="ov-card__chg num"></span>` +
      `</div>` +
      `<div class="ov-card__chart"></div>`;
    overviewGrid.appendChild(root);
    return root;
  }

  function drawOverview(list) {
    const empty = document.querySelector('[data-role="hero-empty"]');
    if (!overviewGrid) return;
    if (!list || !list.length) {
      if (empty) empty.hidden = false;
      return;
    }
    if (empty) empty.hidden = true;
    const seen = new Set();
    for (const s of list) {
      seen.add(s.ticker);
      let root = overviewCards.get(s.ticker);
      if (!root) {
        root = makeOverviewCard(s);
        overviewCards.set(s.ticker, root);
      }
      paintCard(root, s);
    }
    for (const [t, root] of overviewCards) {
      if (!seen.has(t)) {
        root.remove();
        overviewCards.delete(t);
      }
    }
  }

  // Watchlist cards are server-rendered shells; draw the sparkline into each and
  // refresh its value/%. A card with no series (no intraday bars) keeps its
  // server-rendered figures and simply shows no line.
  function drawWatchlist(list) {
    const byTicker = new Map((list || []).map((s) => [s.ticker, s]));
    document.querySelectorAll(".watch-grid .ov-card").forEach((root) => {
      const s = byTicker.get(root.dataset.ticker);
      if (s) paintCard(root, s);
    });
  }

  // The sector heatmap: 11 tiles, each a link to the ETF, coloured by its move.
  function drawSectors(list) {
    const grid = document.querySelector('[data-role="sectors-grid"]');
    if (!grid || !list) return;
    grid.removeAttribute("aria-busy");
    grid.innerHTML = list
      .map((s) => {
        const pct = s.change_pct;
        const cls = pct == null ? "is-flat" : pct >= 0 ? "is-up" : "is-down";
        return (
          `<a class="sector-tile ${cls}" href="/s/${encodeURIComponent(s.ticker)}" ` +
          `style="background:${sectorColor(pct)}" title="${escapeHtml(s.ticker)}">` +
          `<span class="sector-tile__name">${escapeHtml(s.name)}</span>` +
          `<span class="sector-tile__pct num">${fmtPct(pct)}</span>` +
          `</a>`
        );
      })
      .join("");
  }

  // ── market movers ──────────────────────────────────────────────────────────
  function moverRow(m) {
    const pct = m.change_pct;
    const cls = pct == null ? "is-flat" : pct >= 0 ? "is-up" : "is-down";
    return (
      `<a class="mv-row" href="/s/${encodeURIComponent(m.symbol)}" title="${escapeHtml(m.name)}">` +
      `<span class="mv-row__id">` +
      `<span class="mv-row__sym">${escapeHtml(m.symbol)}</span>` +
      `<span class="mv-row__name">${escapeHtml(m.name)}</span></span>` +
      `<span class="mv-row__nums">` +
      `<span class="mv-row__pct num ${cls}">${fmtPctArrow(pct)}</span>` +
      `<span class="mv-row__sub num">${fmtValue(m.price, "$")}</span>` +
      `</span></a>`
    );
  }

  async function loadMovers() {
    let data;
    try {
      const res = await fetch("/api/movers", { headers: { Accept: "application/json" } });
      if (!res.ok) return;
      data = await res.json();
    } catch {
      return;
    }
    const root = document.querySelector('[data-role="movers"]');
    if (!root) return;
    root.removeAttribute("aria-busy");
    const fill = (key, rows) => {
      const list = root.querySelector(`[data-mv="${key}"] .movers__list`);
      if (list) {
        list.innerHTML = (rows || []).map(moverRow).join("") || `<p class="movers__empty">${DASH}</p>`;
      }
    };
    fill("gainers", data.gainers);
    fill("losers", data.losers);
    fill("actives", data.actives);
    const asof = document.querySelector('[data-role="movers-asof"]');
    if (asof) asof.textContent = data.asof ? "as of " + fmtClock(data.asof) : "";
  }

  function patchReads(r) {
    if (!r) return;
    setText("drawdown-pct", r.drawdown_pct != null ? fmtPct(r.drawdown_pct) : DASH);
    setTone("drawdown-pct", r.drawdown_tone || "steady", "read__tone--");
    if (r.drawdown_label) setText("drawdown-label", r.drawdown_label);
    setText("credit-pct", r.credit_pct != null ? fmtPct(r.credit_pct) : DASH);
    setTone("credit-pct", r.credit_tone || "steady", "read__tone--");
    if (r.credit_label) setText("credit-label", r.credit_label);
    setText("vix-level", r.vix_level != null ? r.vix_level.toFixed(2) : DASH);
    if (r.vix_tone) {
      setText("vix-tone", cap(r.vix_tone));
      setTone("vix-tone", r.vix_tone, "read__tone--");
    }
    setText("volume", fmtCompact(r.volume));
    setText("volume-label", r.volume_label ? r.volume_label + " vs avg" : DASH);
    if (r.sma_read) {
      setText("sma-read", r.sma_read);
      setTone("sma-read", r.sma_tone, "read__tone--");
    }
    readsAsofMs = r.asof || null;
    paintAsof();
  }

  // The header "Prices as of 3:42pm · updated 12s ago" caption, ticked in place.
  function paintAsof() {
    if (!readsAsofMs) return;
    const clock = fmtClock(readsAsofMs);
    if (clock) setText("reads-asof", `Prices as of ${clock} ${DASH} updated ${fmtAgo(readsAsofMs)}`);
  }

  function setRefreshing(on) {
    const el = document.querySelector('[data-role="refresh-state"]');
    if (el) el.hidden = !on;
  }

  function patchCountdown() {
    setText("session-note", nextSessionRead() + " " + DASH + " all times ET");
  }

  function patchSession(session) {
    const banner = document.querySelector('[data-role="session-banner"]');
    if (banner && session) banner.dataset.session = session;
    setText("session-label", SESSION_LABELS[session] || "Market closed");
    patchCountdown();
  }

  async function refresh() {
    let data;
    try {
      const res = await fetch("/api/dashboard", { headers: { Accept: "application/json" } });
      if (!res.ok) return;
      data = await res.json();
    } catch {
      return;
    }
    drawOverview(data.series);
    drawSectors(data.sectors);
    drawWatchlist(data.watchlist);
    patchReads(data.reads);
    patchSession(data.session);
  }

  // Land → show stored figures at once, then kick a guarded refresh with a
  // visible "Refreshing…" state so the wait for fresh quotes is never a mystery.
  patchCountdown();
  refresh();
  setRefreshing(true);
  fetch("/api/dashboard/refresh")
    .catch(() => {})
    .finally(() => {
      setRefreshing(false);
      refresh();
    });

  loadMovers();

  const timer = setInterval(refresh, 20000);
  const clockTimer = setInterval(patchCountdown, 30000);
  const asofTimer = setInterval(paintAsof, 5000);
  const moversTimer = setInterval(loadMovers, 240000);
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) {
      patchCountdown();
      paintAsof();
      loadMovers();
      setRefreshing(true);
      fetch("/api/dashboard/refresh")
        .catch(() => {})
        .finally(() => {
          setRefreshing(false);
          refresh();
        });
    }
  });
  window.addEventListener("pagehide", () => {
    clearInterval(timer);
    clearInterval(clockTimer);
    clearInterval(asofTimer);
    clearInterval(moversTimer);
  });
}
