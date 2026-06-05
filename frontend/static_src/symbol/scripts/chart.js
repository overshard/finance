import {
  createChart,
  CandlestickSeries,
  LineSeries,
  HistogramSeries,
  ColorType,
  createSeriesMarkers,
} from "lightweight-charts";

// Paper Ledger theme: ink figures and hairline rules on the warm paper
// surface. autoSize wires an internal ResizeObserver so the chart tracks
// its container on every viewport.
//
// handleScroll / handleScale are OFF: the range buttons are the only way to
// change what's shown, so the viewport can never pan or zoom into empty space
// beyond the loaded data. Drag is freed up for the measure tool below.
// Pane resizing is off for the same reason — the RSI pane keeps a fixed size.
const chartOptions = {
  autoSize: true,
  handleScroll: false,
  handleScale: false,
  layout: {
    background: { type: ColorType.Solid, color: "transparent" },
    textColor: "#6b6456",
    fontFamily: "'JetBrains Mono', monospace",
    attributionLogo: false,
    panes: { enableResize: false, separatorColor: "rgba(33,31,26,0.16)" },
  },
  grid: {
    vertLines: { color: "rgba(33,31,26,0.07)" },
    horzLines: { color: "rgba(33,31,26,0.07)" },
  },
  rightPriceScale: { borderColor: "rgba(33,31,26,0.16)" },
  timeScale: { borderColor: "rgba(33,31,26,0.16)", rightOffset: 0 },
  crosshair: { mode: 1 },
};

// Overlay indicator inks. The candlesticks own semantic green/red, and the
// rest of the app reserves green/amber/red for good/ok/bad, so the moving
// averages get their own muted, non-semantic palette — wayfinding lines, not
// value judgments — desaturated to sit inside the warm Paper Ledger world.
const OVERLAY_INK = {
  sma50: "#3f6f9c",
  sma200: "#9c6b3f",
  ema21: "#6f5b86",
};
const RSI_INK = "#3f6f9c";
// Phase 28 benchmark overlay: a fourth wayfinding ink, dashed so it reads as
// "what would have happened in the benchmark" rather than blending with the
// SMAs. Anchored to the fund's first visible close, so the two lines start
// together and the divergence is the relative performance the eye should
// follow.
const BENCH_INK = "#7a5237";
// Supertrend is the deliberate green/red exception (a user call): the band's
// whole point is its trend colour, and up=green / down=red matches the app's
// price-move semantics. It reuses the candle green/red exactly so the overlay
// reads as part of the price, not a separate wayfinding ink.
const SUPERTREND_UP = "#2f7d4f";
const SUPERTREND_DOWN = "#b23b32";
const VOLUME_UP = "rgba(47,125,79,0.38)";
const VOLUME_DOWN = "rgba(178,59,50,0.38)";
// Phase 25: earnings-date markers. A small ink dot above each candle that
// matches a past 8-K item-2.02 date. Same warm-paper ink-faint as the rest
// of the Paper Ledger palette so it reads as wayfinding, not a value verdict
// (the candles still own green/red and the indicator inks own the other
// non-semantic palette).
const EARNINGS_INK = "rgba(33,31,26,0.55)";

/** `12.4` -> `+$12.40`, `-3` -> `-$3.00`. */
function fmtMoney(n) {
  const sign = n > 0 ? "+" : n < 0 ? "-" : "";
  return `${sign}$${Math.abs(n).toFixed(2)}`;
}

