// Data-health page.
//
// The page ships with an embedded snapshot (#health-data) so it renders at
// once with no flash. From there it stays live off the shared market stream:
// base/stream.js re-broadcasts every SSE `health` nudge as a `finance:health`
// window event, and this script answers each one by pulling a fresh snapshot
// from /api/health and repainting. It also repaints when the tab regains
// focus, and re-renders every 30s so the relative times stay honest.

const $ = (sel) => document.querySelector(sel);

// ---- formatting (mirrors the server-side minijinja filters in templates.rs) ----

const DASH = "·";

function esc(s) {
  return String(s).replace(
    /[&<>"']/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c],
  );
}

// Epoch-ms in the past -> "4m ago" (mirrors the `ago` filter).
function ago(ms) {
  if (ms == null) return DASH;
  const s = Math.round((Date.now() - ms) / 1000);
  if (s < 5) return "just now";
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}

// Epoch-ms in the future -> "in 4m"; already elapsed -> "due now".
function until(ms) {
  if (ms == null) return DASH;
  const s = Math.round((ms - Date.now()) / 1000);
  if (s <= 0) return "due now";
  if (s < 60) return `in ${s}s`;
  if (s < 3600) return `in ${Math.floor(s / 60)}m`;
  if (s < 86400) return `in ${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
  return `in ${Math.floor(s / 86400)}d`;
}

// Epoch-ms -> local 24-hour clock time, e.g. "14:03:21".
function clock(ms) {
  return new Date(ms).toLocaleTimeString("en-US", { hour12: false });
}

function num(n) {
  return n == null ? DASH : n.toLocaleString("en-US");
}

function dur(ms) {
  if (ms == null) return null;
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
}

// ---- badges ----

// A breaker / job / log status maps to one of four visual tones.
const BREAKER_TONE = { closed: "ok", half_open: "warn", open: "bad" };
const JOB_TONE = { ok: "ok", fetching: "warn", error: "bad", stale: "warn", idle: "idle" };
const LOG_TONE = { ok: "ok", skipped: "warn", error: "bad" };

function badge(text, tone) {
  return `<span class="badge badge--${tone}">${esc(text)}</span>`;
}

function errKv(label, msg, at) {
  if (!msg) return "";
  return `<div class="kv kv--err"><dt>${label}</dt>
    <dd>${esc(msg)} <span class="muted">${esc(ago(at))}</span></dd></div>`;
}

// ---- region renderers ----

function renderEndpoints(list) {
  if (!list.length) {
    return `<p class="health-empty">No endpoint has been contacted yet.</p>`;
  }
  const cards = list.map((e) => {
    const tone = BREAKER_TONE[e.state] || "idle";
    const breaker = e.state === "half_open" ? "half-open" : e.state;
    const fillTone =
      e.budget_pct >= 90 ? " track__fill--bad" : e.budget_pct >= 75 ? " track__fill--warn" : "";
    const resets =
      e.hour_start != null && e.hour_count > 0
        ? `<p class="meter__note">Budget window resets ${esc(until(e.hour_start + 3600000))}.</p>`
        : "";
    const probe =
      e.state === "open" && e.retry_at != null
        ? `<div class="kv"><dt>Probe</dt><dd>${esc(until(e.retry_at))}</dd></div>`
        : "";
    return `<article class="endpoint">
      <div class="endpoint__head">
        <h3>${esc(e.label)}</h3>
        ${badge(breaker, tone)}
      </div>
      <div class="meter">
        <div class="meter__row">
          <span class="eyebrow">Hourly request budget</span>
          <span class="num meter__count">${num(e.hour_count)} / ${num(e.hourly_budget)}</span>
        </div>
        <div class="track meter__track">
          <span class="track__fill${fillTone}" style="width:${e.budget_pct}%"></span>
        </div>
        ${resets}
      </div>
      <dl class="kvs">
        <div class="kv"><dt>Circuit trips</dt><dd class="num">${num(e.trip_count)}</dd></div>
        <div class="kv"><dt>Failure streak</dt><dd class="num">${num(e.fail_streak)}</dd></div>
        <div class="kv"><dt>Last success</dt><dd>${esc(ago(e.last_ok_at))}</dd></div>
        ${probe}
        ${errKv("Last error", e.last_error, e.last_error_at)}
      </dl>
    </article>`;
  });
  return `<div class="endpoint-grid">${cards.join("")}</div>`;
}

function renderJobs(list) {
  if (!list.length) {
    return `<p class="health-empty">No job has run yet.</p>`;
  }
  const rows = list.map((j) => {
    const tone = JOB_TONE[j.state] || "idle";
    const nextRun =
      j.state === "fetching"
        ? "running now"
        : j.next_run_at != null
          ? until(j.next_run_at)
          : DASH;
    return `<article class="job${j.state === "fetching" ? " job--active" : ""}">
      <div class="job__head">
        <div class="job__id">
          <h3>${esc(j.label)}</h3>
          ${j.description ? `<p>${esc(j.description)}</p>` : ""}
        </div>
        ${badge(j.state, tone)}
      </div>
      <dl class="kvs">
        <div class="kv"><dt>Last success</dt><dd>${esc(ago(j.last_ok_at))}</dd></div>
        <div class="kv"><dt>Next run</dt><dd>${esc(nextRun)}</dd></div>
        ${errKv("Last error", j.last_error, j.last_error_at)}
      </dl>
    </article>`;
  });
  return `<div class="job-list">${rows.join("")}</div>`;
}

function renderLog(list) {
  if (!list.length) {
    return `<p class="health-empty">The fetch log is empty.</p>`;
  }
  const rows = list.map((r) => {
    const tone = LOG_TONE[r.status] || "idle";
    const meta = [r.rows != null ? `${num(r.rows)} rows` : null, dur(r.duration_ms)]
      .filter(Boolean)
      .join(" · ");
    return `<li class="logrow logrow--${tone}">
      <span class="logrow__time num">${clock(r.started_at)}</span>
      <span class="logrow__job">${esc(r.job)}</span>
      ${badge(r.status, tone)}
      <span class="logrow__detail">${r.detail ? esc(r.detail) : ""}</span>
      <span class="logrow__meta num">${esc(meta)}</span>
    </li>`;
  });
  return `<div class="log"><ul class="logrows">${rows.join("")}</ul></div>`;
}

// The top systems verdict (Phase 7): distil the whole snapshot into one plain
// read — overall tone, a headline, and a supporting clause. Tone is the worst
// thing on the page: a tripped breaker or an errored job is bad; a recovering
// breaker or a stale job is working; otherwise all-clear. A mid-fetch job is
// normal and does not darken the tone (the live banner below names it).
function renderVerdict(snap) {
  const el = $('[data-role="verdict"]');
  if (!el) return;
  const eps = snap.endpoints || [];
  const jobs = snap.jobs || [];
  const log = snap.log || [];

  const epOpen = eps.filter((e) => e.state === "open").length;
  const epHalf = eps.filter((e) => e.state === "half_open").length;
  const epHealthy = eps.filter((e) => e.state === "closed").length;
  const jobErr = jobs.filter((j) => j.state === "error").length;
  const jobStale = jobs.filter((j) => j.state === "stale").length;
  const fetching = jobs.filter((j) => j.state === "fetching").length;

  let tone, head;
  if (epOpen || jobErr) {
    tone = "bad";
    head = "Data flow degraded";
  } else if (epHalf || jobStale) {
    tone = "warn";
    head = "Recovering";
  } else {
    tone = "ok";
    head = "All systems normal";
  }

  // Sources clause: "both data sources healthy" reads best at the usual two
  // (Yahoo + SEC), with a fraction when any is down.
  let srcPart;
  if (!eps.length) {
    srcPart = "no sources contacted yet";
  } else if (epHealthy === eps.length) {
    srcPart = eps.length === 2 ? "both data sources healthy" : `all ${eps.length} data sources healthy`;
  } else {
    srcPart = `${epHealthy}/${eps.length} data sources healthy`;
  }

  // Jobs clause: how many are on schedule (anything not errored or stale).
  let jobPart;
  if (!jobs.length) {
    jobPart = "no jobs yet";
  } else if (jobErr || jobStale) {
    jobPart = `${jobs.length - jobErr - jobStale}/${jobs.length} jobs on schedule`;
  } else {
    jobPart = `${jobs.length} jobs on schedule`;
  }

  const parts = [srcPart, jobPart];
  if (fetching) parts.push("fetching now");
  else if (log.length) parts.push(`last fetch ${ago(log[0].started_at)}`);

  el.dataset.tone = tone;
  el.hidden = false;
  $('[data-role="verdict-head"]').textContent = head;
  $('[data-role="verdict-detail"]').textContent = parts.join(` ${DASH} `);
}

function renderBanner(jobs) {
  const banner = $('[data-role="banner"]');
  if (!banner) return;
  const active = jobs.filter((j) => j.state === "fetching");
  if (!active.length) {
    banner.hidden = true;
    banner.innerHTML = "";
    return;
  }
  const names = active.map((j) => esc(j.label)).join(", ");
  banner.hidden = false;
  banner.innerHTML = `<span class="health-banner__dot"></span>
    <span>Fetching now — ${names}</span>`;
}

export function initHealth() {
  const dataEl = document.getElementById("health-data");
  if (!dataEl) return;

  // The most recent snapshot, kept so the relative-time ticker can repaint
  // without another request.
  let current = null;

  function render(snap) {
    current = snap;
    renderVerdict(snap);
    $('[data-role="endpoints"]').innerHTML = renderEndpoints(snap.endpoints);
    $('[data-role="jobs"]').innerHTML = renderJobs(snap.jobs);
    $('[data-role="log"]').innerHTML = renderLog(snap.log);
    renderBanner(snap.jobs);
    $('[data-role="asof"]').textContent = `updated ${ago(snap.generated_at)}`;
  }

  try {
    render(JSON.parse(dataEl.textContent));
  } catch {
    return; // a malformed embed: leave the empty shell rather than throw
  }

  let pending = false;
  async function refresh() {
    if (pending) return;
    pending = true;
    try {
      const res = await fetch("/api/health", { headers: { Accept: "application/json" } });
      if (res.ok) render(await res.json());
    } catch {
      /* a failed poll just leaves the last snapshot up */
    } finally {
      pending = false;
    }
  }

  // A job's start and finish can land within a few ms of each other; debounce
  // so a burst of nudges costs one /api/health pull.
  let timer = null;
  window.addEventListener("finance:health", () => {
    clearTimeout(timer);
    timer = setTimeout(refresh, 250);
  });

  // Catch up after the tab was hidden, and keep "4m ago" honest while idle.
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) refresh();
  });
  setInterval(() => {
    if (current && !document.hidden) render(current);
  }, 30000);
}
