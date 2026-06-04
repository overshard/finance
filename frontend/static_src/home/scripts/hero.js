// The dashboard's hero day graph + market reads (Phase C).
//
// Draws every watchlist symbol plus the S&P 500 on one chart, each as % change
// from today's open (the TradingView/Google "compare" shape), and fills the
// headline reads. Both come from /api/dashboard, re-fetched ~every minute (and
// on tab focus) so the chart and reads stay live without a reload — the
// watchlist cards below already live-tick via the base stream client.

import { createChart, LineSeries, ColorType } from "lightweight-charts";

// Non-semantic line palette for the watchlist (green/amber/red stay reserved
// for good/ok/bad reads elsewhere). Chosen to spread across the wheel so
// adjacent lines stay tellable apart; the S&P baseline is drawn in ink.
const PALETTE = [
  "#2f6fb0", // blue
  "#e07b29", // orange
  "#8a4fb3", // purple
  "#0f8b8d", // teal
  "#c2407a", // magenta
  "#8c6239", // brown
  "#3b4a9c", // indigo
  "#6b8e23", // olive
];
const INK = "#211f1a";
const DASH = "·";

// Display name for a ticker on the axis label and legend.
const displayName = (ticker) => (ticker === "^SPX" ? "S&P 500" : ticker);

const SESSION_LABELS = {
  regular: "Regular session",
  pre: "Pre-market",
  post: "After hours",
  closed: "Market closed",
};

const pctFmt = (v) => (v >= 0 ? "+" : "") + v.toFixed(2) + "%";

function fmtMoney(n) {
  if (n == null || Number.isNaN(n)) return DASH;
  return "$" + n.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
}
function fmtPct(n) {
  if (n == null || Number.isNaN(n)) return DASH;
  return n.toLocaleString("en-US", {
    minimumFractionDigits: 2, maximumFractionDigits: 2, signDisplay: "exceptZero",
  }) + "%";
}
// Compact volume: 1.2M / 853K, matching the server-side `compact` filter shape.
function fmtCompact(n) {
  if (n == null || Number.isNaN(n)) return DASH;
  const abs = Math.abs(n);
  if (abs >= 1e9) return (n / 1e9).toFixed(1).replace(/\.0$/, "") + "B";
  if (abs >= 1e6) return (n / 1e6).toFixed(1).replace(/\.0$/, "") + "M";
  if (abs >= 1e3) return (n / 1e3).toFixed(1).replace(/\.0$/, "") + "K";
  return String(n);
}
const cap = (s) => (s ? s.charAt(0).toUpperCase() + s.slice(1) : s);
// A fixed timestamp as an ET clock time, matching the server `asof` filter.
function fmtClock(ms) {
  if (!ms) return null;
  return new Date(ms)
    .toLocaleTimeString("en-US", {
      timeZone: "America/New_York", hour: "numeric", minute: "2-digit",
    })
    .replace(/\s/g, "")
    .toLowerCase();
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

export function initHero() {
  const mount = document.querySelector('[data-role="hero-chart"]');
  if (!mount) return;

  const chart = createChart(mount, {
    autoSize: true,
    handleScroll: false,
    handleScale: false,
    layout: {
      background: { type: ColorType.Solid, color: "transparent" },
      textColor: "#6b6456",
      fontFamily: "'JetBrains Mono', monospace",
      attributionLogo: false,
    },
    grid: {
      vertLines: { color: "rgba(33,31,26,0.06)" },
      horzLines: { color: "rgba(33,31,26,0.07)" },
    },
    rightPriceScale: { borderColor: "rgba(33,31,26,0.16)" },
    timeScale: {
      borderColor: "rgba(33,31,26,0.16)",
      timeVisible: true,
      secondsVisible: false,
    },
    crosshair: { mode: 1 },
    localization: { priceFormatter: pctFmt },
  });

  const seriesByTicker = new Map(); // ticker -> { series }

  function drawSeries(list) {
    const empty = document.querySelector('[data-role="hero-empty"]');
    if (!list || !list.length) {
      if (empty) empty.hidden = false;
      return;
    }
    if (empty) empty.hidden = true;

    const seen = new Set();
    const legend = [];
    let ci = 0;

    for (const s of list) {
      seen.add(s.ticker);
      const color = s.baseline ? INK : PALETTE[ci++ % PALETTE.length];
      let entry = seriesByTicker.get(s.ticker);
      if (!entry) {
        const series = chart.addSeries(LineSeries, {
          color,
          lineWidth: s.baseline ? 2 : 1.75,
          // The title labels the line at its last value on the price axis, so
          // each line is identifiable without decoding the colour.
          title: displayName(s.ticker),
          priceLineVisible: false,
          lastValueVisible: true,
          crosshairMarkerRadius: 3,
        });
        entry = { series };
        seriesByTicker.set(s.ticker, entry);
      } else {
        entry.series.applyOptions({ color, title: displayName(s.ticker) });
      }
      entry.series.setData(s.points.map((p) => ({ time: p.t, value: p.v })));
      const last = s.points.length ? s.points[s.points.length - 1].v : null;
      legend.push({ ticker: s.ticker, color, baseline: s.baseline, last });
    }

    // Drop series whose ticker is no longer in the payload.
    for (const [t, entry] of seriesByTicker) {
      if (!seen.has(t)) {
        chart.removeSeries(entry.series);
        seriesByTicker.delete(t);
      }
    }

    chart.timeScale().fitContent();
    renderLegend(legend);
  }

  function renderLegend(items) {
    const box = document.querySelector('[data-role="hero-legend"]');
    if (!box) return;
    box.innerHTML = "";
    for (const it of items) {
      const el = document.createElement("span");
      el.className = "legend-item" + (it.baseline ? " legend-item--baseline" : "");
      const sw = document.createElement("span");
      sw.className = "legend-item__swatch";
      sw.style.background = it.color;
      const name = document.createElement("span");
      name.textContent = it.ticker === "^SPX" ? "S&P 500" : it.ticker;
      const pct = document.createElement("span");
      pct.className = "legend-item__pct";
      pct.textContent = it.last == null ? "" : pctFmt(it.last);
      el.append(sw, name, pct);
      box.appendChild(el);
    }
  }

  function patchReads(r) {
    if (!r) return;
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
    const clock = fmtClock(r.asof);
    if (clock) setText("reads-asof", "Prices as of " + clock);
  }

  function patchSession(session) {
    const banner = document.querySelector('[data-role="session-banner"]');
    if (banner && session) banner.dataset.session = session;
    setText("session-label", SESSION_LABELS[session] || "Market closed");
    setText(
      "session-note",
      session === "closed"
        ? "Prices update during market hours " + DASH + " ET"
        : "US equities " + DASH + " all times ET",
    );
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
    drawSeries(data.series);
    patchReads(data.reads);
    patchSession(data.session);
  }

  // Draw immediately from stored data, then pull fresh quotes once on open (the
  // dashboard otherwise only updates via the market-hours intraday poll, so it
  // can look stale on open — especially after the close) and redraw. The 60s
  // loop keeps the chart/reads live; the watchlist cards live-tick over the
  // stream from the quotes the open refresh publishes.
  refresh();
  fetch("/api/dashboard/refresh")
    .catch(() => {})
    .finally(() => refresh());
  const timer = setInterval(refresh, 60000);
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) refresh();
  });
  window.addEventListener("pagehide", () => clearInterval(timer));
}
