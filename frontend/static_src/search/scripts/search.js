// Search page enhancement: the "Add <TICKER>" button.
//
// The page is server-rendered; this only wires the add-symbol affordance.
// Clicking it POSTs the ticker to /api/symbols, which validates it against
// Yahoo and registers it. On success the browser lands on the new symbol's
// page; on failure the server's message is shown inline.

export function initSearch() {
  const btn = document.querySelector("[data-add-ticker]");
  if (!btn) return;

  const errEl = document.querySelector('[data-role="add-error"]');
  const ticker = btn.dataset.addTicker;
  const label = btn.textContent;

  function showError(msg) {
    if (!errEl) return;
    errEl.textContent = msg;
    errEl.hidden = false;
  }

  btn.addEventListener("click", async () => {
    btn.disabled = true;
    btn.textContent = "Adding…";
    if (errEl) errEl.hidden = true;

    try {
      const res = await fetch("/api/symbols", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ ticker }),
      });
      let data = {};
      try {
        data = await res.json();
      } catch {
        /* a non-JSON body falls through to the generic message below */
      }

      if (res.ok && data.ok && data.ticker) {
        // Land on the freshly added symbol's page.
        window.location.href = "/s/" + encodeURIComponent(data.ticker);
        return;
      }
      showError(data.error || "Could not add that symbol. Try again shortly.");
    } catch {
      showError("Could not reach the server. Check your connection and try again.");
    }

    btn.disabled = false;
    btn.textContent = label;
  });
}
