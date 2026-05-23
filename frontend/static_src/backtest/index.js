// Backtest page (Phase 30).
//
// Renders one horizon at a time inside #bt-panel: a stats panel, an equity
// curve drawn with lightweight-charts (strategy + ^SPX benchmark), and a
// table of every rebalance period's picks and returns. Switching tabs runs
// the same renderer against a fresh /api/backtest fetch, so the page never
// reloads.

import "./styles/backtest.scss";
import {
  createChart,
  AreaSeries,
  LineSeries,
  ColorType,
} from "lightweight-charts";

const STRAT_LINE = "#2f7d4f"; // ink-green; matches the home up palette
const STRAT_FILL_TOP = "rgba(47, 125, 79, 0.20)";
const STRAT_FILL_BTM = "rgba(47, 125, 79, 0.00)";
const BENCH_LINE = "#7a5237"; // wayfinding ink-brown (Phase 8 indicator palette)

const chartOptions = {
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
    vertLines: { color: "rgba(33,31,26,0.07)" },
    horzLines: { color: "rgba(33,31,26,0.07)" },
  },
  rightPriceScale: { borderColor: "rgba(33,31,26,0.16)" },
  timeScale: { borderColor: "rgba(33,31,26,0.16)", rightOffset: 0 },
  crosshair: { mode: 1 },
};

const panel = document.getElementById("bt-panel");
const tabs = document.querySelectorAll(".bt-tab");

let current = "month";

function setActive(horizon) {
  current = horizon;
  tabs.forEach((t) => {
    t.classList.toggle("is-active", t.dataset.horizon === horizon);
    t.setAttribute("aria-selected", t.dataset.horizon === horizon);
  });
}

// Phase 31: a small skeleton (4 stat bones + a chart bone + status text)
// shown while the JSON loads, instead of bare "Loading backtest…" text on
// empty paper. The first render keeps the server-rendered skeleton; later
// tab switches re-create it so each horizon change shows the skeleton too.
function skeletonHtml() {
  return `
    <div class="bt-skeleton" data-role="skeleton">
      <div class="bt-skeleton__stats">
        <div class="bt-skeleton__stat"></div>
        <div class="bt-skeleton__stat"></div>
        <div class="bt-skeleton__stat"></div>
        <div class="bt-skeleton__stat"></div>
      </div>
      <div class="bt-skeleton__chart"></div>
      <p class="bt-status">Loading backtest&hellip;</p>
    </div>`;
}

function load(horizon) {
  setActive(horizon);
  // Reuse the existing server-rendered skeleton on the first load (no
  // flicker / paint cost); regenerate it on each subsequent horizon
  // change so the loading affordance is consistent.
  if (!panel.querySelector("[data-role=skeleton]")) {
    panel.innerHTML = skeletonHtml();
  }
  fetch(`/api/backtest?horizon=${encodeURIComponent(horizon)}`)
    .then((res) => {
      if (!res.ok) throw new Error(`backtest ${res.status}`);
      return res.json();
    })
    .then(render)
    .catch((err) => {
      panel.innerHTML = `<p class="bt-status bt-status--err">Backtest failed: ${err.message}</p>`;
      console.error(err);
    });
}