// A bar's `time` is a `YYYY-MM-DD` string on the daily ranges and a UNIX-
// seconds number on the intraday ranges (1D / 1W). `barMs` returns epoch-ms
// for either, and `fmtBarTime` a human label — a plain date for daily bars, a
// New-York date+time for intraday ones (so the measure readout reads sensibly
// in both worlds).
function barMs(t) {
  return typeof t === "number" ? t * 1000 : Date.parse(t);
}
function fmtBarTime(t) {
  if (typeof t !== "number") return t;
  return new Date(t * 1000).toLocaleString("en-US", {
    timeZone: "America/New_York",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

/**
 * A human caption for a visible span, e.g. "over 6 months", "over 8 years".
 * Derived from the actual bars shown rather than the range button, so a deep
 * MAX history clamped to what fits is described honestly. Handles both the
 * daily date strings and the intraday UNIX-seconds times, scaling its unit
 * from hours (an intraday session) up to years.
 */
function spanLabel(from, to) {
  const ms = barMs(to) - barMs(from);
  const hours = ms / 3.6e6;
  if (hours < 20) {
    const h = Math.max(1, Math.round(hours));
    return `over ${h} hour${h === 1 ? "" : "s"}`;
  }
  const days = ms / 8.64e7;
  if (days < 11) {
    const d = Math.round(days);
    return `over ${d} day${d === 1 ? "" : "s"}`;
  }
  const months = ms / 2.6298e9; // 30.44d
  if (months < 1.6) return "over 1 month";
  if (months < 11.5) return `over ${Math.round(months)} months`;
  const years = months / 12;
  return years < 1.5 ? "over 1 year" : `over ${Math.round(years)} years`;
}

export function initChart() {
  const el = document.getElementById("chart");
  if (!el) return;
  const ticker = el.dataset.ticker;

  const chart = createChart(el, chartOptions);

  // Volume first so the candlesticks draw over it in the shared bottom strip;
  // its own price scale, pinned low and unlabelled, keeps it off the axis.
  const volumeSeries = chart.addSeries(HistogramSeries, {
    priceScaleId: "volume",
    priceFormat: { type: "volume" },
    lastValueVisible: false,
    priceLineVisible: false,
  });
  chart.priceScale("volume").applyOptions({
    scaleMargins: { top: 0.82, bottom: 0 },
  });

  const series = chart.addSeries(CandlestickSeries, {
    upColor: "#2f7d4f",
    downColor: "#b23b32",
    wickUpColor: "#2f7d4f",
    wickDownColor: "#b23b32",
    borderVisible: false,
  });

  // Moving-average overlays on the price pane. Created up front and shown or
  // hidden by the toggle row; sma200 first so the faster lines sit on top.
  const overlay = (key, dashed) =>
    chart.addSeries(LineSeries, {
      color: OVERLAY_INK[key],
      lineWidth: 2,
      lineStyle: dashed ? 2 : 0,
      priceLineVisible: false,
      lastValueVisible: false,
      crosshairMarkerVisible: false,
    });
  const overlays = {
    sma200: overlay("sma200", false),
    sma50: overlay("sma50", false),
    ema21: overlay("ema21", true),
  };

  // Benchmark line (Phase 28). Created up front and shown only when the
  // history payload carries `benchmark`; the toggle row gets a swatch so
  // the user can hide it the same way as the SMAs.
  const benchmarkSeries = chart.addSeries(LineSeries, {
    color: BENCH_INK,
    lineWidth: 2,
    lineStyle: 2,
    priceLineVisible: false,
    lastValueVisible: false,
    crosshairMarkerVisible: false,
    visible: false,
  });

  // Supertrend overlay: a single line whose colour is set per point — green
  // while the band trails below price (uptrend), red while it rides above
  // (downtrend). One line means one value and one colour per bar, so the two
  // trends can never draw at the same time; the band simply jumps to the other
  // side at a flip. (Two whitespace-gapped series were tried first but the line
  // connected straight across the gaps, drawing both colours at once.)
  const supertrendSeries = chart.addSeries(LineSeries, {
    color: SUPERTREND_UP,
    lineWidth: 2,
    priceLineVisible: false,
    lastValueVisible: false,
    crosshairMarkerVisible: false,
  });

  // Earnings-date markers (Phase 25). Stocks only; the payload carries an
  // `earnings` array of `YYYY-MM-DD` past dates that match candle times.
  // Each draws a small ink dot above the matching bar. v5's
  // createSeriesMarkers attaches to the candle series and is replaced
  // wholesale on each setMarkers call.
  const earningsMarkers = createSeriesMarkers(series, []);

  let bars = []; // loaded candles, ascending by time
  let latest = null; // last loaded payload, kept so RSI can attach on demand

  // Prior-close reference line (Phase 6). The intraday ranges draw the previous
  // daily close as a dashed guide so the session's move is legible at a glance;
  // the daily ranges carry no `prev_close`, so the line is cleared on those.
  let prevCloseLine = null;
  function setPrevCloseLine(price) {
    if (prevCloseLine) {
      series.removePriceLine(prevCloseLine);
      prevCloseLine = null;
    }
    if (price == null) return;
    prevCloseLine = series.createPriceLine({
      price,
      color: "rgba(33,31,26,0.42)",
      lineWidth: 1,
      lineStyle: 2,
      axisLabelVisible: true,
      title: "prev close",
    });
  }

  // The moving-average, RSI and benchmark overlays are all derived from the
  // daily series, so they are meaningless on the intraday ranges. Their toggle
  // buttons hide there (benchmark hides on its own when the payload has none).
  const DAILY_ONLY_INDS = ["sma50", "sma200", "ema21", "rsi", "supertrend"];

  // RSI lives in its own pane below the price pane and is created only while
  // toggled on, so an empty second pane never lingers when it is off.
  let rsiSeries = null;
  function buildRsi() {
    if (rsiSeries) return;
    rsiSeries = chart.addSeries(
      LineSeries,
      {
        color: RSI_INK,
        lineWidth: 2,
        priceLineVisible: false,
        lastValueVisible: false,
        crosshairMarkerVisible: false,
        // Pin the pane to 0..100 so the 30/70 guides are always in view.
        autoscaleInfoProvider: () => ({
          priceRange: { minValue: 0, maxValue: 100 },
        }),
      },
      1,
    );
    for (const level of [70, 30]) {
      rsiSeries.createPriceLine({
        price: level,
        color: "rgba(33,31,26,0.30)",
        lineWidth: 1,
        lineStyle: 2,
        axisLabelVisible: true,
        title: String(level),
      });
    }
    // Tight margins so the pinned 0..100 range fills the pane without the
    // axis padding out to stray values like 120.
    rsiSeries.priceScale().applyOptions({
      scaleMargins: { top: 0.12, bottom: 0.12 },
    });
    const panes = chart.panes();
    if (panes.length > 1) panes[1].setHeight(116);
    if (latest) rsiSeries.setData(latest.rsi14);
  }
  function destroyRsi() {
    if (!rsiSeries) return;
    chart.removeSeries(rsiSeries);
    rsiSeries = null;
    // removeSeries leaves the now-empty pane behind; drop it explicitly.
    if (chart.panes().length > 1) chart.removePane(1);
  }

  // Measure-tool overlay: a shaded band plus a readout chip, drawn by
  // click-dragging across the chart to compare two points (the Google
  // Finance gesture). Both are pointer-transparent so the drag stays on #chart.
  const band = document.createElement("div");
  band.className = "chart-band";
  band.hidden = true;
  const readout = document.createElement("div");
  readout.className = "chart-readout";
  readout.hidden = true;
  el.append(band, readout);

  const ts = chart.timeScale();
  let anchorIdx = null; // bar index where the drag began
  let curIdx = null; // bar index under the pointer now
  let dragging = false;

  // Map a viewport x to the nearest loaded bar index.
  function barIndexAt(clientX) {
    const x = clientX - el.getBoundingClientRect().left;
    const logical = ts.coordinateToLogical(x);
    if (logical === null) return null;
    return Math.min(bars.length - 1, Math.max(0, Math.round(logical)));
  }

  function clearSelection() {
    anchorIdx = null;
    curIdx = null;
    band.hidden = true;
    readout.hidden = true;
  }

  // Position the band between the two selected bars and fill the readout
  // with the % / absolute change over the interval. Called on drag and on
  // every chart relayout so the band stays glued to the data.
  function renderSelection() {
    if (anchorIdx === null || curIdx === null || anchorIdx === curIdx) {
      band.hidden = true;
      readout.hidden = true;
      return;
    }
    const a = Math.min(anchorIdx, curIdx);
    const b = Math.max(anchorIdx, curIdx);
    const xa = ts.logicalToCoordinate(a);
    const xb = ts.logicalToCoordinate(b);
    if (xa === null || xb === null) return;

    const left = Math.min(xa, xb);
    const width = Math.abs(xb - xa);
    band.style.left = `${left}px`;
    band.style.width = `${width}px`;
    band.hidden = false;

    const startClose = bars[a].close;
    const endClose = bars[b].close;
    const absChange = endClose - startClose;
    const pct = startClose !== 0 ? (absChange / startClose) * 100 : 0;
    const up = absChange >= 0;
    readout.dataset.dir = up ? "up" : "down";
    readout.innerHTML =
      `<span class="chart-readout__pct">${up ? "▲" : "▼"} ` +
      `${up ? "+" : ""}${pct.toFixed(2)}%</span>` +
      `<span class="chart-readout__sub">${fmtMoney(absChange)} · ` +
      `${fmtBarTime(bars[a].time)} → ${fmtBarTime(bars[b].time)}</span>`;
    readout.hidden = false;

    // Center the readout over the band, clamped to the chart's width.
    const mid = left + width / 2;
    const rw = readout.offsetWidth;
    const max = el.clientWidth - rw - 4;
    readout.style.left = `${Math.min(max, Math.max(4, mid - rw / 2))}px`;
  }

  el.addEventListener("pointerdown", (e) => {
    if (bars.length < 2) return;
    const idx = barIndexAt(e.clientX);
    if (idx === null) return;
    dragging = true;
    anchorIdx = idx;
    curIdx = idx;
    el.setPointerCapture(e.pointerId);
    renderSelection();
  });
  el.addEventListener("pointermove", (e) => {
    if (!dragging) return;
    const idx = barIndexAt(e.clientX);
    if (idx === null) return;
    curIdx = idx;
    renderSelection();
  });
  function endDrag(e) {
    if (!dragging) return;
    dragging = false;
    try {
      el.releasePointerCapture(e.pointerId);
    } catch {
      /* pointer already released */
    }
    // A click with no drag clears any existing selection.
    if (anchorIdx === curIdx) clearSelection();
  }
  el.addEventListener("pointerup", endDrag);
  el.addEventListener("pointercancel", endDrag);

  // The change chip beside the range buttons: % and absolute move across the
  // bars actually visible on the chart, so the headline figure always agrees
  // with what is drawn — a deep MAX history is clamped to what legibly fits,
  // and the chip then reports only that visible span, not the full dataset.
  function renderRangeSummary() {
    const summary = document.getElementById("range-summary");
    if (!summary) return;
    let lo = 0;
    let hi = bars.length - 1;
    const lr = ts.getVisibleLogicalRange();
    if (lr) {
      lo = Math.max(lo, Math.ceil(lr.from));
      hi = Math.min(hi, Math.floor(lr.to));
    }
    if (hi <= lo) {
      summary.hidden = true;
      return;
    }
    const start = bars[lo].close;
    const end = bars[hi].close;
    const abs = end - start;
    const pct = start !== 0 ? (abs / start) * 100 : 0;
    const up = abs >= 0;
    summary.dataset.dir = up ? "up" : "down";
    summary.innerHTML =
      `<span class="range-summary__chg num">${up ? "▲" : "▼"} ` +
      `${up ? "+" : ""}${pct.toFixed(2)}%` +
      `<span class="range-summary__abs">${fmtMoney(abs)}</span></span>` +
      `<span class="range-summary__cap">${spanLabel(bars[lo].time, bars[hi].time)}</span>`;
    summary.hidden = false;
  }

  // Keep the band and the range chip glued to the data when the chart relays
  // out — a range change (fitContent), a resize, anything that moves the view.
  ts.subscribeVisibleLogicalRangeChange(() => {
    if (anchorIdx !== null && curIdx !== null) renderSelection();
    renderRangeSummary();
  });

  // Apply a freshly fetched payload to every series at once.
  function applyData(d) {
    latest = d;
    bars = d.candles;
    series.setData(d.candles);
    volumeSeries.setData(
      d.candles.map((c) => ({
        time: c.time,
        value: c.volume,
        color: c.close >= c.open ? VOLUME_UP : VOLUME_DOWN,
      })),
    );
    overlays.sma50.setData(d.sma50);
    overlays.sma200.setData(d.sma200);
    overlays.ema21.setData(d.ema21);
    if (rsiSeries) rsiSeries.setData(d.rsi14);

    // Supertrend: one line, coloured per bar by its trend side.
    supertrendSeries.setData(
      (d.supertrend || []).map((p) => ({
        time: p.time,
        value: p.value,
        color: p.up ? SUPERTREND_UP : SUPERTREND_DOWN,
      })),
    );
    // Phase 28: benchmark overlay rides on the price pane when present.
    const bench = d.benchmark || [];
    benchmarkSeries.setData(bench);
    // Only show the benchmark series — and the toggle for it — when the
    // payload actually has one; its visibility then follows the toggle's
    // is-active state (off by default).
    const benchBtn = document.querySelector('[data-ind="benchmark"]');
    if (benchBtn) benchBtn.hidden = bench.length === 0;
    const benchOn = bench.length > 0 && (!benchBtn || benchBtn.classList.contains("is-active"));
    benchmarkSeries.applyOptions({ visible: benchOn });

    // Phase 25: earnings-date pips. Filter to dates inside the visible
    // candle window — lightweight-charts ignores markers whose time
    // does not match a candle, but trimming first keeps the payload small
    // and the markers sorted ascending (the API requires it).
    const earnings = d.earnings || [];
    const candleTimes = new Set(d.candles.map((c) => c.time));
    const markers = earnings
      .filter((e) => candleTimes.has(e.time))
      .map((e) => ({
        time: e.time,
        position: "aboveBar",
        color: EARNINGS_INK,
        shape: "circle",
      }))
      .sort((a, b) => (a.time < b.time ? -1 : a.time > b.time ? 1 : 0));
    earningsMarkers.setMarkers(markers);

    // Phase 6: intraday ranges (1D / 1W) draw the prior close and drop the
    // daily-only overlays + their toggles. Returning to a daily range restores
    // them (RSI rebuilds its pane only if its toggle is still on).
    setPrevCloseLine(d.intraday ? (d.prev_close ?? null) : null);
    DAILY_ONLY_INDS.forEach((key) => {
      const btn = document.querySelector(`[data-ind="${key}"]`);
      if (btn) btn.hidden = !!d.intraday;
    });
    if (d.intraday) {
      destroyRsi();
    } else if (document.querySelector('[data-ind="rsi"]')?.classList.contains("is-active")) {
      buildRsi();
    }
  }

  // ── indicator toggles ──────────────────────────────────────────────────
  function applyIndicator(key, on) {
    if (key === "rsi") {
      if (on) buildRsi();
      else destroyRsi();
    } else if (key === "volume") {
      volumeSeries.applyOptions({ visible: on });
    } else if (key === "benchmark") {
      benchmarkSeries.applyOptions({ visible: on });
    } else if (key === "supertrend") {
      supertrendSeries.applyOptions({ visible: on });
    } else if (overlays[key]) {
      overlays[key].applyOptions({ visible: on });
    }
  }

  document.querySelectorAll(".ind-btn").forEach((btn) => {
    const key = btn.dataset.ind;
    // Paint the swatch from the JS palette so the inks live in one place.
    const dot = btn.querySelector(".ind-btn__dot");
    if (dot) {
      // Supertrend's swatch is split green/red to telegraph its two-tone line;
      // the rest take their single ink from the palette.
      dot.style.background =
        key === "supertrend"
          ? `linear-gradient(90deg, ${SUPERTREND_UP} 50%, ${SUPERTREND_DOWN} 50%)`
          : key === "rsi"
            ? RSI_INK
            : key === "benchmark"
              ? BENCH_INK
              : OVERLAY_INK[key];
    }
    // The template's is-active class is the initial visibility.
    applyIndicator(key, btn.classList.contains("is-active"));
    btn.addEventListener("click", () => {
      const on = !btn.classList.contains("is-active");
      btn.classList.toggle("is-active", on);
      btn.setAttribute("aria-pressed", on ? "true" : "false");
      applyIndicator(key, on);
    });
  });

  // ── range buttons ──────────────────────────────────────────────────────
  const isIntraday = (range) => range === "1D" || range === "1W";

  // Fetch a range and paint it. `quiet` is the 60s intraday refresh: it skips
  // the loading dim and keeps any measure selection, since it is just folding
  // in newly-stored bars rather than answering a click.
  async function reload(range, quiet) {
    if (!quiet) el.classList.add("is-loading");
    try {
      const res = await fetch(
        `/api/symbols/${encodeURIComponent(ticker)}/history?range=${range}`,
      );
      if (!res.ok) throw new Error(`history ${res.status}`);
      applyData(await res.json());
      chart.timeScale().fitContent();
      renderRangeSummary();
    } catch (err) {
      if (!quiet) loaded = null;
      console.error("chart load failed", err);
    } finally {
      if (!quiet) el.classList.remove("is-loading");
    }
  }

  // While an intraday range is shown, re-pull every 60s so freshly-stored 15m
  // bars appear without a click. The fetch only touches the local DB (no Yahoo
  // call), and the live quote stream keeps the trailing bar moving in between.
  let refreshTimer = null;
  function stopRefresh() {
    if (refreshTimer) {
      clearInterval(refreshTimer);
      refreshTimer = null;
    }
  }

  let loaded = null;
  async function load(range) {
    if (loaded === range) return;
    loaded = range;
    clearSelection();
    stopRefresh();
    await reload(range, false);
    if (isIntraday(range)) {
      refreshTimer = setInterval(() => reload(range, true), 60000);
    }
  }

  // Live-tick the trailing intraday bar from the shared quote stream (Phase 6):
  // stream.js re-broadcasts each quote as a `finance:quote` event, so the chart
  // moves the last bar's close/high/low in place without a second EventSource.
  window.addEventListener("finance:quote", (e) => {
    const q = e.detail;
    if (!q || q.ticker !== ticker || q.price == null) return;
    if (!latest || !latest.intraday || !bars.length) return;
    const last = bars[bars.length - 1];
    last.close = q.price;
    if (q.price > last.high) last.high = q.price;
    if (q.price < last.low) last.low = q.price;
    series.update({
      time: last.time,
      open: last.open,
      high: last.high,
      low: last.low,
      close: last.close,
    });
    volumeSeries.update({
      time: last.time,
      value: last.volume,
      color: last.close >= last.open ? VOLUME_UP : VOLUME_DOWN,
    });
    renderRangeSummary();
    if (anchorIdx !== null && curIdx !== null) renderSelection();
  });

  window.addEventListener("pagehide", stopRefresh);

  const buttons = Array.from(document.querySelectorAll("[data-range]"));
  buttons.forEach((btn) => {
    btn.addEventListener("click", () => {
      buttons.forEach((b) => b.classList.remove("is-active"));
      btn.classList.add("is-active");
      load(btn.dataset.range);
    });
  });

  const active = buttons.find((b) => b.classList.contains("is-active"));
  load(active ? active.dataset.range : "1Y");
}
