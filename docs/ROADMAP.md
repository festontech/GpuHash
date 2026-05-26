# Roadmap

All phases run **on one Windows Intel laptop**. No servers, no other machines, no networked workers.

Mark a phase complete by checking its top-level box and adding a logbook entry summarizing the deliverable.

---

## Phase 0 — Workspace bootstrap (0.5–1 day)

**Goal.** Cargo workspace; `cargo check && cargo test` clean on Windows. Toolchain prerequisites installed.

- [x] Workspace `Cargo.toml`, `.gitignore`, repo docs (this file, ARCHITECTURE.md, ETHICS.md, LOGBOOK.md).
- [x] `gpuhash-core` crate with module stubs (`error`, `event`, `hash`, `config`).
- [x] `gpuhash-cli` crate with clap skeleton + `--i-own-these-hashes` gate.
- [x] Rust toolchain installed via rustup (stable-msvc).
- [x] MSVC build tools 2022 (Desktop development with C++) installed.
- [x] `cargo check --workspace` passes.
- [ ] `cargo run -p gpuhash-cli -- --help` prints usage. *(do this once, paste output into logbook)*

---

## Phase 1 — CPU prototype (1–2 days)

**Goal.** MD5 dictionary attack on CPU using the `md-5` crate; unit tests with RFC 1321 vectors.

- [x] Add `md-5`, `tokio`, `tokio-util`, `serde_json` deps. *(`rayon` deferred to Phase 4 — single-threaded CPU is sufficient for the prototype baseline.)*
- [x] Implement `Engine` (+ `RunningAttack` handle) with a CPU-only `run` (no GPU yet).
- [x] Implement `WordlistSource` + `CandidateSource` trait (`attacks.rs`).
- [x] Implement `load_targets` (one hex digest per line, `loader.rs`).
- [x] Tests: RFC 1321 MD5 test vectors all pass; loader rejects malformed input.
- [x] `cargo run -p gpuhash-cli -- attack --algo md5 --hashes examples/sample_hashes.txt --wordlist examples/tiny_dict.txt --i-own-these-hashes` prints all 10 matches.
- [x] Ethics gate verified (refuses without `--i-own-these-hashes`).
- [x] `--json` mode emits NDJSON `EngineEvent`s.
- [x] Logbook: baseline CPU H/s recorded (~1.67 MH/s release, single-threaded).

---

## Phase 2 — GPU smoke test (1 day)

**Goal.** Get Appendix A's "smallest possible compute kernel" returning `1` on the laptop's Intel iGPU.

- [x] Add `wgpu`, `bytemuck` deps to `gpuhash-core`.
- [x] Implement `gpu::smoke()` per ARCHITECTURE.md Appendix A.
- [x] One green test that the GPU plumbing works end-to-end.
- [x] Log adapter info (`Adapter::get_info`) — Vulkan backend on this Intel iGPU (not DX12; both are valid wgpu backends).
- [x] Logbook: adapter name, backend, driver version.

---

## Phase 3 — GPU MD5 (3–5 days)

**Goal.** Port MD5 to WGSL; one batch per dispatch, await each. Match results bit-for-bit against the CPU prototype.

- [x] `gpu/shaders/md5.wgsl` — full MD5 in WGSL with the 64 round constants.
- [x] `gpu/runner.rs` — bind group layout, compute pipeline, persistent buffers (subsumes the planned `pipeline.rs`).
- [x] `gpu/buffers.rs` — `CandidateSlot` / `Params` / `MatchRecord` POD types.
- [x] `Engine` routes on `Backend::Gpu`; CLI exposes `--gpu`.
- [x] Cross-check: same 10/10 matches as Phase 1 CPU prototype on the canonical example. GPU runner unit tests also confirm GPU MD5 agrees with the CPU reference across inputs of 0–26 bytes.
- [x] Logbook: first GPU H/s number (~6.4 MH/s release on Intel UHD, ~2.2× CPU single-thread — Phase 4 expected to widen the gap).

---

## Phase 4 — Scheduler & batching (2–3 days)

**Goal.** Ring buffer of staging buffers, overlapped dispatches, brute-force on-GPU candidate generation. Expect 5–20× speedup over Phase 3 on Intel iGPU.

