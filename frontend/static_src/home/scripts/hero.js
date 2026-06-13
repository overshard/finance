// The dashboard's market-overview + watchlist charts (Phase F).
//
// Both the fixed market overview and the personal watchlist are drawn as the
// same per-instrument chart card: one Schwab trading day (7 AM–8 PM ET), an
// area line coloured green/red by the day's direction, pre-market / after-hours
// shaded, and a headline value + % change vs the previous close (the
// universally-quoted number). Everything comes from /api/dashboard, re-fetched
// ~every minute and on tab focus. Overview cards are built here; watchlist cards
// are server-rendered shells (for the link + remove button) that we draw into.

import { createChart, BaselineSeries, ColorType } from "lightweight-charts";

// Semantic day-direction colours (the Paper Ledger up/down inks) + soft fills.
// The day chart is drawn as a BASELINE series anchored at the previous close, so
// the line is green where price sits above yesterday's close and red where it
// sits below — the Google-Finance / Robinhood read, far more honest than tinting
// the whole line by the net day move.
const UP = "#2f7d4f";
const DOWN = "#b23b32";
const UP_FILL_NEAR = "rgba(47, 125, 79, 0.16)";
const UP_FILL_FAR = "rgba(47, 125, 79, 0)";
const DOWN_FILL_NEAR = "rgba(178, 59, 50, 0.16)";
const DOWN_FILL_FAR = "rgba(178, 59, 50, 0)";
const REF = "rgba(33, 31, 26, 0.28)"; // dashed previous-close line
const DASH = "·";

