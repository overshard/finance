// On-demand data refresh for the symbol page (Phase B).
//
// The server fetches nothing on a timer any more: a symbol's data is pulled
// here, when its page is open. On load this auto-runs a refresh (the live price
// always; slow SEC / metadata only when their stored copy is stale); the
// Refresh button re-pulls everything. Progress streams over SSE from
// `/api/symbols/{ticker}/refresh` and drives the bar.
//
// When a "deep" (server-rendered) section was refreshed, the server's `done`
// event asks us to reload so the new data and its "synced … ago" age render;
// when only the live price moved, the stream client has already patched it in
// place, so no reload is needed. A one-shot sessionStorage flag set just before
// our own reload keeps that reload from re-triggering the auto-refresh.

export function initRefresh() {
  const root = document.querySelector("[data-refresh-root]");
  if (!root) return;

  const ticker = root.dataset.ticker;
  const btn = root.querySelector("[data-refresh]");
  const bar = root.querySelector("[data-refresh-bar]");
  const fill = root.querySelector("[data-refresh-fill]");
  const status = root.querySelector("[data-refresh-status]");
  const skipKey = "fin:skipnext:" + ticker;

  let running = false;

  const setStatus = (t) => {
    if (status) status.textContent = t;
  };
  const setFill = (pct) => {
    if (fill) fill.style.width = Math.max(0, Math.min(100, pct)) + "%";
  };

  function run(force) {
    if (running) return;
    running = true;
    root.classList.add("is-running");
    if (bar) bar.hidden = false;
    setFill(2);
    if (btn) btn.disabled = true;
    setStatus("Updating…");

    const url = `/api/symbols/${encodeURIComponent(ticker)}/refresh?force=${force ? 1 : 0}`;
    const es = new EventSource(url);
    let done = false;

    es.addEventListener("step", (e) => {
      let d;
      try {
        d = JSON.parse(e.data);
      } catch {
        return;
      }
      // A step counts as half-done while running, full when it reports back, so
      // the bar advances smoothly across the step list.
      const progressed = d.i - (d.state === "running" ? 0.5 : 0);
      setFill((progressed / Math.max(1, d.n)) * 100);
      setStatus("Updating: " + d.label);
    });

    es.addEventListener("done", (e) => {
      done = true;
      es.close();
      running = false;
      setFill(100);
      let reload = false;
      try {
        reload = !!JSON.parse(e.data).reload;
      } catch {}
      if (reload) {
        // Show the new server-rendered data + ages. Mark this so the load it
        // triggers does not auto-refresh again.
        sessionStorage.setItem(skipKey, "1");
        location.reload();
        return;
      }
      root.classList.remove("is-running");
      if (btn) btn.disabled = false;
      setStatus("Up to date");
      window.setTimeout(() => {
        if (bar) bar.hidden = true;
      }, 700);
    });

    es.onerror = () => {
      if (done) return; // normal end-of-stream after `done`
      es.close();
      running = false;
      root.classList.remove("is-running");
      if (btn) btn.disabled = false;
      if (bar) bar.hidden = true;
      setStatus("Couldn’t refresh — showing stored data");
    };
  }

  if (btn) btn.addEventListener("click", () => run(true));

  // Auto-refresh on load, unless this very load was triggered by our own reload
  // after a refresh (the one-shot skip flag).
  if (sessionStorage.getItem(skipKey)) {
    sessionStorage.removeItem(skipKey);
    setStatus("Updated just now");
  } else {
    run(false);
  }
}
