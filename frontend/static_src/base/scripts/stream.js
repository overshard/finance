// Live market stream client.
//
// Opens one EventSource to /stream, declaring the tickers the current page
// shows so the server only polls Yahoo for symbols actually on screen. `quote`
// events patch the data-field nodes in place; `market` events drive the status
// pill; a `health` nudge is re-broadcast as a `finance:health` window event
// for the data-health page. Pages are server-rendered with the freshest known
// figures, so the initial snapshot lands silently and only genuine moves flash.

// These mirror the server-side minijinja filters (templates.rs) so a value
// patched here is identical to one the server rendered.
const DASH = "·";

// Dashboard sparkline viewBox: 0 0 100 36, line drawn within y ∈ [3, 33].
// Mirrors the SPARK_* constants in compute.rs.
const SPARK_TOP = 3;
const SPARK_BOTTOM = 33;

function fmtMoney(n) {
  if (n == null || Number.isNaN(n)) return DASH;
  return "$" + n.toLocaleString("en-US", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
}

function fmtSigned(n) {
  if (n == null || Number.isNaN(n)) return DASH;
  return n.toLocaleString("en-US", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
    signDisplay: "exceptZero",
  });
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

// Set the semantic move class (green/red/flat) from a percentage change.
function setMove(el, pct) {
  el.classList.remove("is-up", "is-down", "is-flat");
  if (pct == null || Number.isNaN(pct)) el.classList.add("is-flat");
  else el.classList.add(pct >= 0 ? "is-up" : "is-down");
}

// Nudge a dashboard card's sparkline from a live quote: recolour the card and
// move the line's trailing point onto the new price, keeping the same value→y
// mapping the server used (compute::sparkline). data-lo/data-hi carry the
// y-scale; a price outside it is clamped to the box.
function paintSparkline(root, q) {
  root.classList.toggle("is-up-card", q.change_pct >= 0);
  root.classList.toggle("is-down-card", q.change_pct < 0);

  const svg = root.querySelector("svg.spark");
  if (!svg || q.price == null) return;
  const lo = parseFloat(svg.dataset.lo);
  const hi = parseFloat(svg.dataset.hi);
  if (!(hi > lo)) return;

  const t = Math.min(1, Math.max(0, (q.price - lo) / (hi - lo)));
  const y = (SPARK_BOTTOM - t * (SPARK_BOTTOM - SPARK_TOP)).toFixed(2);

  const line = svg.querySelector(".spark__line");
  if (line) {
    const pts = line.getAttribute("points").trim().split(/\s+/);
    const x = pts[pts.length - 1].split(",")[0];
    pts[pts.length - 1] = `${x},${y}`;
    line.setAttribute("points", pts.join(" "));
  }
  // The area fill's points are [x0,bottom  …line…  xN,bottom], so the line's
  // final point is the second-to-last token.
  const area = svg.querySelector(".spark__area");
  if (area) {
    const pts = area.getAttribute("points").trim().split(/\s+/);
    if (pts.length >= 3) {
      const x = pts[pts.length - 2].split(",")[0];
      pts[pts.length - 2] = `${x},${y}`;
      area.setAttribute("points", pts.join(" "));
    }
  }
}

// Last price seen per ticker, so a card flashes in the right direction.
const lastPrice = new Map();

function applyQuote(q) {
  const prev = lastPrice.get(q.ticker);
  lastPrice.set(q.ticker, q.price);
  const dir = prev === undefined ? 0 : Math.sign(q.price - prev);

  document.querySelectorAll(`[data-ticker="${q.ticker}"]`).forEach((root) => {
    const price = root.querySelector('[data-field="price"]');
    if (price) price.textContent = fmtMoney(q.price);

    // Compact form on the dashboard cards: the day's % move alone.
    const pct = root.querySelector('[data-field="change_pct"]');
    if (pct) {
      pct.textContent = fmtPct(q.change_pct);
      setMove(pct, q.change_pct);
    }
    // Full form in the symbol header: absolute and % together.
    const chg = root.querySelector('[data-field="change"]');
    if (chg && q.change_pct != null) {
      chg.textContent = `${fmtSigned(q.change_abs)} (${fmtPct(q.change_pct)})`;
      setMove(chg, q.change_pct);
    }

    // Dashboard sparkline cards track the live quote: recolour and move the
    // line tip. (The price/change nodes above are already patched.)
    if (root.classList.contains("spark-card") && q.change_pct != null) {
      paintSparkline(root, q);
    }

    const isCard =
      root.classList.contains("ticker-card") ||
      root.classList.contains("spark-card");
    if (dir !== 0 && isCard) {
      root.classList.remove("flash-up", "flash-down");
      void root.offsetWidth; // reflow, so the animation re-triggers
      root.classList.add(dir > 0 ? "flash-up" : "flash-down");
    }
  });
}

// Status-pill appearance per market session. The dot color reuses the
// existing data-state styles; the label is the human session name.
const SESSIONS = {
  regular: { state: "ok", label: "Market open" },
  pre: { state: "ok", label: "Pre-market" },
  post: { state: "ok", label: "After hours" },
  closed: { state: "idle", label: "Market closed" },
};

export function initStream() {
  const pill = document.querySelector('[data-role="status-pill"]');
  const label = document.querySelector('[data-role="status-label"]');

  let connected = false;
  let session = "closed";

  function paintPill() {
    if (!pill) return;
    if (!connected) {
      pill.dataset.state = "stale";
      if (label) label.textContent = "Reconnecting…";
      return;
    }
    const s = SESSIONS[session] || SESSIONS.closed;
    pill.dataset.state = s.state;
    if (label) label.textContent = s.label;
  }

  const tickers = [
    ...new Set(
      [...document.querySelectorAll("[data-ticker]")]
        .map((el) => el.dataset.ticker)
        .filter(Boolean),
    ),
  ];
  const query = tickers.length
    ? "?symbols=" + tickers.map(encodeURIComponent).join(",")
    : "";

  let es = null;

  function connect() {
    es = new EventSource("/stream" + query);

    es.addEventListener("open", () => {
      connected = true;
      paintPill();
    });
    es.addEventListener("error", () => {
      // EventSource reconnects on its own; just reflect the gap in the pill.
      connected = false;
      paintPill();
    });
    es.addEventListener("quote", (e) => {
      try {
        applyQuote(JSON.parse(e.data));
      } catch {
        /* ignore a malformed frame */
      }
    });
    es.addEventListener("market", (e) => {
      try {
        session = JSON.parse(e.data).session;
        paintPill();
      } catch {
        /* ignore a malformed frame */
      }
    });
    es.addEventListener("health", () => {
      // Re-broadcast as a window event so the data-health page can react
      // without opening its own EventSource. The frame carries no payload —
      // it is purely a nudge to re-pull /api/health.
      window.dispatchEvent(new Event("finance:health"));
    });
  }

  connect();

  // Close the stream cleanly before the page goes away. Without this the
  // browser aborts an in-flight chunked response on every navigation, which
  // Chrome logs as ERR_INCOMPLETE_CHUNKED_ENCODING. If the page is later
  // restored from the back/forward cache, reconnect so it stays live.
  window.addEventListener("pagehide", () => {
    if (es) es.close();
  });
  window.addEventListener("pageshow", (e) => {
    if (e.persisted && (!es || es.readyState === EventSource.CLOSED)) {
      connected = false;
      connect();
    }
  });
}
