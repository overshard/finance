// Markets dashboard — live hero verdict + breadth (Phase 7).
//
// The hero's plain-language verdict, its headline figures, and the breadth band
// are server-rendered at page load and then kept live: the scheduler recomputes
// a market `summary` as the indexes tick intraday, base/stream.js re-broadcasts
// it as a `finance:summary` window event, and this patches those nodes in place
// from it — no second EventSource. The sparkline cards already stream live via
// the base quote patcher; this closes the gap the verdict + breadth left (they
// used to be frozen page-load snapshots, see PLAN.md Phase 5's known limitation).
//
// Patch targets carry data-role hooks in home.html. A node that wasn't rendered
// at load (a figure that was null then) is simply skipped — the patcher refines
// what's on the page, it doesn't build missing rows.

const DASH = "·";

// Mirrors the server-side `pct` filter and base/stream.js's fmtPct.
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

function fmtInt(n) {
  return (n ?? 0).toLocaleString("en-US");
}

function setText(role, text) {
  const el = document.querySelector(`[data-role="${role}"]`);
  if (el) el.textContent = text;
}

function setWidth(role, pct) {
  const el = document.querySelector(`[data-role="${role}"]`);
  if (el && pct != null) el.style.width = `${pct}%`;
}

// Set the semantic move class (green/red/flat) from a percentage change.
function setMove(el, pct) {
  el.classList.remove("is-up", "is-down", "is-flat");
  if (pct == null || Number.isNaN(pct)) el.classList.add("is-flat");
  else el.classList.add(pct >= 0 ? "is-up" : "is-down");
}

function applySummary(s) {
  if (!s) return;

  // Hero verdict + supporting clause.
  setText("hero-verdict", s.verdict);
  setText("hero-detail", s.detail);

  // Headline figures.
  const broad = document.querySelector('[data-role="hero-broad"]');
  if (broad && s.broad_pct != null) {
    broad.textContent = fmtPct(s.broad_pct);
    setMove(broad, s.broad_pct);
  }
  const b = s.breadth || {};
  if (b.pct_green != null) setText("hero-green", `${b.pct_green}%`);
  if (s.vix_label) setText("hero-vix", s.vix_label);

  // Breadth band: the counts, the share-green label, and the proportion bar.
  setText("breadth-adv", fmtInt(b.advancers));
  setText("breadth-dec", fmtInt(b.decliners));
  setText("breadth-flat", fmtInt(b.unchanged));
  if (b.pct_green != null) setText("breadth-pct", `${b.pct_green}% green`);
  setWidth("breadth-up", b.up_w);
  setWidth("breadth-flat-seg", b.flat_w);
  setWidth("breadth-down", b.down_w);

  const bar = document.querySelector('[data-role="breadth-bar"]');
  if (bar && b.total != null) {
    bar.setAttribute(
      "aria-label",
      `${b.advancers} advancing, ${b.decliners} declining of ${b.total} stocks`,
    );
  }
}

export function initSummary() {
  // Only the dashboard carries the hero; elsewhere this is a no-op.
  if (!document.querySelector('[data-role="hero"]')) return;
  window.addEventListener("finance:summary", (e) => applySummary(e.detail));
}
