import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ----- EngineEvent contract -----
// Mirrors `gpuhash_core::EngineEvent` (tagged union on `type`). Keeping this
// type in sync with the Rust enum is part of the cross-shell contract:
// changing one without the other is a breaking change.

type Algorithm = "md5" | "sha1" | "sha256";

type AttackSummary = {
  tested_total: number;
  matches_total: number;
  elapsed_secs: number;
};

type EngineEvent =
  | { type: "Started"; algo: Algorithm; total: number | null }
  | {
      type: "Progress";
      tested: number;
      hashes_per_sec: number;
      eta_secs: number | null;
    }
  | { type: "Match"; plaintext: string; target_idx: number }
  | { type: "Finished"; summary: AttackSummary }
  | { type: "Error"; message: string };

type SessionListEntry = {
  name: string;
  status: "saved" | "finished" | "error";
  updated_at: number;
  matches_total: number;
};

type AttackMode =
  | { kind: "dictionary"; wordlist: string }
  | { kind: "bruteforce"; mask: string; start: number; end: number | null };

type AttackConfig = {
  algo: Algorithm;
  hashes_path: string;
  mode: AttackMode;
  backend: "cpu" | "gpu";
  gpu_tuning: { batch_size: number | null; workgroup_size: number | null };
  session_name: string | null;
};

type SessionMatch = { plaintext: string; target_idx: number };

type DemoCorpus = {
  wordlist_path: string;
  hashes_path: string;
  candidate_count: number;
  planted_count: number;
};

type Session = {
  name: string;
  status: "saved" | "finished" | "error";
  config: AttackConfig;
  matches: SessionMatch[];
  summary: AttackSummary | null;
  created_at: number;
  updated_at: number;
};

// ----- DOM lookup with type narrowing -----

function $<T extends HTMLElement>(id: string): T {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing #${id}`);
  return el as T;
}

// ----- Live audit state -----

let unlisten: UnlistenFn | null = null;
let running = false;

const CHART_MAX_SAMPLES = 60;
const history: number[] = [];
let matchCount = 0;

function setRunning(state: boolean) {
  running = state;
  $<HTMLButtonElement>("start").disabled = state;
  $<HTMLButtonElement>("cancel").disabled = !state;
}

function resetStats() {
  $("tested").textContent = "0";
  $("rate").textContent = "0";
  $("eta").textContent = "—";
  $("elapsed").textContent = "—";
  $("status").textContent = "idle";
  $("matches-body").innerHTML = "";
  $("matches-empty").hidden = false;
  $("log").textContent = "";
  matchCount = 0;
  history.length = 0;
  drawChart();
}

function log(line: string) {
  const el = $("log");
  el.textContent += line + "\n";
  el.scrollTop = el.scrollHeight;
}

function fmtRate(hps: number): string {
  if (hps >= 1e6) return `${(hps / 1e6).toFixed(1)} MH/s`;
  if (hps >= 1e3) return `${(hps / 1e3).toFixed(1)} kH/s`;
  return `${hps.toFixed(0)} H/s`;
}

// ----- Chart -----
//
// Hand-rolled SVG sparkline. The history buffer holds at most the last 60
// `Progress.hashes_per_sec` samples; we normalize against the peak so the
// line stretches the full vertical range. Zero-deps, ~30 lines, transparent
// — see LOGBOOK 2026-05-25 (Phase 8) for the reasoning.