- [x] `Scheduler` with `max_in_flight = 2` ring of staging buffers (per-slot bufs + submit/read split on the runners).
- [x] On-GPU brute-force candidate derivation from `gid.x` (`md5_bruteforce.wgsl` + `Md5BruteforceRunner` + `crate::mask`).
- [x] Tune `batch_size` (swept 16384 / 65536 / 262144; **chose 1<<18**).
- [x] Tune `workgroup_size` (swept 32 / 64 / 128 / 256; **chose 256**).
- [x] Logbook: batch / workgroup sweep results, chosen defaults.

---

## Phase 5 — SHA1 / SHA256 (2 days)

**Goal.** Port additional algorithms by templating shader code. Benchmark suite.

- [x] `gpu/shaders/sha1/{funcs,dict,bruteforce}.wgsl` — and the matching `sha256/` folder.
- [x] `gpu/shaders/sha256/{funcs,dict,bruteforce}.wgsl`.
- [x] NIST test vectors pass on both CPU and GPU paths (CPU vectors as inline tests; GPU agrees with CPU on the same inputs for all three algorithms via the runner tests).
- [x] `gpuhash benchmark` returns three numbers for *this* Intel iGPU.

---

## Phase 6 — CLI polish (1 day)

**Goal.** Subcommands, `--json`, sessions, exit codes. Test from PowerShell.

- [x] `gpuhash session list/save/load/delete` (`session show` added as a bonus).
- [x] `--json` mode emits NDJSON `EngineEvent`s (wired in Phase 1; re-verified by `scripts/smoke.ps1`).
- [x] Exit codes: 0 / 1 (matches found) / 2 (error).
- [x] PowerShell smoke test: `scripts/smoke.ps1` builds the CLI then pipes `--json` output into `ConvertFrom-Json` and exercises the full session lifecycle.

---

## Phase 7 — Tauri + vanilla-ts shell (3–4 days)

**Goal.** Commands, events, basic Dashboard with start/cancel. `npm run tauri dev` launches on Windows.

**Note.** Phase 7 was originally planned as Tauri + React. Settled on **vanilla TypeScript** instead — see logbook 2026-05-25 (Phase 7). React/Zustand stay available as a Phase 10 stretch if the UI grows.

- [x] `npm create tauri-app@latest crates/gpuhash-tauri --template vanilla-ts --tauri-version 2 -y -f`.
- [x] Added `gpuhash-core` path dependency in `crates/gpuhash-tauri/src-tauri/Cargo.toml`; new crate joined the workspace.
- [x] Implemented `start_attack`, `cancel_attack`, `benchmark`, `list_sessions`, `load_session`, `delete_session` Tauri commands.
- [x] Vanilla-ts Dashboard (attack form, live stats, match list, sessions table) replacing the template's Greet UI.
- [x] `listen("engine-event")` drains `EngineEvent`s straight into DOM updates — no Zustand needed at this size.
- [x] `npm run tauri dev` launches: Vite ready in ~430 ms, backend compiles + launches the webview window.

---

## Phase 8 — Live charts + sessions (2 days)

**Goal.** Live H/s chart, persistent sessions surfaced in the UI, demo-ready.

- [x] Hand-rolled SVG sparkline (last 60 `Progress` samples). Picked instead of recharts because the shell is vanilla-ts — see logbook 2026-05-25 (Phase 8).
- [x] `MatchesTable` streams idx + plaintext rows as matches arrive (was a `<ol>` in Phase 7).
- [x] `SessionList` has Load + Delete per row. Save is implicit via `session_name` on Run; load replays the stored matches into the UI without re-executing.
- [x] Demo script in LOGBOOK.md (2026-05-25 Phase 8 entry).

---

## Phase 9 — Optimization sweep (2–3 days)

**Goal.** Workgroup sizes (32/64/128), batch sizes (16K → 1M), thermal-aware *sustained* throughput.

- [ ] Run each algorithm with at least 5 batch sizes × 3 workgroup sizes.
- [ ] Plot H/s vs (batch × workgroup) — one chart per algorithm.
- [ ] Document chosen defaults in code with a comment pointing to the chart.
- [ ] 60-second sustained-throughput run with thermal observations recorded.

---

## Phase 10 — (Stretch) Bonus features

Pick **1–2** from `ARCHITECTURE.md §13`. The **WGSL ↔ OpenCL backend bake-off** is the highest-payoff option for course alignment.

- [ ] Chosen feature 1: ____
- [ ] Chosen feature 2: ____

---

## Out of scope (on purpose)

- Distributed / multi-machine cracking, GPU clusters, networked work-stealing.
- Cloud worker pools.
- Anything that requires more than the one Windows Intel laptop.