// Arrow + sign + colour together: a colourblind-safe change indicator (WCAG 1.4.1
// wants a second channel beyond colour). "▲ +1.96%" reads at a glance in greyscale.
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
function fmtSigned(n, unit) {
  if (n == null || Number.isNaN(n)) return DASH;
  const s = Math.abs(n).toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  const sign = n >= 0 ? "+" : "-";
  return unit === "$" ? `${sign}$${s}` : `${sign}${s}`;
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

// Sector-tile background: a green/red wash whose strength scales with the move,
// clamped at ±3% (the de-facto heatmap scale Yahoo/Finviz use), so a +0.3% tile
// is a faint tint and a +3%+ tile is saturated. Neutral wash when unknown.
function sectorColor(pct) {
  if (pct == null || Number.isNaN(pct)) return "var(--ink-wash, rgba(33, 31, 26, 0.05))";
  const t = Math.max(-1, Math.min(1, pct / 3));
  const a = (0.1 + 0.62 * Math.abs(t)).toFixed(3);
  return t >= 0 ? `rgba(47, 125, 79, ${a})` : `rgba(178, 59, 50, ${a})`;
}

// 12-hour AM/PM ET clock for the axis ticks and crosshair (never 24-hour).
function fmtAxisTime(tSec) {
  return new Date(tSec * 1000).toLocaleTimeString("en-US", {
    timeZone: "America/New_York",
    hour: "numeric",
    minute: "2-digit",
    hour12: true,
  });
}
function fmtWeekday(tSec) {
  return new Date(tSec * 1000).toLocaleDateString("en-US", {
    timeZone: "America/New_York",
    weekday: "short",
  });
}
// Axis tick formatter for the end-of-week full-week frame: label day-boundary
// ticks (DayOfMonth and coarser) with the weekday, intraday ticks with the time,
// so a Mon→Fri frame reads "Mon … 12 PM … Tue …" instead of repeating times.
function fmtWeekTick(tSec, tickMarkType) {
  return tickMarkType <= 2 ? fmtWeekday(tSec) : fmtAxisTime(tSec);
}
function fmtCrosshairTime(tSec) {
  return new Date(tSec * 1000).toLocaleString("en-US", {
    timeZone: "America/New_York",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
    hour12: true,
  });
}
function fmtClock(ms) {
  if (!ms) return null;
  return new Date(ms)
    .toLocaleTimeString("en-US", { timeZone: "America/New_York", hour: "numeric", minute: "2-digit" })
    .replace(/\s/g, "")
    .toLowerCase();
}

// ── freshness ────────────────────────────────────────────────────────────────
// Per-card data age, as a short chip + a tone. The whole point of this is that a
// quote is never *silently* stale: the card always says how old its number is, so
// landing on an out-of-date dashboard reads as "2m ago", not a mystery.
function fmtAge(ms) {
  if (!ms) return { text: "—", tone: "stale" };
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (s < 45) return { text: "live", tone: "live" };
  if (s < 90) return { text: "1m ago", tone: "fresh" };
  if (s < 3600) return { text: `${Math.round(s / 60)}m ago`, tone: s < 600 ? "fresh" : "stale" };
  if (s < 86400) return { text: `${Math.round(s / 3600)}h ago`, tone: "stale" };
  return { text: "stale", tone: "stale" };
}
function fmtAgo(ms) {
  if (!ms) return "";
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (s < 5) return "just now";
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.round(s / 60)}m ago`;
  return `${Math.round(s / 3600)}h ago`;
}

// Repaint one card's freshness chip from the asof epoch-ms stashed on it.
function paintFresh(el) {
  const ms = parseInt(el.dataset.asof || "", 10);
  const { text, tone } = fmtAge(Number.isNaN(ms) ? 0 : ms);
  el.textContent = text;
  el.dataset.tone = tone;
}

// Last shown value per ticker, so a card flashes when its number actually moves.
const lastShown = new Map();
// The freshest reads quote time (epoch-ms), so the header "updated Ns ago" ticks.
let readsAsofMs = null;

// ── session countdown ────────────────────────────────────────────────────────
// "Market closes in 2h 14m" in the banner: the next boundary on the fixed ET
// schedule (no holiday calendar, by design — mirrors market.rs): weekdays
// Pre 4:00 → Regular 9:30 → Post 16:00 → Closed 20:00; weekends closed.
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
  // Past today's last boundary, or a weekend: count to the next weekday's
  // pre-market open (Friday evening → Monday).
  const days = wd === 5 ? 3 : wd === 6 ? 2 : 1;
  const span = (days - 1) * 1440 + (1440 - minutes) + PRE_OPEN;
  return `Pre-market opens in ${fmtSpan(span)}`;
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

// ── extended-hours shading ───────────────────────────────────────────────────
// US regular session in ET minutes-of-day (9:30 AM – 4:00 PM). Everything else
// is "extended" (pre-market / after-hours / overnight) and gets shaded.
const REG_START = 9 * 60 + 30;
const REG_END = 16 * 60;
function etMinutes(tSec) {
  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone: "America/New_York",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  }).formatToParts(new Date(tSec * 1000));
  let h = 0;
  let m = 0;
  for (const p of parts) {
    if (p.type === "hour") h = parseInt(p.value, 10);
    else if (p.type === "minute") m = parseInt(p.value, 10);
  }
  if (h === 24) h = 0;
  return h * 60 + m;
}
const isExtended = (tSec) => {
  const x = etMinutes(tSec);
  return x < REG_START || x >= REG_END;
};

// Shade the extended-hours spans behind a chart's line (pointer-transparent
// overlay divs), recomputed on every relayout.
function renderBands(entry) {
  const box = entry.bandsEl;
  if (!box) return;
  box.innerHTML = "";
  const times = entry.times;
  if (!times || times.length < 2) return;
  const ts = entry.chart.timeScale();
  const w = entry.chartEl.clientWidth;
  const coords = times.map((p) => ts.timeToCoordinate(p.t));
  let sum = 0;
  let n = 0;
  for (let k = 1; k < coords.length; k++) {
    if (coords[k] != null && coords[k - 1] != null) {
      sum += coords[k] - coords[k - 1];
      n++;
    }
  }
  const half = (n ? sum / n : 6) / 2;
  let i = 0;
  while (i < times.length) {
    if (!times[i].ext || coords[i] == null) {
      i++;
      continue;
    }
    let j = i;
    while (j + 1 < times.length && times[j + 1].ext && coords[j + 1] != null) j++;
    const left = Math.max(0, coords[i] - half);
    const right = Math.min(w, coords[j] + half);
    if (right > left) {
      const band = document.createElement("div");
      band.className = "ov-band";
      band.style.left = `${left}px`;
      band.style.width = `${right - left}px`;
      box.appendChild(band);
    }
    i = j + 1;
  }
}

// ── chart card ───────────────────────────────────────────────────────────────
// Attach a lightweight-charts area chart to a card's `.ov-card__chart` mount.
function attachChart(chartEl) {
  const bandsEl = document.createElement("div");
  bandsEl.className = "ov-bands";
  chartEl.appendChild(bandsEl);

  const chart = createChart(chartEl, {
    autoSize: true,
    handleScroll: false,
    handleScale: false,
    layout: {
      background: { type: ColorType.Solid, color: "transparent" },
      textColor: "#8a8372",
      fontFamily: "'JetBrains Mono', monospace",
      fontSize: 10,
      attributionLogo: false,
    },
    grid: {
      vertLines: { visible: false },
      horzLines: { color: "rgba(33,31,26,0.05)" },
    },
    rightPriceScale: {
      borderVisible: false,
      scaleMargins: { top: 0.16, bottom: 0.08 },
    },
    timeScale: {
      borderColor: "rgba(33,31,26,0.12)",
      timeVisible: true,
      secondsVisible: false,
      tickMarkFormatter: (t) => fmtAxisTime(t),
      // We pin the visible range to the full Schwab-day grid ourselves (see
      // drawSeries); keep that pin across resizes instead of letting the chart
      // re-fit to wherever the real data happens to sit.
      lockVisibleTimeRangeOnResize: true,
    },
    crosshair: {
      mode: 1,
      vertLine: { labelVisible: true, width: 1, color: "rgba(33,31,26,0.25)", style: 3 },
      horzLine: { labelVisible: true, color: "rgba(33,31,26,0.25)", style: 3 },
    },
    localization: { timeFormatter: (t) => fmtCrosshairTime(t) },
  });

  const series = chart.addSeries(BaselineSeries, {
    // baseValue (the previous close) is set per-card in drawSeries.
    baseValue: { type: "price", price: 0 },
    topLineColor: UP,
    topFillColor1: UP_FILL_NEAR,
    topFillColor2: UP_FILL_FAR,
    bottomLineColor: DOWN,
    bottomFillColor1: DOWN_FILL_FAR,
    bottomFillColor2: DOWN_FILL_NEAR,
    lineWidth: 2,
    priceLineVisible: false,
    lastValueVisible: false,
    crosshairMarkerRadius: 3,
    crosshairMarkerBorderWidth: 0,
    crosshairMarkerBackgroundColor: "#211f1a",
  });

  const entry = { chart, series, chartEl, bandsEl, refLine: null, times: [], points: [], unit: "pts" };
  attachMeasure(entry);
  // Keep the shading + measure band glued to the data on every relayout.
  chart.timeScale().subscribeVisibleLogicalRangeChange(() => {
    renderBands(entry);
    if (entry.renderMeasure) entry.renderMeasure();
  });
  return entry;
}

// Click-drag measure tool: a shaded band + a readout chip showing the % and
// value change between the two bars under the drag (the symbol chart's gesture,
// ported to the mini charts). Snaps to real bars; suppresses the navigating
// click on watchlist cards when a drag actually happened.
function attachMeasure(entry) {
  const el = entry.chartEl;
  const band = document.createElement("div");
  band.className = "ov-measure-band";
  band.hidden = true;
  const readout = document.createElement("div");
  readout.className = "ov-measure-readout";
  readout.hidden = true;
  el.append(band, readout);

  let dragging = false;
  let anchorX = null;
  let curX = null;
  let moved = false;
  const localX = (e) => e.clientX - el.getBoundingClientRect().left;

  // Real (valued) bars and their current x-coordinates; whitespace is ignored.
  function realCoords() {
    const ts = entry.chart.timeScale();
    const out = [];
    for (const p of entry.points) {
      const x = ts.timeToCoordinate(p.t);
      if (x != null) out.push({ p, x });
    }
    return out;
  }
  function nearest(coords, x) {
    let best = null;
    let bd = Infinity;
    for (const o of coords) {
      const d = Math.abs(o.x - x);
      if (d < bd) {
        bd = d;
        best = o;
      }
    }
    return best;
  }

  function render() {
    if (anchorX == null || curX == null) {
      band.hidden = true;
      readout.hidden = true;
      return;
    }
    const coords = realCoords();
    const a = nearest(coords, anchorX);
    const b = nearest(coords, curX);
    if (!a || !b || a.p.t === b.p.t) {
      band.hidden = true;
      readout.hidden = true;
      return;
    }
    const left = Math.min(a.x, b.x);
    const right = Math.max(a.x, b.x);
    band.style.left = `${left}px`;
    band.style.width = `${right - left}px`;
    band.hidden = false;

    const start = a.p.t < b.p.t ? a.p : b.p;
    const end = a.p.t < b.p.t ? b.p : a.p;
    const abs = end.v - start.v;
    const pct = start.v !== 0 ? (abs / start.v) * 100 : 0;
    const up = abs >= 0;
    readout.dataset.dir = up ? "up" : "down";
    readout.innerHTML =
      `<span class="ov-measure__pct">${up ? "▲" : "▼"} ${up ? "+" : ""}${pct.toFixed(2)}%</span>` +
      `<span class="ov-measure__sub">${fmtSigned(abs, entry.unit)}</span>`;
    readout.hidden = false;
    const mid = (left + right) / 2;
    const rw = readout.offsetWidth;
    const max = el.clientWidth - rw - 4;
    readout.style.left = `${Math.min(max, Math.max(4, mid - rw / 2))}px`;
  }
  entry.renderMeasure = render;

  el.addEventListener("pointerdown", (e) => {
    if (!entry.points || entry.points.length < 2) return;
    dragging = true;
    moved = false;
    anchorX = localX(e);
    curX = anchorX;
    try {
      el.setPointerCapture(e.pointerId);
    } catch {
      /* ignore */
    }
    render();
  });
  el.addEventListener("pointermove", (e) => {
    if (!dragging) return;
    curX = localX(e);
    if (Math.abs(curX - anchorX) > 3) moved = true;
    render();
  });
  function end(e) {
    if (!dragging) return;
    dragging = false;
    try {
      el.releasePointerCapture(e.pointerId);
    } catch {
      /* ignore */
    }
    // A click with no drag clears the selection.
    if (!moved) {
      anchorX = null;
      curX = null;
      render();
    }
  }
  el.addEventListener("pointerup", end);
  el.addEventListener("pointercancel", end);
  // Suppress the watchlist card's navigation when the pointerup ended a drag.
  el.addEventListener(
    "click",
    (e) => {
      if (moved) {
        e.preventDefault();
        e.stopPropagation();
        moved = false;
      }
    },
    true,
  );
}

// Draw/update a series into an attached chart entry.
function drawSeries(entry, s) {
  entry.points = s.points; // real bars, for the measure tool
  entry.unit = s.unit;
  // Anchor the green/red split at the previous close, so the line reads above /
  // below yesterday's close rather than one flat colour for the whole day.
  entry.series.applyOptions({ baseValue: { type: "price", price: s.base } });
  entry.chart.applyOptions({
    // A full-week frame labels the axis by weekday; a single day, by time.
    timeScale: { tickMarkFormatter: s.week ? fmtWeekTick : (t) => fmtAxisTime(t) },
    localization: { priceFormatter: (v) => fmtValue(v, s.unit), timeFormatter: (t) => fmtCrosshairTime(t) },
  });

  // Frame every card on ONE identical x-axis so the small multiples line up: a
  // fixed 15-minute grid spanning the whole Schwab day [start_t, end_t] (7 AM–8 PM
  // ET). Each real 15m bar drops onto its grid slot (Yahoo's 15m bars land exactly
  // on this grid); every empty slot stays whitespace — not just before the first
  // bar and after the last, but ALSO any interior gap where Yahoo skipped an
  // illiquid pre-/after-hours bar. Filling those interior gaps is the point:
  // otherwise lightweight-charts collapses consecutive bars into adjacent slots,
  // so a card missing a few extended-hours bars ends up with fewer slots and
  // fitContent() stretches it differently — the same clock time would sit at a
  // different x on each card. With the full grid every card has the same slot
  // count and the same time at the same horizontal position.
  const pts = s.points;
  const GRID_STEP = 900; // 15 minutes, the intraday bar interval — shared by all cards
  const slots = Math.max(0, Math.round((s.end_t - s.start_t) / GRID_STEP));
  const valueAt = new Map();
  for (const p of pts) {
    const idx = Math.round((p.t - s.start_t) / GRID_STEP);
    if (idx >= 0 && idx <= slots) valueAt.set(idx, p.v);
  }
  const data = [];
  for (let idx = 0; idx <= slots; idx++) {
    const t = s.start_t + idx * GRID_STEP;
    const v = valueAt.get(idx);
    data.push(v == null ? { time: t } : { time: t, value: v });
  }
  entry.series.setData(data);
  entry.times = data.map((d) => ({ t: d.time, ext: isExtended(d.time) }));

  if (entry.refLine) entry.series.removePriceLine(entry.refLine);
  entry.refLine = entry.series.createPriceLine({
    price: s.base,
    color: REF,
    lineWidth: 1,
    lineStyle: 2,
    axisLabelVisible: false,
  });

  // Pin the visible range to the WHOLE grid, not fitContent(): fitContent frames
  // to wherever the real values sit, so a sparse card (e.g. BTC with only a few
  // recent bars) zooms in differently than a full one — the exact drift we're
  // killing. With every card showing the identical logical range [0 .. last slot],
  // 7 AM (slot 0) sits flush at the left edge and 8 PM (last slot) at the right on
  // every card, regardless of how many real bars it has.
  entry.chart.timeScale().setVisibleLogicalRange({ from: 0, to: data.length - 1 });
  renderBands(entry);
}

// Update a card's header value + % pill, its session label, the week-to-date
// move, and the freshness chip — and flash the card when the value actually moves.
function setHead(root, s) {
  const prev = lastShown.get(s.ticker);

  const v = root.querySelector(".ov-card__value");
  if (v) v.textContent = fmtValue(s.last, s.unit);
  const c = root.querySelector(".ov-card__chg");
  if (c) {
    c.textContent = fmtPctArrow(s.change_pct);
    c.classList.remove("is-up", "is-down", "is-flat");
    c.classList.add(s.change_pct == null ? "is-flat" : s.change_pct >= 0 ? "is-up" : "is-down");
  }

  // Session label badge ("Futures" / "Pre-market" / "After hours" / …): present
  // only off-hours, so the regular-session number stays unlabelled.
  const lab = root.querySelector(".ov-card__label");
  if (lab) {
    lab.textContent = s.headline_label || "";
    lab.hidden = !s.headline_label;
  }

  // Week-to-date move beside the day move.
  const wk = root.querySelector(".ov-card__week");
  if (wk) {
    wk.hidden = s.week_pct == null;
    if (s.week_pct != null) {
      wk.textContent = "wk " + fmtPct(s.week_pct);
      wk.classList.remove("is-up", "is-down", "is-flat");
      wk.classList.add(s.week_pct >= 0 ? "is-up" : "is-down");
    }
  }

  // Freshness chip: stash the asof epoch-ms so the ticker can age it in place.
  const fr = root.querySelector(".ov-card__fresh");
  if (fr) {
    fr.dataset.asof = s.asof || "";
    paintFresh(fr);
  }

  // Flash the card when the displayed value genuinely changed between polls, so
  // a live move is felt, not just silently swapped in.
  if (prev != null && s.last != null && prev !== s.last) {
    root.classList.remove("ov-flash-up", "ov-flash-down");
    void root.offsetWidth; // reflow so the animation re-triggers
    root.classList.add(s.last >= prev ? "ov-flash-up" : "ov-flash-down");
  }
  if (s.last != null) lastShown.set(s.ticker, s.last);
}

const escapeHtml = (s) =>
  String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

export function initHero() {
  const overviewGrid = document.querySelector('[data-role="overview-grid"]');
  const overviewCards = new Map(); // ticker -> { root, entry }
  const watchCards = new Map(); // ticker -> { root, entry }

  function makeOverviewCard(s) {
    const root = document.createElement("div");
    root.className = "ov-card";
    root.dataset.ticker = s.ticker;
    root.innerHTML =
      `<div class="ov-card__head">` +
      `<div class="ov-card__id"><span class="ov-card__name">${escapeHtml(s.name)}</span></div>` +
      `<div class="ov-card__nums"><span class="ov-card__value num"></span>` +
      `<span class="ov-card__chg num"></span></div></div>` +
      `<div class="ov-card__meta">` +
      `<span class="ov-card__label" hidden></span>` +
      `<span class="ov-card__week num" hidden></span>` +
      `<span class="ov-card__fresh" data-asof=""></span>` +
      `</div>` +
      `<div class="ov-card__chart"></div>`;
    overviewGrid.appendChild(root);
    const entry = attachChart(root.querySelector(".ov-card__chart"));
    return { root, entry };
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
      let c = overviewCards.get(s.ticker);
      if (!c) {
        c = makeOverviewCard(s);
        overviewCards.set(s.ticker, c);
      }
      setHead(c.root, s);
      drawSeries(c.entry, s);
    }
    for (const [t, c] of overviewCards) {
      if (!seen.has(t)) {
        c.entry.chart.remove();
        c.root.remove();
        overviewCards.delete(t);
      }
    }
  }

  // Watchlist cards are server-rendered shells; draw the chart into each and
  // refresh its value/%. A card with no series (no intraday bars) keeps its
  // server-rendered figures and simply shows no line.
  function drawWatchlist(list) {
    const byTicker = new Map((list || []).map((s) => [s.ticker, s]));
    document.querySelectorAll(".watch-grid .ov-card").forEach((root) => {
      const s = byTicker.get(root.dataset.ticker);
      if (!s) return;
      let c = watchCards.get(root.dataset.ticker);
      if (!c) {
        c = { root, entry: attachChart(root.querySelector(".ov-card__chart")) };
        watchCards.set(root.dataset.ticker, c);
      }
      setHead(root, s);
      drawSeries(c.entry, s);
    });
  }

  // The sector heatmap: 11 tiles, each a link to the ETF, coloured by its move.
  // Cheap to rebuild wholesale (11 nodes), and there is no per-tile animation to
  // preserve, so a fresh innerHTML each poll keeps it simple.
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
  // Top gainers / losers / most active from /api/movers (server-cached 8 min).
  // Fetched independently of the dashboard poll, after first paint, so a cold
  // pull never blocks the page.
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
      `<span class="mv-row__sub num">${fmtValue(m.price, "$")} ${DASH} ${fmtCompact(m.volume)}</span>` +
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
    // Crash-response lead: S&P drawdown from its record close.
    setText("drawdown-pct", r.drawdown_pct != null ? fmtPct(r.drawdown_pct) : DASH);
    setTone("drawdown-pct", r.drawdown_tone || "steady", "read__tone--");
    if (r.drawdown_label) setText("drawdown-label", r.drawdown_label);
    // Credit stress (HYG day move).
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

  // The header "Prices as of 3:42pm · updated 12s ago" caption, ticked in place
  // so the "updated …" part counts up between polls instead of looking frozen.
  function paintAsof() {
    if (!readsAsofMs) return;
    const clock = fmtClock(readsAsofMs);
    if (clock) setText("reads-asof", `Prices as of ${clock} ${DASH} updated ${fmtAgo(readsAsofMs)}`);
  }

  // Age every card's freshness chip + the header caption, so a static dashboard
  // visibly counts up rather than sitting at "live" forever.
  function tickFreshness() {
    document.querySelectorAll(".ov-card__fresh").forEach(paintFresh);
    paintAsof();
  }

  // Toggle the "Refreshing…" indicator while an on-open / on-focus pull is in
  // flight, so the wait for fresh quotes is visible instead of a silent stall.
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

  // Land → show the stored figures at once, then kick a guarded refresh (which
  // only re-hits Yahoo for anything older than the scheduler's throttle) with a
  // visible "Refreshing…" state so the wait is never a mystery.
  patchCountdown();
  refresh();
  setRefreshing(true);
  fetch("/api/dashboard/refresh")
    .catch(() => {})
    .finally(() => {
      setRefreshing(false);
      refresh();
    });

  // Movers load after first paint (server-cached, so this is usually a cache hit)
  // and refresh on a slow cadence aligned with the 8-minute server cache.
  loadMovers();

  // The /api/dashboard poll is a local DB read (no Yahoo call), so a tighter 20s
  // cadence is free and keeps the cards close to the freshest stored quote.
  const timer = setInterval(refresh, 20000);
  // The countdown drifts a minute at a time; a 30s repaint keeps it honest.
  const clockTimer = setInterval(patchCountdown, 30000);
  // Age the freshness chips + header caption every few seconds.
  const freshTimer = setInterval(tickFreshness, 5000);
  // Movers change slowly and are server-cached; a 4-minute refresh is plenty.
  const moversTimer = setInterval(loadMovers, 240000);
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) {
      patchCountdown();
      tickFreshness();
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
    clearInterval(freshTimer);
    clearInterval(moversTimer);
  });
}
