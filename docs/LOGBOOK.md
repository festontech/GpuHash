# Logbook

Append-only, dated entries. The point is twofold: (a) you'll need this for the final report, (b) it forces you to articulate *why* you made choices when they were fresh.

## Template

```markdown
## YYYY-MM-DD — <one-line summary>

**Goal today.** What were you trying to accomplish?

**What I did.** Concrete steps; commands; commits (`abc123`).

**What worked.**

**What didn't / surprises.**

**Decisions made.** Choice + alternatives considered + reason.

**Numbers.** Anything measurable: H/s, batch latency, file sizes, etc.

**Next.** What unblocks tomorrow.
```

---

## 2026-05-06 — Project bootstrap (Phase 0)

**Goal today.** Get the project skeleton in place: workspace, docs, crate stubs. Do not write any hash logic yet.

**What I did.**
- Created the workspace `Cargo.toml` with two members: `gpuhash-core`, `gpuhash-cli`.
- Created `docs/ARCHITECTURE.md`, `docs/ROADMAP.md`, `docs/ETHICS.md`, this logbook, and a top-level `README.md`.
- Stubbed `gpuhash-core` with the contract types: `Algorithm`, `EngineEvent`, `AttackConfig`, `Error`. No engine logic yet — Phase 1.
- Stubbed `gpuhash-cli` with the clap surface (`attack`, `benchmark` subcommands) and the `--i-own-these-hashes` gate. Stubs print "Phase 0 stub".
- Decided on target environment: **single Windows laptop with Intel CPU/GPU**. Distributed mode is out of scope.

**What worked.** File scaffolding only — nothing to validate yet beyond `cargo check`.

**What didn't / surprises.** Rust toolchain isn't installed yet on this machine — that's the next entry.

**Decisions made.**
- **wgpu over OpenCL** for the GPU layer, despite school teaching OpenCL. Reason: keeps the entire stack in Rust without FFI, the rest of the project (Tauri, serde, clap) benefits, and we accept ~10–30% throughput cost. *Mitigation:* if there's time in Phase 10, add an OpenCL backend behind a `HashKernel` trait so we can present a head-to-head comparison. (See ARCHITECTURE.md §4.1.1.)
- **Tauri over Electron.** Smaller, faster, and the Rust backend has zero IPC translation cost.
- **Workspace with `gpuhash-core` shared.** Both CLI and (later) Tauri call into the same crate — no duplication.

**Numbers.** N/A.

**Next.**
- Install rustup (stable-msvc) + MSVC Build Tools 2022 (Desktop C++ workload) + WebView2 runtime check.
- Run `cargo check --workspace` and `cargo run -p gpuhash-cli -- --help`.
- Begin Phase 1: CPU MD5 prototype with `md-5` crate and RFC 1321 test vectors.

---

## 2026-05-06 — Toolchain install + first build (Phase 0 finish)

**Goal today.** Get `cargo check --workspace` green.

**What I did.** Installed Rust + MSVC build tools, ran `cargo check --workspace`. Passed cleanly.

**What worked.** Workspace compiles; `Algorithm` `FromStr` impl wires through clap via the small `parse_algo` adapter in `main.rs`.

**What didn't / surprises.** *(none recorded)*

**Decisions made.** Kept clap dependency out of `gpuhash-core` (the alternative was implementing `clap::ValueEnum` on `Algorithm`, which would couple the engine crate to clap). Adapter function in the CLI is the right boundary.

**Numbers.** N/A — no engine logic yet.

**Next.** Phase 1: add `md-5`, `tokio`, `tokio-util`, `rayon` to `gpuhash-core`; build a CPU-only `Engine::run_attack` that does dictionary MD5; RFC 1321 vectors as tests; record baseline CPU H/s.

---

## 2026-05-06 — CPU MD5 baseline (Phase 1)

**Goal today.** End-to-end CPU dictionary attack working with RFC 1321 test vectors passing. Record baseline CPU H/s.

**What I did.**
- Added Phase 1 deps to `gpuhash-core`: `tokio`, `tokio-util`, `md-5`. Added `serde_json` workspace-wide for the CLI's `--json` mode.
- Created four new modules in `gpuhash-core`:
  - `digest.rs` — algorithm-dispatching CPU hasher built on the `md-5` crate. RFC 1321 §A.5 vectors as inline tests.
  - `loader.rs` — parses one-hex-digest-per-line hash files; rejects bad length, bad hex, empty files, with line numbers in error messages.
  - `attacks.rs` — `CandidateSource` trait + `WordlistSource` (line-by-line `BufReader`). Pre-counts lines so `Started` events have a `total` for ETA.
  - `engine.rs` — `Engine` + `RunningAttack { events, cancel }`. Spawns the work on `tokio::spawn`; events flow over `mpsc::unbounded_channel`. Progress throttled to ~10 Hz.
