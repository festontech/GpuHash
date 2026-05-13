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

- [ ] `gpu/shaders/md5.wgsl` — full MD5 in WGSL with the 64 round constants.
- [ ] `gpu/pipeline.rs` — bind group layout, compute pipeline.
- [ ] `gpu/buffers.rs` — candidate / target / output buffers.
- [ ] `Engine::run_attack` with `--gpu` flag.
- [ ] Cross-check: same matches as Phase 1 CPU prototype on the same input.
- [ ] Logbook: first GPU H/s number.

---

## Phase 4 — Scheduler & batching (2–3 days)

**Goal.** Ring buffer of staging buffers, overlapped dispatches, brute-force on-GPU candidate generation. Expect 5–20× speedup over Phase 3 on Intel iGPU.

- [ ] `Scheduler` with `max_in_flight = 2` ring of staging buffers.
- [ ] On-GPU brute-force candidate derivation from `gid.x`.
- [ ] Tune `batch_size` (start at `1<<16` on iGPU; sweep).
- [ ] Tune `workgroup_size` (32 vs 64).
- [ ] Logbook: batch / workgroup sweep results, chosen defaults.

---

## Phase 5 — SHA1 / SHA256 (2 days)

**Goal.** Port additional algorithms by templating shader code. Benchmark suite.

- [ ] `gpu/shaders/sha1.wgsl`.
- [ ] `gpu/shaders/sha256.wgsl`.
- [ ] NIST test vectors pass on both CPU and GPU paths.
- [ ] `gpuhash benchmark` returns three numbers for *this* Intel iGPU.

---

## Phase 6 — CLI polish (1 day)

**Goal.** Subcommands, `--json`, sessions, exit codes. Test from PowerShell.

- [ ] `gpuhash session list/save/load/delete`.
- [ ] `--json` mode emits NDJSON `EngineEvent`s.
- [ ] Exit codes: 0 / 1 (matches found) / 2 (error).
- [ ] PowerShell smoke test: pipe `--json` output into `ConvertFrom-Json`.

---

## Phase 7 — Tauri + React shell (3–4 days)

**Goal.** Commands, events, basic Dashboard with start/cancel. `npm run tauri dev` launches on Windows.

- [ ] `npm create tauri-app@latest` to scaffold `crates/gpuhash-tauri/` (React + TypeScript template).
- [ ] Add `gpuhash-core` path dependency in the new Tauri crate.
- [ ] Implement `start_attack`, `cancel_attack`, `benchmark` commands.
- [ ] React Dashboard, AttackPanel, LiveStats.
- [ ] Listener that pushes `EngineEvent`s into a Zustand store.
- [ ] `npm run tauri dev` launches; clicking **Audit** runs the same workload as the CLI.

---

## Phase 8 — Live charts + sessions (2 days)

**Goal.** Recharts, persistent sessions, demo-ready UI.

- [ ] `recharts` line chart bound to the Zustand `history` slice.
- [ ] `MatchesTable` streaming as matches arrive.
- [ ] `SessionList` with save/load wired to backend `save_session` command.
- [ ] Demo script in LOGBOOK.md.

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
