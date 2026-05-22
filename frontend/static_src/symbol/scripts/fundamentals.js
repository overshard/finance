// Annual / quarterly toggle for the financials table. Both tables are
// server-rendered; this just shows one and hides the other.
export function initFundamentals() {
  const tabs = document.querySelectorAll(".fin__tab");
  const panels = document.querySelectorAll(".fin__panel");
  if (!tabs.length) return;

  tabs.forEach((tab) => {
    tab.addEventListener("click", () => {
      const period = tab.dataset.period;
      tabs.forEach((t) => {
        const on = t === tab;
        t.classList.toggle("is-active", on);
        t.setAttribute("aria-selected", on ? "true" : "false");
      });
      panels.forEach((p) => {
        p.hidden = p.dataset.period !== period;
      });
    });
  });
}