- Refactored `EngineEvent::Finished` to carry an `AttackSummary` struct (cleaner Rust + JS shape).
- Rewrote CLI `main.rs` from stub → real driver. Renders human-readable progress on stderr or NDJSON on stdout (`--json`). Exit codes per the architecture doc: `0` no matches, `1` matches found, `2` error / refusal.
- 8 unit tests pass: RFC 1321 vectors (×7), unsupported-algo `NotImplemented`, loader good/bad/empty cases, algorithm parsing.
- Regenerated `examples/sample_hashes.txt` from PowerShell-computed MD5s — the original file (which I'd hand-typed) had a wrong digest on line 5.

**What worked.** End-to-end audit on the 10-word example: 10/10 matches, exit code 1. NDJSON output is well-formed. Ethics gate refuses cleanly.

**What didn't / surprises.**
- First `cargo check` failed: I'd put `tokio-util = { features = ["sync"] }` in the workspace deps. `CancellationToken` is in `tokio_util::sync` but is part of the **default** API — no feature flag needed. Removed `features` and it compiled.
- The hand-typed `examples/sample_hashes.txt` had a wrong digest for "welcome" — caught immediately by the engine reporting 9/10 matches. Good early validation that the comparator is strict.

**Decisions made.**
- **Single-threaded for Phase 1.** Roadmap originally listed `rayon` as a Phase 1 dep, but adding parallel candidate iteration interacts awkwardly with the streaming-progress model (multiple threads competing to emit `Progress` events). Deferred to Phase 4 where the GPU dispatch path naturally needs parallel CPU prep.
- **`Engine` returns `RunningAttack` rather than a tuple of `(EventStream, JoinHandle<Result<AttackSummary>>)`.** The architecture doc had the latter shape; the former is one type, easier to pass through the Tauri command boundary later, and the summary travels in `EngineEvent::Finished` so consumers don't juggle two channels. Documented in the engine docstring.
- **No external `hex` crate.** Wrote a 6-line `parse_hex` inline. Avoids one dependency.

**Numbers.**
- 10-word example, debug build: tested=10, matches=10, elapsed≈0.00s (too short to measure reliably).
- 100,000-candidate synthetic dict, **release build**: tested=100,000, elapsed=0.06s, **rate ≈ 1.67 MH/s** single-threaded MD5 on this Intel laptop.
- Reality check: this is a CPU baseline for a single core. Modern CPUs hit ~10–50 MH/s per core for MD5 with vectorized intrinsics; the `md-5` crate is a portable scalar implementation, so 1–2 MH/s is the right ballpark. Phase 4 with rayon should hit `cores × this`. Phase 3 GPU should jump 1–2 orders of magnitude.

**Next.**
- Phase 2: GPU smoke test. Add `wgpu` + `bytemuck`. Get Appendix A's smallest-possible compute kernel (`data[0] = 1u`) to round-trip on the Intel iGPU. Log adapter info — confirm DX12 backend.

---

## 2026-05-13 — GPU smoke test (Phase 2)

**Goal today.** End-to-end GPU plumbing: adapter → device → pipeline → buffer round-trip. Confirm wgpu works on this Intel iGPU before Phase 3 ports MD5 to WGSL.

**What I did.**
- Uncommented `wgpu = "22"` and `bytemuck` in `gpuhash-core/Cargo.toml`; added `gpu` module to [lib.rs](../crates/gpuhash-core/src/lib.rs).
- Wrote [gpu.rs](../crates/gpuhash-core/src/gpu.rs) `smoke()` per ARCHITECTURE.md Appendix A — single-element storage buffer, WGSL kernel `data[0] = 1u`, COPY_SRC → MAP_READ staging buffer, `device.poll(Wait)` then mapped-range read.
- Added `tracing-subscriber` as a dev-dependency so the test can install a subscriber and surface `Adapter::get_info()` on `--nocapture`. Production consumers of the library install their own subscriber (the CLI already does).
- `cargo test --workspace`: 9/9 pass (the new `gpu::tests::smoke_returns_one` joins the 8 from Phase 1). `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` both clean.

**What worked.** First run returned `1u` cleanly. No driver crashes, no validation errors. wgpu's default adapter selection picked the integrated GPU without me having to set power preference.

**What didn't / surprises.**
- **Backend was Vulkan, not DX12.** The roadmap predicted DX12 on Windows + Intel. wgpu 22 on this machine prefers Vulkan when both backends are available, and the Intel driver exposes a Vulkan ICD (driver_info `101.7084`). Functionally equivalent for our purposes — both go through `wgpu_hal` and end at the same Intel compute units. Noted in the roadmap checkbox so I don't chase this later thinking something is misconfigured.
- `wgpu::Instance::request_adapter` returns `Option` rather than `Result` in this version, so I wrapped it with `ok_or_else` into the engine's `Error::Gpu`. Not a surprise so much as a small API-shape adjustment from how `request_device` looks.
- First clean compile of `wgpu` + transitive deps was ~80s. Worth noting as the new cost-of-touching the GPU crate; incremental rebuilds are sub-second.

**Decisions made.**
- **`tracing-subscriber` as dev-dep, not regular dep.** A library should not install a global subscriber; that's a binary's job. But the smoke test is the one place inside the library where we want adapter info actually printed (so the logbook entry can quote it). Dev-dep + `with_test_writer()` + `try_init()` gives us that without leaking into the public dependency graph.
- **Keep the smoke test as a real `#[tokio::test]`, not a manual `cargo run` invocation.** Phase 3 will keep building on this path (real MD5 dispatches), and a passing CI-able test is much more valuable than a one-off binary that drifts.
- **Did not gate the test with `#[ignore]` or a feature flag.** Risk: CI machines without a GPU adapter would fail. Acceptable for now — the project is scoped to "single Windows laptop with Intel iGPU"; if we ever wire up a headless CI, we'll add `#[ignore]` then.

**Numbers.**
- Adapter: **Intel(R) UHD Graphics**, vendor `32902` (0x8086, Intel), `IntegratedGpu`.
- Backend: **Vulkan**, driver `Intel Corporation` `101.7084`.
- Test wall time: 0.29s (debug build, including device init).
- No H/s yet — single dispatch, no throughput meaning.

**Next.**
- Phase 3: port MD5 to WGSL. `gpu/shaders/md5.wgsl` with the 64 round constants. `gpu/pipeline.rs` for layout + compute pipeline (build once, reuse). `gpu/buffers.rs` for candidate/target/output buffers (allocate once per run, never `MAP_READ` the hot path). Wire `--gpu` flag into the CLI; cross-check matches against the Phase 1 CPU prototype on the same input before trusting any throughput number.

---

## 2026-05-13 — GPU MD5 (Phase 3)

**Goal today.** Port MD5 to WGSL, run it on the Intel iGPU, and prove bit-exact agreement with the Phase 1 CPU reference. Record the first GPU H/s number.

**What I did.**
- Converted `gpu.rs` to a `gpu/` directory module so submodules can live alongside the shader assets:
  - [gpu/mod.rs](../crates/gpuhash-core/src/gpu/mod.rs) — keeps Phase 2 `smoke()`, exports submodules.
  - [gpu/shaders/md5.wgsl](../crates/gpuhash-core/src/gpu/shaders/md5.wgsl) — full MD5 (64 rounds, 4 mixing functions, message-schedule index per round group). One thread per candidate; on a digest match, atomically reserves a slot in a match buffer.
  - [gpu/buffers.rs](../crates/gpuhash-core/src/gpu/buffers.rs) — `CandidateSlot` (60-byte POD, `len: u32 + bytes: [u32; 14]`), `Params`, `MatchRecord`. `pack()` packs a byte slice little-endian into `bytes`; caps at `MAX_CANDIDATE_LEN = 55` (single-block MD5).
  - [gpu/runner.rs](../crates/gpuhash-core/src/gpu/runner.rs) — `Md5GpuRunner` owns device, queue, pipeline, and the persistent storage buffers. `dispatch_batch()` writes candidates, zeroes the match-buffer header, dispatches, copies match-buffer to a `MAP_READ` staging buffer, awaits, returns `Vec<MatchRecord>`.
- Added [`Backend` enum](../crates/gpuhash-core/src/config.rs) (`Cpu | Gpu`) on `AttackConfig`. `#[serde(default)]` keeps any pre-Phase-3 session JSON readable.
- Refactored [engine.rs](../crates/gpuhash-core/src/engine.rs) to route on the backend: the CPU loop is unchanged; the GPU loop fills a batch from the candidate source, dispatches via `Md5GpuRunner`, looks each `MatchRecord` back up by candidate index, and emits the same `EngineEvent::Match` shape the CPU path emits. Progress events still throttled to ~10 Hz.
- Added `--gpu` to the CLI's `attack` subcommand. The user-facing event stream is unchanged.
- New tests: `gpu::buffers::tests::*` (pack endianness, oversize rejection, layout invariants) and `gpu::runner::tests::*` (8-input batch with CPU-generated targets — every candidate must match its own target; bogus-target case — no matches). Total now 16/16 passing.

**What worked.** Once the kernel compiled, MD5 was correct on the first run for all 8 short inputs in the runner test. Canonical Phase 1 audit (`examples/tiny_dict.txt` against `examples/sample_hashes.txt`) finds 10/10 matches with `--gpu`, identical to the CPU path, exit code 1 as designed.

**What didn't / surprises.**
- **naga rejected dynamic indexing into `const` arrays and into `let`-bound struct fields.** First validation error was `m[i] = slot.bytes[i]` — `let slot = candidates[..]` gave a value-of-array that can only be const-indexed. Changing `let slot` to `var slot` (function-local mutable copy) fixed that, then the same rule fired for `K[i]`/`S[i]`. Resolution: declare K and S as `var<private>` with const initializers. That's the idiomatic WGSL workaround for "a read-only table I want to walk with a dynamic index", and it's documented in the shader comment so I don't keep re-learning it.
- **Modest 2.2× GPU/CPU speedup on a 2 M synthetic dict** — much less than the "1–2 orders of magnitude" I had in mind. Explanations:
  - Per-thread MD5 is ~100 instructions of integer work; on an iGPU each batch is bottlenecked by dispatch + readback latency, not arithmetic.
  - The Phase 3 dispatch model is intentionally serial: each batch fully syncs (`map_async` blocks the next dispatch). Phase 4's ring-buffer scheduler is the design that removes that floor.
  - The host is doing real CPU work per candidate (read line, UTF-8 validate, allocate `String`, pack into 60-byte slot). Once Phase 4 generates candidates on-GPU from `gid.x`, that goes away.
  - This matches CLAUDE.md's framing: the "1–2 OOM jump" line assumed brute-force on-GPU candidate generation was already wired; dictionary mode through the host pipeline is the wrong workload to measure on.
- **Windows Application Control blocked first release build** of an unsigned wgpu build script (`glutin_wgl_sys` build-script-build, OS error 4551). Exempting the cargo target dir / disabling Smart App Control unblocked it. Flagging in case it bites again on a fresh wgpu transitive-dep upgrade.
- **`MAX_CANDIDATE_LEN = 55` bytes** is the single-block MD5 cap (9 bytes consumed by the 0x80 marker and the 8-byte bit-length). Longer candidates are silently skipped with a final-tally `tracing::warn!`. Real wordlists fit easily; multi-block lands when needed.

**Decisions made.**
- **Match buffer as a single combined struct** (`count: atomic<u32>` + `_pad: [u32; 3]` + `pairs: array<u32>`) instead of two separate bindings. One bind group entry, one staging copy, one map_async.
- **Pipeline construction is per-`Engine::run` call**, not cached on the `Engine`. Means ~150–300 ms of adapter/device/pipeline init each time. Cheap to fix in Phase 7 (cache a runner on the Tauri command handler), not worth doing yet.
- **`max_matches = batch_size`** is the per-dispatch cap. Each candidate produces one digest, and with a collision-resistant hash that maps to at most one target match, so this is a safe upper bound on matches-per-dispatch without inflating the staging buffer.
- **Did not extract `pipeline.rs` separately** as the roadmap had drafted. Pipeline + buffers + dispatch are all owned by `Md5GpuRunner`; splitting them into separate files would have meant exposing internals through a layer that doesn't pay rent yet. Phase 5 (SHA-1/SHA-256) will revisit — that's the right moment to factor out a `HashRunner` trait.

**Numbers.** Release build, 2 000 000-line synthetic dict (`candidate-1` … `candidate-2000000`), 10 targets from `examples/sample_hashes.txt`, zero expected matches:

| Backend | Elapsed | Rate |
| --- | --- | --- |
| CPU (single-thread `md-5`) | 0.69 s | **~2.91 MH/s** |
| GPU (Intel UHD Graphics, Vulkan, this code) | 0.31 s | **~6.44 MH/s** |

CPU number is up from 1.67 MH/s in the Phase 1 entry — that earlier measurement was on a 100 k dict where process-startup tax was a much larger fraction. The 2 M result is the steadier number to anchor against.

Correctness: `cargo run -p gpuhash-cli -- attack --algo md5 --hashes examples/sample_hashes.txt --wordlist examples/tiny_dict.txt --i-own-these-hashes --gpu` finds the same 10 matches as the CPU path. With 10 candidates fitting in one batch, the match order is deterministic for this case; larger batches won't be (matches arrive in atomic-counter-order, not candidate-index order).

**Next.**
- Phase 4: scheduler + on-GPU bruteforce. Ring buffer of staging buffers with `max_in_flight = 2` so dispatch N+1 starts before dispatch N's readback finishes. Move bruteforce candidate generation into the shader (derived from `gid.x` against a mask). Expect this to be where the GPU finally pulls 1–2 orders of magnitude ahead. CLAUDE.md target: `batch_size = 1<<16`, workgroup 32 vs 64 sweep.

---