function render(d) {
  if (!d.stats || d.equity.length < 2) {
    panel.innerHTML = `
      <p class="bt-status">
        Not enough history for a <strong>${escape(d.horizon.label)}</strong>
        backtest yet. ${escape(d.horizon.desc)}.
      </p>`;
    return;
  }
  const s = d.stats;
  // Header summary: which horizon, over what window, with how many
  // rebalances. The signal-description rides under as a quiet note so the
  // reader knows what they are looking at without scrolling.
  const head = `
    <header class="bt-head">
      <h2 class="bt-head__title">${escape(d.horizon.label)} horizon</h2>
      <p class="bt-head__desc">${escape(d.horizon.desc)}.</p>
      <p class="bt-head__window">
        ${escape(s.period_start)} &rarr; ${escape(s.period_end)} &middot;
        ${s.num_periods} rebalances &middot; ${s.num_picks} picks total
      </p>
    </header>`;

  // The four headline figures: strategy total return, benchmark total
  // return, per-pick win rate, per-period win rate. Each is a small card
  // with the figure and a quiet label.
  const stratGood = s.total_return_pct >= s.benchmark_return_pct;
  const stats = `
    <div class="bt-stats">
      <div class="bt-stat">
        <div class="bt-stat__label">Strategy total</div>
        <div class="bt-stat__val num ${stratGood ? "is-up" : "is-down"}">${pct(s.total_return_pct)}</div>
        <div class="bt-stat__sub">$${fmtUsd(d.starting_capital)} &rarr; <strong>$${fmtUsd(s.final_strategy)}</strong> &middot; ${pct(s.cagr_pct)}/yr</div>
      </div>
      <div class="bt-stat">
        <div class="bt-stat__label">${escape(d.bench_ticker)} benchmark</div>
        <div class="bt-stat__val num">${pct(s.benchmark_return_pct)}</div>
        <div class="bt-stat__sub">$${fmtUsd(d.starting_capital)} &rarr; <strong>$${fmtUsd(s.final_benchmark)}</strong> &middot; ${pct(s.benchmark_cagr_pct)}/yr</div>
      </div>
      <div class="bt-stat">
        <div class="bt-stat__label">Per-pick win rate</div>
        <div class="bt-stat__val num">${winPct(s.per_pick_win_rate)}</div>
        <div class="bt-stat__sub">share of individual picks that closed up</div>
      </div>
      <div class="bt-stat">
        <div class="bt-stat__label">Beat benchmark</div>
        <div class="bt-stat__val num">${winPct(s.per_period_win_rate)}</div>
        <div class="bt-stat__sub">share of periods the basket beat ${escape(d.bench_ticker)}</div>
      </div>
    </div>`;

  // History table: one row per rebalance, picks listed compactly. The
  // newest rebalance comes first so the most recent test is at the top.
  const rows = d.periods
    .slice()
    .reverse()
    .map((p) => {
      const picks = p.picks
        .map(
          (pick) =>
            `<a class="bt-pick ${pick.return_pct >= 0 ? "is-up" : "is-down"}"
                href="/s/${encodeURIComponent(pick.ticker)}"
                title="${escape(pick.ticker)}: ${pct(pick.return_pct)}">
               ${escape(pick.ticker)}<span class="bt-pick__ret">${pct(pick.return_pct)}</span>
             </a>`,
        )
        .join("");
      return `
        <tr class="${p.beat_benchmark ? "is-beat" : ""}">
          <td class="num">${escape(p.start_date)}</td>
          <td class="num">${escape(p.end_date)}</td>
          <td class="bt-row__picks">${picks}</td>
          <td class="num ${p.basket_return_pct >= 0 ? "is-up" : "is-down"}">${pct(p.basket_return_pct)}</td>
          <td class="num">${pct(p.benchmark_return_pct)}</td>
        </tr>`;
    })
    .join("");
  const history = `
    <h3 class="bt-section">Rebalance history</h3>
    <div class="bt-table-wrap">
      <table class="bt-table">
        <thead>
          <tr><th>Start</th><th>End</th><th>Picks</th>
              <th>Basket</th><th>${escape(d.bench_ticker)}</th></tr>
        </thead>
        <tbody>${rows}</tbody>
      </table>
    </div>`;

  panel.innerHTML = `
    ${head}
    ${stats}
    <h3 class="bt-section">Equity curve</h3>
    <div id="bt-chart" class="bt-chart"></div>
    ${history}`;

  // Draw the chart after the DOM is in place.
  const el = document.getElementById("bt-chart");
  const chart = createChart(el, chartOptions);
  const strat = chart.addSeries(AreaSeries, {
    lineColor: STRAT_LINE,
    topColor: STRAT_FILL_TOP,
    bottomColor: STRAT_FILL_BTM,
    lineWidth: 2,
    priceFormat: { type: "custom", formatter: fmtCompactUsd, minMove: 1 },
  });
  strat.setData(d.equity.map((p) => ({ time: p.date, value: p.strategy })));
  const bench = chart.addSeries(LineSeries, {
    color: BENCH_LINE,
    lineWidth: 2,
    lineStyle: 2,
    priceFormat: { type: "custom", formatter: fmtCompactUsd, minMove: 1 },
    priceLineVisible: false,
  });
  bench.setData(d.equity.map((p) => ({ time: p.date, value: p.benchmark })));
  chart.timeScale().fitContent();
}

// ── small formatters ──
function pct(n) {
  if (n == null || !Number.isFinite(n)) return "—";
  const sign = n > 0 ? "+" : "";
  return `${sign}${n.toFixed(2)}%`;
}
function winPct(n) {
  if (n == null || !Number.isFinite(n)) return "—";
  return `${n.toFixed(1)}%`;
}
function fmtUsd(n) {
  return n.toLocaleString("en-US", { maximumFractionDigits: 0 });
}
function fmtCompactUsd(n) {
  const abs = Math.abs(n);
  if (abs >= 1e9) return `$${(n / 1e9).toFixed(1)}B`;
  if (abs >= 1e6) return `$${(n / 1e6).toFixed(1)}M`;
  if (abs >= 1e3) return `$${(n / 1e3).toFixed(1)}K`;
  return `$${n.toFixed(0)}`;
}
// HTML-escape any user-visible string injected into innerHTML.
function escape(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;",
  })[c]);
}

tabs.forEach((t) => t.addEventListener("click", () => load(t.dataset.horizon)));
load(current);
