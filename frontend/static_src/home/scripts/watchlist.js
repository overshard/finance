// Dashboard watchlist editing (Phase C).
//
// Add and remove post to /api/watchlist; on success the page reloads so the
// server re-renders the cards, re-registers the live stream tickers, and the
// hero graph re-fetches with the new set. A brand-new symbol is validated and
// backfilled server-side before it returns, so the reloaded page is complete.

async function post(url, ticker) {
  try {
    const res = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ ticker }),
    });
    return await res.json();
  } catch {
    return { ok: false, error: "Network error — try again." };
  }
}

export function initWatchlist() {
  const form = document.querySelector('[data-role="watch-add"]');
  const msg = document.querySelector('[data-role="watch-msg"]');

  function showMsg(text, isError) {
    if (!msg) return;
    msg.textContent = text;
    msg.hidden = false;
    msg.classList.toggle("is-error", !!isError);
  }

  if (form) {
    form.addEventListener("submit", async (e) => {
      e.preventDefault();
      const input = form.querySelector('[name="ticker"]');
      const ticker = (input.value || "").trim();
      if (!ticker) return;
      const btn = form.querySelector("button");
      if (btn) btn.disabled = true;
      showMsg("Adding " + ticker.toUpperCase() + "…", false);
      const res = await post("/api/watchlist", ticker);
      if (res.ok) {
        location.reload();
      } else {
        if (btn) btn.disabled = false;
        showMsg(res.error || "Could not add that symbol.", true);
      }
    });
  }

  document.querySelectorAll("[data-remove]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const ticker = btn.dataset.ticker;
      if (!ticker) return;
      btn.disabled = true;
      const res = await post("/api/watchlist/remove", ticker);
      if (res.ok) location.reload();
      else btn.disabled = false;
    });
  });
}