function drawChart() {
  const line = document.getElementById("rate-line") as SVGPolylineElement | null;
  const peak = document.getElementById("rate-peak") as SVGTextElement | null;
  if (!line || !peak) return;

  if (history.length === 0) {
    line.setAttribute("points", "");
    peak.textContent = "";
    return;
  }

  const maxRate = Math.max(...history, 1);
  const width = 600;
  const height = 120;
  const padX = 4;
  const padY = 6;
  const innerW = width - padX * 2;
  const innerH = height - padY * 2;
  // Always lay out as if the buffer were full so the rightmost sample stays
  // pinned to the right edge; fills in left-to-right as samples arrive.
  const stepX = innerW / (CHART_MAX_SAMPLES - 1);
  const startIdx = CHART_MAX_SAMPLES - history.length;

  const pts = history
    .map((v, i) => {
      const x = padX + (startIdx + i) * stepX;
      const y = padY + innerH * (1 - v / maxRate);
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  line.setAttribute("points", pts);
  peak.textContent = `peak ${fmtRate(maxRate)}`;
}

// ----- Matches table -----

function appendMatch(idx: number, plaintext: string) {
  const body = $("matches-body");
  const tr = document.createElement("tr");
  const tdIdx = document.createElement("td");
  tdIdx.className = "idx";
  tdIdx.textContent = String(idx);
  const tdText = document.createElement("td");
  tdText.textContent = plaintext;
  tr.appendChild(tdIdx);
  tr.appendChild(tdText);
  body.appendChild(tr);
  matchCount += 1;
  $("matches-empty").hidden = matchCount > 0;
}

function handleEvent(ev: EngineEvent) {
  switch (ev.type) {
    case "Started":
      $("status").textContent = `running (${ev.algo}, total=${ev.total ?? "?"})`;
      log(`started: algo=${ev.algo} total=${ev.total ?? "unknown"}`);
      break;
    case "Progress":
      $("tested").textContent = ev.tested.toLocaleString();
      $("rate").textContent = fmtRate(ev.hashes_per_sec);
      $("eta").textContent = ev.eta_secs != null ? `${ev.eta_secs.toFixed(1)}s` : "—";
      history.push(ev.hashes_per_sec);
      if (history.length > CHART_MAX_SAMPLES) history.shift();
      drawChart();
      break;
    case "Match":
      appendMatch(ev.target_idx, ev.plaintext);
      break;
    case "Finished": {
      const s = ev.summary;
      $("status").textContent = "finished";
      $("elapsed").textContent = `${s.elapsed_secs.toFixed(2)}s`;
      log(
        `done: tested=${s.tested_total} matches=${s.matches_total} elapsed=${s.elapsed_secs.toFixed(2)}s`,
      );
      setRunning(false);
      void refreshSessions();
      break;
    }
    case "Error":
      $("status").textContent = "error";
      log(`engine error: ${ev.message}`);
      setRunning(false);
      break;
  }
}

// ----- Form wiring -----

function attackMode(): AttackMode {
  const mode = (document.querySelector('input[name="mode"]:checked') as HTMLInputElement).value;
  if (mode === "dictionary") {
    return { kind: "dictionary", wordlist: $<HTMLInputElement>("wordlist").value };
  }
  return {
    kind: "bruteforce",
    mask: $<HTMLInputElement>("mask").value,
    start: 0,
    end: null,
  };
}

function attackConfig(): AttackConfig {
  const algo = $<HTMLSelectElement>("algo").value as Algorithm;
  const hashes_path = $<HTMLInputElement>("hashes").value;
  const sessionRaw = $<HTMLInputElement>("session").value.trim();
  return {
    algo,
    hashes_path,
    mode: attackMode(),
    backend: $<HTMLInputElement>("gpu").checked ? "gpu" : "cpu",
    gpu_tuning: { batch_size: null, workgroup_size: null },
    session_name: sessionRaw === "" ? null : sessionRaw,
  };
}

function applyConfigToForm(cfg: AttackConfig) {
  $<HTMLSelectElement>("algo").value = cfg.algo;
  $<HTMLInputElement>("hashes").value = cfg.hashes_path;
  $<HTMLInputElement>("gpu").checked = cfg.backend === "gpu";
  $<HTMLInputElement>("session").value = cfg.session_name ?? "";

  if (cfg.mode.kind === "dictionary") {
    (document.querySelector('input[name="mode"][value="dictionary"]') as HTMLInputElement).checked =
      true;
    $<HTMLInputElement>("wordlist").value = cfg.mode.wordlist;
    $("wordlist-label").hidden = false;
    $("mask-label").hidden = true;
  } else {
    (document.querySelector('input[name="mode"][value="bruteforce"]') as HTMLInputElement).checked =
      true;
    $<HTMLInputElement>("mask").value = cfg.mode.mask;
    $("wordlist-label").hidden = true;
    $("mask-label").hidden = false;
  }
}

async function startAttack() {
  if (running) return;
  resetStats();
  setRunning(true);
  $("status").textContent = "starting…";

  if (unlisten == null) {
    unlisten = await listen<EngineEvent>("engine-event", (e) => handleEvent(e.payload));
  }

  try {
    await invoke("start_attack", {
      config: attackConfig(),
      iOwnTheseHashes: $<HTMLInputElement>("ack").checked,
    });
  } catch (err) {
    setRunning(false);
    $("status").textContent = "error";
    log(`invoke failed: ${(err as { message?: string }).message ?? String(err)}`);
  }
}

async function cancelAttack() {
  try {
    const cancelled = await invoke<boolean>("cancel_attack");
    log(cancelled ? "cancel sent" : "nothing to cancel");
  } catch (err) {
    log(`cancel failed: ${(err as { message?: string }).message ?? String(err)}`);
  }
}

// ----- Sessions panel -----

function fmtUpdated(unix: number): string {
  if (!unix) return "—";
  return new Date(unix * 1000).toLocaleString();
}

async function refreshSessions() {
  let rows: SessionListEntry[] = [];
  try {
    rows = await invoke<SessionListEntry[]>("list_sessions");
  } catch (err) {
    log(`list_sessions failed: ${(err as { message?: string }).message ?? String(err)}`);
    return;
  }
  const body = $("sessions-body");
  body.innerHTML = "";
  if (rows.length === 0) {
    const tr = document.createElement("tr");
    tr.innerHTML = `<td colspan="5" class="muted">No saved sessions.</td>`;
    body.appendChild(tr);
    return;
  }
  for (const row of rows) {
    const tr = document.createElement("tr");
    tr.innerHTML = `
      <td>${row.name}</td>
      <td>${row.status}</td>
      <td>${row.matches_total}</td>
      <td>${fmtUpdated(row.updated_at)}</td>
      <td class="actions">
        <button data-name="${row.name}" data-action="load">Load</button>
        <button data-name="${row.name}" data-action="delete">Delete</button>
      </td>
    `;
    body.appendChild(tr);
  }
}

async function loadSession(name: string) {
  try {
    const session = await invoke<Session>("load_session", { name });
    applyConfigToForm(session.config);
    // Restore the saved view: replay stored matches into the table so the
    // user sees what the saved run found without re-executing it.
    resetStats();
    $("status").textContent = `loaded session "${session.name}"`;
    for (const m of session.matches) appendMatch(m.target_idx, m.plaintext);
    if (session.summary) {
      $("tested").textContent = session.summary.tested_total.toLocaleString();
      $("elapsed").textContent = `${session.summary.elapsed_secs.toFixed(2)}s`;
    }
    log(`loaded session ${session.name} (${session.matches.length} prior matches)`);
  } catch (err) {
    log(`load_session failed: ${(err as { message?: string }).message ?? String(err)}`);
  }
}

// ----- Random Demo panel -----

async function onGenerateDemo() {
  const count = parseInt($<HTMLInputElement>("demo-count").value, 10);
  const planted = parseInt($<HTMLInputElement>("demo-planted").value, 10);
  const algo = $<HTMLSelectElement>("demo-algo").value as Algorithm;
  if (!Number.isFinite(count) || !Number.isFinite(planted)) {
    log("invalid demo inputs");
    return;
  }

  const btn = $<HTMLButtonElement>("demo-generate");
  btn.disabled = true;
  $("demo-status").textContent = "generating…";

  try {
    const corpus = await invoke<DemoCorpus>("generate_demo_corpus", {
      count,
      planted,
      algo,
    });
    // Switch the form into "dictionary, demo paths, matching algorithm".
    $<HTMLSelectElement>("algo").value = algo;
    $<HTMLInputElement>("hashes").value = corpus.hashes_path;
    $<HTMLInputElement>("wordlist").value = corpus.wordlist_path;
    (document.querySelector('input[name="mode"][value="dictionary"]') as HTMLInputElement).checked =
      true;
    $("wordlist-label").hidden = false;
    $("mask-label").hidden = true;
    $("demo-status").textContent =
      `ready: ${corpus.candidate_count.toLocaleString()} candidates, ${corpus.planted_count} planted. Tick "I own these hashes" and Run Audit.`;
  } catch (err) {
    $("demo-status").textContent =
      `generate failed: ${(err as { message?: string }).message ?? String(err)}`;
  } finally {
    btn.disabled = false;
  }
}

async function onSessionsClick(e: Event) {
  const t = e.target as HTMLElement;
  if (t.tagName !== "BUTTON") return;
  const name = t.getAttribute("data-name");
  const action = t.getAttribute("data-action");
  if (!name) return;
  if (action === "delete") {
    try {
      await invoke("delete_session", { name });
      await refreshSessions();
    } catch (err) {
      log(`delete failed: ${(err as { message?: string }).message ?? String(err)}`);
    }
  } else if (action === "load") {
    await loadSession(name);
  }
}

// ----- Bootstrap -----

window.addEventListener("DOMContentLoaded", () => {
  $("attack-form").addEventListener("submit", (e) => {
    e.preventDefault();
    void startAttack();
  });
  $("cancel").addEventListener("click", () => void cancelAttack());
  $("refresh-sessions").addEventListener("click", () => void refreshSessions());
  $("sessions-body").addEventListener("click", onSessionsClick);
  $("demo-form").addEventListener("submit", (e) => {
    e.preventDefault();
    void onGenerateDemo();
  });

  document.querySelectorAll<HTMLInputElement>('input[name="mode"]').forEach((r: HTMLInputElement) =>
    r.addEventListener("change", () => {
      const dict =
        (document.querySelector('input[name="mode"]:checked') as HTMLInputElement).value ===
        "dictionary";
      $("wordlist-label").hidden = !dict;
      $("mask-label").hidden = dict;
    }),
  );

  drawChart();
  void refreshSessions();
});
