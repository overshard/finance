// Industries page (Phase 15).
//
// On `/industries` the page is server-rendered HTML; nothing to do here.
// On a sector / industry detail page a `#ind-chart` element carries
// `data-sector` (and optionally `data-industry`); we fetch the equal-weight
// composite history + ^SPX benchmark and draw both with lightweight-charts.

import "./styles/industries.scss";
import { createChart, AreaSeries, LineSeries, ColorType } from "lightweight-charts";

const COMP_LINE = "#2f7d4f";
const COMP_FILL_TOP = "rgba(47, 125, 79, 0.20)";
const COMP_FILL_BTM = "rgba(47, 125, 79, 0.00)";
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
  rightPriceScale: { borderColor: "rgba(33,31,26,0.16)" },
  timeScale: { borderColor: "rgba(33,31,26,0.16)", rightOffset: 0 },
  crosshair: { mode: 1 },
};

const el = document.getElementById("ind-chart");
if (el) {
  const sector = el.dataset.sector;
  const industry = el.dataset.industry;
  // `/api/industries/{sector}/{industry?}/history` returns a composite + benchmark
  // series, each anchored at 100 on the first shared trading date.
  const url = industry
    ? `/api/industries/${encodeURIComponent(sector)}/${encodeURIComponent(industry)}/history`
    : `/api/industries/${encodeURIComponent(sector)}/history`;
  fetch(url)
    .then((res) => {
      if (!res.ok) throw new Error(`industries chart ${res.status}`);
      return res.json();
    })
    .then((d) => {
      if (!d.composite || d.composite.length < 2) {
        el.innerHTML = '<p class="ind-chart__empty">No composite history available yet.</p>';
        return;
      }
      const chart = createChart(el, chartOptions);
      const comp = chart.addSeries(AreaSeries, {
        lineColor: COMP_LINE,
        topColor: COMP_FILL_TOP,
        bottomColor: COMP_FILL_BTM,
        lineWidth: 2,
        priceFormat: { type: "custom", formatter: fmtIndex, minMove: 0.01 },
      });
      comp.setData(d.composite.map((p) => ({ time: p.d, value: p.v })));
      if (d.benchmark && d.benchmark.length > 1) {
        const bench = chart.addSeries(LineSeries, {
          color: BENCH_LINE,
          lineWidth: 2,
          lineStyle: 2,
          priceFormat: { type: "custom", formatter: fmtIndex, minMove: 0.01 },
          priceLineVisible: false,
        });
        bench.setData(d.benchmark.map((p) => ({ time: p.d, value: p.v })));
      }
      chart.timeScale().fitContent();
    })
    .catch((err) => {
      el.innerHTML = `<p class="ind-chart__empty">Chart failed: ${err.message}</p>`;
      console.error(err);
    });
}

function fmtIndex(n) {
  if (n == null || !Number.isFinite(n)) return "—";
  return n.toFixed(1);
}
