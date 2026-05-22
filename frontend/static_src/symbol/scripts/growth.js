// Growth-of-$10,000 chart (Phase 28). A small area chart on the ETF symbol
// page that scales the fund's daily closes so $10,000 invested at the
// series' first bar reads as that, then runs forward to today. When a
// benchmark is configured, a dashed line of the same $10,000 in the
// benchmark runs alongside so the relative path is read at a glance.
//
// Driven by GET /api/symbols/{ticker}/growth, which returns
// `{ fund: [{date, value}, ...], benchmark: [...], benchmark_ticker }`.

import {
  createChart,
  AreaSeries,
  LineSeries,
  ColorType,
} from "lightweight-charts";

// Paper Ledger palette: ink fill (very translucent), the same warm-ink line
// the symbol price chart uses, and the dashed benchmark in the chart's
// non-semantic wayfinding-ink palette.
const FUND_LINE = "#2f7d4f";
const FUND_FILL_TOP = "rgba(47, 125, 79, 0.20)";
const FUND_FILL_BTM = "rgba(47, 125, 79, 0.00)";
const BENCH_LINE = "#7a5237";

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
  rightPriceScale: {
    borderColor: "rgba(33,31,26,0.16)",
    // Format the y-axis in compact dollars (e.g. $12.4K, $58K) so the
    // growth path reads cleanly without a bare 50000 number.
    mode: 0,
  },
  timeScale: { borderColor: "rgba(33,31,26,0.16)", rightOffset: 0 },
  crosshair: { mode: 1 },
};

export function initGrowth() {
  const el = document.getElementById("growth-chart");
  if (!el) return;
  const ticker = el.dataset.ticker;

  fetch(`/api/symbols/${encodeURIComponent(ticker)}/growth`)
    .then((res) => {
      if (!res.ok) throw new Error(`growth ${res.status}`);
      return res.json();
    })
    .then((d) => {
      if (!d.fund || d.fund.length < 2) {
        el.innerHTML = '<p class="growth-empty">Not enough history to draw the growth path.</p>';
        return;
      }
      const chart = createChart(el, chartOptions);
      const fund = chart.addSeries(AreaSeries, {
        lineColor: FUND_LINE,
        topColor: FUND_FILL_TOP,
        bottomColor: FUND_FILL_BTM,
        lineWidth: 2,
        priceFormat: {
          type: "custom",
          // Compact dollar formatter for the y-axis and crosshair: $42.1K,
          // $1.2M etc., which keeps the labels short across the 4-orders-of-
          // magnitude growth a multi-decade fund accrues.
          formatter: fmtCompactUsd,
          minMove: 1,
        },
      });
      fund.setData(
        d.fund.map((p) => ({ time: p.date, value: p.value })),
      );
      if (d.benchmark && d.benchmark.length >= 2) {
        const bench = chart.addSeries(LineSeries, {
          color: BENCH_LINE,
          lineWidth: 2,
          lineStyle: 2,
          priceFormat: {
            type: "custom",
            formatter: fmtCompactUsd,
            minMove: 1,
          },
          priceLineVisible: false,
        });
        bench.setData(
          d.benchmark.map((p) => ({ time: p.date, value: p.value })),
        );
      }
      chart.timeScale().fitContent();
    })
    .catch((err) => {
      console.error("growth load failed", err);
    });
}

/** $1,234,567 -> "$1.2M", $42_100 -> "$42.1K". */
function fmtCompactUsd(n) {
  const abs = Math.abs(n);
  if (abs >= 1e9) return `$${(n / 1e9).toFixed(1)}B`;
  if (abs >= 1e6) return `$${(n / 1e6).toFixed(1)}M`;
  if (abs >= 1e3) return `$${(n / 1e3).toFixed(1)}K`;
  return `$${n.toFixed(0)}`;
}
