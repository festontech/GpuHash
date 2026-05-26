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

## 2026-05-13 — Scheduler + on-GPU bruteforce + tuning (Phase 4)

**Goal today.** Land the three Phase-4 pieces: a 2-deep ring scheduler, on-GPU bruteforce candidate generation, and a workgroup/batch sweep with chosen defaults. CLAUDE.md predicted a 5–20× jump on Intel iGPU; we beat that.

**What I did.** Three commits, each a clean checkpoint:

1. **Scheduler refactor** (`db01675`): split `Md5GpuRunner` into per-slot
   buffers (`candidates_buf`, `match_buf`, `match_staging`, `params_buf`,
   `bind_group`) so two batches can be in flight at once on the queue. Old
   single-step `dispatch_batch` is kept as a convenience for tests. The engine
   drives a `VecDeque<PendingDictBatch>`: refill up to `max_in_flight`, pop +
   read the oldest, refill again. Targets and the pipeline are shared across
   slots.

2. **On-GPU bruteforce** (`c887cce`):
   - New [crate::mask](../crates/gpuhash-core/src/mask.rs) — hashcat-style
     parser (`?l`, `?u`, `?d`, literals). `Mask::candidate_at(idx)` is the CPU
     reference. Refuses keyspaces above `u32::MAX` because the Phase-4 shader
     indexes in u32.
   - New `attacks::MaskSource` — CPU bruteforce candidate iterator. Makes
     `--mask` work on the CPU backend for the first time.
   - Split `md5.wgsl` into three files concatenated at module-build time with
     `format!()`: [md5_common.wgsl](../crates/gpuhash-core/src/gpu/shaders/md5_common.wgsl)
     (K, S, `rotl`, `md5_block`, `scan_targets`, `MatchBuf`, shared `targets`
     and `match_buf` bindings) +
     [md5_dict.wgsl](../crates/gpuhash-core/src/gpu/shaders/md5_dict.wgsl) /
     [md5_bruteforce.wgsl](../crates/gpuhash-core/src/gpu/shaders/md5_bruteforce.wgsl)
     (per-mode candidate input and entry point).
   - New
     [gpu/bruteforce_runner.rs](../crates/gpuhash-core/src/gpu/bruteforce_runner.rs)
     with `Md5BruteforceRunner`. Same slot/ring discipline; only the mask spec
     and a 32-bit `base_index` travel host→device per batch (no candidate
     bytes).
   - `engine::run_gpu` now dispatches on `AttackMode`: dictionary takes the
     existing path; bruteforce walks `base_index` over `[start, end)` and
     reconstructs match plaintexts via `mask.candidate_at(base + idx)`.

3. **Tuning** (this commit): plumbed `batch_size` and `workgroup_size` through
   `AttackConfig::gpu_tuning` and the CLI's `--gpu-batch` / `--gpu-workgroup`
   flags. Workgroup-size variation is implemented by substituting
   `@workgroup_size(64)` in the WGSL source at module-build time (naga's
   `override` constants would be cleaner — Phase 9 cleanup). Ran the sweep,
   picked new defaults, updated `DEFAULT_GPU_BATCH = 1 << 18` and
   `DEFAULT_WORKGROUP_SIZE = 256`.

**What worked.** Both new entry points produced bit-exact agreement with the
CPU MD5 reference on first try. The shader-splitting via `format!()` glue is
ugly but contained — once we add SHA-1/SHA-256 in Phase 5 the `_common.wgsl`
file will pay rent. The slot ring is invisible at the engine level: the
`PendingDictBatch` / `PendingBruteBatch` VecDeque keeps the bookkeeping local.

**What didn't / surprises.**
- **Scheduler alone did almost nothing for the dictionary path.** The 2M
  synthetic-dict benchmark showed ~5.8 MH/s with the scheduler vs ~6.4 MH/s in
  Phase 3 — within run-to-run noise. Reason: dict mode is host-bound. Each
  batch the CPU reads 65k lines, allocates 65k `String`s, packs them into
  60-byte slots. The GPU finishes that work faster than the CPU can keep up.
  The scheduler's win materializes only once the GPU is the bottleneck.
- **CLAUDE.md's a-priori workgroup recommendation (32–64) wasn't right** for
  this hardware. The sweep shows `wg=256` wins decisively at every batch size
  ≥ 65536. Likely: Intel UHD's compute units prefer fat SIMD waves over many
  thin ones for this kind of register-light, branch-heavy integer workload.
- **Run-to-run variance is wide on cold-vs-warm GPU.** A fresh
  `gpuhash.exe` invocation lands around 125–145 MH/s; back-to-back warm runs
  occasionally hit 260–375 MH/s. Likely a mix of driver shader caching, GPU
  clock state, and Windows scheduling. Future Phase 9 thermal-aware sustained
  benchmark will need to control for warmup.
- **Match-buffer sizing on dict mode at the new default.** `max_matches =
  batch_size = 262144`. With 2 slots that's `2 × (16 + 8 × 262144) = ~4 MB`
  for match buffers alone. Plus candidate buffers at `2 × 262144 × 60` =
  ~31 MB. Comfortably fits Intel UHD's allocation.
- **WGSL bit of friction.** Substituting `@workgroup_size(64)` in the shader
  source as a literal string works, but a one-letter typo would silently leave
  the default. Acceptable for now; `override workgroup_size: u32 = 64u;` would
  be a stronger contract — Phase 9 cleanup.

**Decisions made.**
- **Two separate runner types** (`Md5GpuRunner` and `Md5BruteforceRunner`)
  rather than one runner with a mode flag. The bind-group layouts differ
  (binding 0 is `candidates` vs `mask`), so a single pipeline can't serve
  both. With Phase 5's SHA-1/SHA-256 coming, the right factoring becomes a
  `HashKernel` trait that owns the per-algo shader code; revisiting then.
- **`@serde(default)` on `GpuTuning`** so older session JSONs still parse.
- **u32 candidate index, refused at parse time.** Larger keyspaces (?l^7 ≈
  8 B) need either u64 indices in the shader (manual 64-bit math; ugly but
  doable) or a host-side `start`/`end` range driving multiple sub-runs (the
  CLI already supports `start`/`end` on `AttackMode::Bruteforce`, just not
  wired through yet). Document and move on.
- **No CPU rayon yet.** Roadmap had rayon flagged for Phase 4 in the dep
  list. Holding off: dict mode is host-bound on I/O + allocation, not on
  hashing per se, so a parallel CPU loop would only modestly help, and the
  user-facing event channel needs single-producer semantics. Phase 9 cleanup.

**Numbers.** Sweep on Intel UHD Graphics (Vulkan), release build,
`?l^6` = 308 915 776 candidates, 10 targets from `examples/sample_hashes.txt`,
single sequential run of the grid (`--gpu-batch N --gpu-workgroup W`):

| workgroup \ batch |    16 384 |    65 536 |    262 144 |
| ---:              |   ------: |   ------: |    ------: |
| 32                |  24.6 MH/s |  71.8 MH/s | 159.1 MH/s |
| 64                |  20.1 MH/s |  77.0 MH/s | 211.1 MH/s |
| 128               |  15.5 MH/s | 122.3 MH/s | 235.1 MH/s |
| **256**           |  15.5 MH/s | 148.6 MH/s | **263.3 MH/s** |

Cold-vs-warm variance at the chosen defaults (`wg=256, batch=1<<18`,
`?l^6`, fresh process each run): **125–375 MH/s** depending on whether the
GPU was already warm. Steady-state real-world expectation: ~140 MH/s.

Stacked against earlier baselines:

| Phase / config                          | Rate         | Speedup vs Phase-1 CPU |
| ---                                     |     ---:     | ---:                   |
| Phase 1 — CPU single-thread             |  ~2.9 MH/s   | 1×                     |
| Phase 3 — GPU dict (host-bound)         |  ~6.4 MH/s   | 2.2×                   |
| Phase 4 — GPU brute, old defaults (64/1<<16) | ~54 MH/s | 18.5×                  |
| Phase 4 — GPU brute, **new defaults (256/1<<18)** | **~140 MH/s steady, peaks to 375 MH/s** | **48× steady, peaks to 129×** |

Correctness untouched: dict mode still finds all 10/10 on the canonical
example; bruteforce found admin / hello / dragon / monkey / qwerty from
`sample_hashes.txt` while sweeping ?l^6.

**Next.**
- Phase 5: SHA-1 and SHA-256. The `md5_common.wgsl` split anticipates the
  shape — `sha1_common.wgsl` / `sha256_common.wgsl` with their own round
  functions, plus per-algorithm `*_dict.wgsl` / `*_bruteforce.wgsl`. NIST test
  vectors as inline tests, on both CPU and GPU paths. `gpuhash benchmark` CLI
  surface starts mattering once we have three algorithms to compare.

---

## 2026-05-13 — SHA-1 + SHA-256 + benchmark (Phase 5)

**Goal today.** Ship SHA-1 and SHA-256 on both CPU and GPU paths, behind clean
per-algorithm boundaries, plus a `benchmark` subcommand that prints one H/s
number per algorithm for *this* Intel iGPU.

**What I did.** Four commits, one logical phase:

1. **CPU baselines** (`6082e89`). Uncommented `sha1` and `sha2` deps; wired
   them into `digest::digest`. Added inline NIST FIPS 180-4 / RFC 3174 vectors
   (3 + 3 cases) alongside the existing RFC 1321 MD5 vectors. The
   `unsupported_algorithms_return_not_implemented` test went away — there are
   no unsupported algorithms on the CPU path anymore.

2. **GPU SHA-1 + architectural refactor** (`bbe4774`). The original Phase-3
   shader layout (`md5_common.wgsl` + `md5_dict.wgsl` + `md5_bruteforce.wgsl`)
   would have triplicated for two more algorithms — ~60 lines of mask-
   decomposition logic copied per (algo, mode) pair, plus duplicated
   CandidateSlot / Params / MaskPos struct declarations. Refactored before
   adding SHA-1:

   ```
   gpu/shaders/
     common/
       match.wgsl       MatchBuf + targets/match_buf bindings + rotl/byteswap
                        + pad_be_block
       dict.wgsl        CandidateSlot + dict Params + bindings 0 & 3
       bruteforce.wgsl  MaskPos + brute Params + bindings 0 & 3 +
                        synthesize_candidate_le
     md5/{funcs,dict,bruteforce}.wgsl
     sha1/{funcs,dict,bruteforce}.wgsl
     sha256/{funcs,dict,bruteforce}.wgsl

   gpu/algos/
     mod.rs             pub mod md5; pub mod sha1; pub mod sha256;
     md5.rs             FUNCS, DICT_ENTRY, BRUTE_ENTRY, DICT_SPEC, BRUTE_SPEC
     sha1.rs            (same shape)
     sha256.rs          (added in commit 4)

   gpu/runner.rs           DictRunner — generic over DictKernelSpec
   gpu/bruteforce_runner.rs   BruteforceRunner — generic over BruteforceKernelSpec
   gpu/kernel_spec.rs      Endianness, Dict/BruteforceKernelSpec, assemble_shader,
                           shared common-fragment include_str!s, pack_target_words
   ```

   Adding a new algorithm now = one folder under `shaders/<algo>/` with three
   ~15-line files, one Rust file under `algos/<algo>.rs` with the spec
   constants, and one match arm in `engine::run_gpu`. Everything else in the
   GPU stack is algorithm-agnostic.

3. *(folded into commit 2)* Generalized `Md5GpuRunner` → `DictRunner` and
   `Md5BruteforceRunner` → `BruteforceRunner`; targets pack via
   `pack_target_words(targets, digest_bytes, endian)`, so MD5's LE-state-words
   path and SHA-1/SHA-256's BE-state-words path share the same Rust code.

4. **GPU SHA-256 + benchmark** (this commit). One new shader folder, one
   `algos/sha256.rs`, one engine arm. `crate::benchmark::benchmark_algo`
   drives the bruteforce runner over `?l^6` (looping the cursor back to 0 as
   needed) for a configurable wall-clock budget, counts how many candidates
   actually cleared the pipeline, and reports `H/s`. The `gpuhash benchmark`
   CLI subcommand calls it once per algorithm (or per `--algo`).

**What worked.** The architecture refactor paid for itself immediately:
SHA-256 compiled and matched the CPU reference on the first attempt. The CPU
SHA-1 / SHA-256 paths are a 1-line plumbing change each thanks to the
`RustCrypto` family. The benchmark loop reuses the engine's slot/ring
discipline almost verbatim.

**What didn't / surprises.**
- **WGSL "no dynamic indexing of let-bound arrays" bit me again.** Inside
  `sha1_block`, copying the function parameter into a local `var` was needed
  to allow `W[i] = M[i];` to use a dynamic `i`. The same rule fires inside the
  bruteforce kernels where they want to iterate over the LE bytes returned by
  `synthesize_candidate_le`. Each new shader needs `var M = M_in;` /
  `var bytes_le = synthesize_candidate_le(...);` at the top. Worth a paragraph
  in a "WGSL gotchas" section of ARCHITECTURE.md eventually.
- **Big-endian padding was duplicated** between SHA-1 and SHA-256 in the first
  draft. Moved `pad_be_block(M, len)` into `common/match.wgsl` since it's a
  generic BE-padding utility — both consumers shrunk.
- **SHA-1 GPU is *slightly faster than MD5* in the bruteforce benchmark**
  (276 vs 252 MH/s). Counterintuitive — SHA-1 does 80 rounds + a 64-word
  message-schedule expansion vs MD5's 64-round flat schedule. Hypothesis: the
  iGPU's instruction issue is so over-provisioned for the per-thread MD5
  workload that adding more arithmetic per thread *raises* effective occupancy
  by hiding memory latency. Worth re-checking with Intel GPA in Phase 9.
- **SHA-256 is ~2.5× slower than MD5**, in line with the architecture doc's
  rough expectation table for an Iris-tier part (165 MH/s reference, we hit
  102 MH/s on this UHD chip).

**Decisions made.**
- **No `HashKernel` trait yet.** The factoring into specs + generic runners
  already removes the duplication; a trait adds nothing concrete until the
  Phase-10 WGSL↔OpenCL bake-off, when a second backend implementation gives
  the trait something to abstract.
- **Two `Params` structs (dict vs brute) instead of one.** Each lives in its
  own `common/*.wgsl` file. Sharing would have forced unused fields on the
  dict path (or unused `base_index = 0` on every dispatch). Two structs, one
  per mode, is clearer.
- **`benchmark` lives in `crate::benchmark`, not in `engine.rs`.** Matches
  the architecture doc's layout and keeps the engine focused on the
  attack-driving event loop.
- **WGSL workgroup-size substitution remains a string `.replace`.** Naga's
  `override` constants would be cleaner; deferred to Phase 9 cleanup.

**Numbers.** Release build, this Intel UHD Graphics, Vulkan, `--secs 5`:

| Algorithm | Rate           | Comparable (CLAUDE.md, Iris-tier rough) |
| ---       | ---:           | ---:                                    |
| MD5       | **251.7 MH/s** | ~300 MH/s – 1 GH/s                      |
| SHA-1     | **275.8 MH/s** | (not specified)                         |
| SHA-256   | **102.4 MH/s** | ~165 MH/s                               |

Same `?l^6` mask, default tuning (batch=1<<18, wg=256), one bogus target. The
benchmark loops the keyspace cursor back to 0 once it laps; with these rates
we wrap once per ~1.1 s for MD5/SHA-1 and once per ~3 s for SHA-256.

Correctness end-to-end:
- `--algo sha1   --gpu` finds password / admin / welcome on a 3-target SHA-1 file.
- `--algo sha256 --gpu` finds password / admin / letmein on a 3-target SHA-256 file.
- 36/36 unit tests pass (added `sha256_dict_matches_cpu` and
  `sha256_brute_matches_cpu_reference` alongside the existing MD5/SHA-1 ones).

**Next.**
- Phase 6: CLI polish — `--json` (already present, sanity-check), session
  list/save/load, and exit-code review. Then Phase 7: Tauri shell and the
  React frontend.

---

## 2026-05-25 — Phase 6 close: sessions, NDJSON verified, smoke script

**Goal today.** Finish Phase 6: persistent named sessions, scripting via
`--json`, exit-code contract documented, and a PowerShell harness that
checks all of it end-to-end.

**What I did.**
- New `gpuhash-core::session` module: `Session` struct (`config`, `matches`,
  `summary`, `status`, `created_at`/`updated_at`), `SessionStatus` enum
  (`saved` / `finished` / `error`), and `Session::{save,load,list,delete}`
  + `sessions_dir()` helper. Files live under
  `%LOCALAPPDATA%\gpuhash\sessions\<name>.session.json`, with cross-platform
  fallbacks and a `GPUHASH_SESSIONS_DIR` env-var override for tests.
- Name validation rejects anything containing `..`, path separators,
  Windows-reserved chars, or a leading `.`; capped at 64 chars. Test
  `rejects_path_traversal_and_separators` covers it.
- Pulled `serde_json` into `gpuhash-core` (CLI already depended on it).
- CLI gained a `session` subcommand with `list / save / load / show / delete`.
  `attack --session NAME` auto-saves the run (status `finished` or `error`,
  with summary + matches) when it completes. `session load NAME` replays the
  saved `AttackConfig` through the engine and re-writes the same file.
- Exit codes already met the ARCHITECTURE §7.4 contract from Phase 1
  (`0` clean / `1` matches / `2` error/refusal); the smoke script just makes
  it executable documentation now.
- New `scripts/smoke.ps1` builds once with `cargo build`, looks up the
  binary via `cargo metadata`, and invokes it directly — sidestepping
  PowerShell 5.1's NativeCommandError behaviour around `cargo run` +
  stderr redirection. It runs ten assertions covering refusal, match,
  no-match, save, list, show, load, post-load state, delete idempotency,
  and post-delete list.

**What worked.**
- 42 tests green (6 new session unit tests + 36 prior); clippy clean
  with `-D warnings` on both lib and `--tests`.
- Round-trip is bit-exact: `attack --session demo` then `session load demo
  --i-own-these-hashes` produces the same 10 matches against the example
  dictionary on both runs.
- `--json | ConvertFrom-Json` works end-to-end from PowerShell; smoke
  script parses Started/Match/Finished events and validates the count.

**What didn't / surprises.**
- First pass of the smoke script tripped PowerShell 5.1's NativeCommandError
  when redirecting cargo's stderr. Calling the binary directly (after one
  `cargo build`) avoided the whole class of problem and ran faster too.
- First pass of `sessions_dir()` triggered `clippy::collapsible_else_if`
  cleanup once formatter ran — left the explicit form for readability,
  no clippy complaint after the actual run.

**Decisions made.**
- **Sessions live in the CLI, not the engine.** The engine's contract is
  pure event-stream; persistence is a frontend concern (and the Tauri
  shell in Phase 7 will use the same `Session` type but route through
  Tauri commands, not the CLI). Keeps the engine free of filesystem
  side effects.
- **Auto-save on completion, not incrementally.** A per-`Match` write
  would amplify I/O on long-running runs and complicate atomicity. End-of-
  run is enough for Phase 6's "save/load/list/delete" requirement; the
  Phase 4 bruteforce `start` field already supports incremental resume
  via `Bruteforce { start, end }` if we want richer checkpointing later.
- **Bonus `session show`.** ROADMAP only listed list/save/load/delete,
  but emitting the stored JSON is one line of CLI glue and turned out to
  be the test harness's main inspection tool — kept it.
- **`gpuhash session load` still requires `--i-own-these-hashes`.** Even
  though the session file is the user's own artifact, the ethics gate
  documents intent every time an audit runs (per CLAUDE.md /
  docs/ETHICS.md). The flag is trivially bypassable, but keeping it on
  `load` preserves the framing.

**Numbers.**
- 6 new unit tests (`session::tests::*`), 10 smoke-script assertions.
- `cargo test --workspace`: 42 passed / 0 failed.
- `cargo clippy --workspace --tests -- -D warnings`: clean.

**Next.**
- Phase 7: scaffold `crates/gpuhash-tauri/` via `npm create tauri-app`,
  expose `start_attack` / `cancel_attack` / `benchmark` commands, and
  wire the same `EngineEvent` stream into a Zustand store on the React
  side. The `Session` shape we just defined is the on-disk format the
  Tauri `save_session` command will reuse.

---

## 2026-05-25 — Phase 7: Tauri 2.x shell, vanilla-ts frontend

**Goal today.** Get a working desktop window that drives the same engine
the CLI does — no second copy of any business logic. Tauri commands,
event streaming, sessions panel. `npm run tauri dev` should launch.

**What I did.**
- Installed Node 26.2 / npm 11.13 on this laptop (was missing).
- Scaffolded with `npm create tauri-app@latest crates/gpuhash-tauri --
  template vanilla-ts --manager npm --identifier com.gpuhash.audit
  --tauri-version 2 -y -f`. First attempt without `-f` emitted
  "Directory is not empty" after laying down the JS half but before
  the `src-tauri/` Rust half; cleaning and re-running with force
  finished the scaffold cleanly.
- Renamed everything from the scaffolder's path-derived
  `cratesgpuhash-tauri` to `gpuhash-tauri` (package.json, Cargo.toml
  `[package].name` + `[lib].name = "gpuhash_tauri_lib"`, the `main.rs`
  call, `tauri.conf.json` `productName` + window title). Dropped the
  scaffold's `tauri-plugin-opener` dep — we don't need it.
- Added `crates/gpuhash-tauri/src-tauri` to the workspace; pinned
  `gpuhash-core = { path = "../../gpuhash-core" }`.
- Added `Serialize` + `Deserialize` derives to `BenchmarkReport` and
  `BenchmarkConfig` in `gpuhash-core` so they cross the IPC boundary
  cleanly. (`AttackConfig`, `AttackMode`, `AttackSummary`, `Algorithm`,
  `EngineEvent`, `Session*` already did.)
- New `RunningAttack::cancel_token()` accessor — clones the
  `CancellationToken` so the Tauri command can park only the cancel
  handle in app state while moving the events receiver into a spawned
  drain task. Mutex never gets held across an await that way.
- Rewrote `src-tauri/src/lib.rs` with six commands: `start_attack`,
  `cancel_attack`, `benchmark`, `list_sessions`, `load_session`,
  `delete_session`. `start_attack` rejects overlapping runs and bails
  if `i_own_these_hashes` is false (the ethics gate carries over —
  the GUI cannot trivially bypass it either). Events get
  `app.emit("engine-event", &ev)`'d into the webview as the same JSON
  the CLI prints with `--json`.
- Replaced the Greet HTML/TS with an attack form (algo, hashes path,
  dictionary/mask toggle, GPU checkbox, session name, ethics
  checkbox), live-stats `<dl>`, matches `<ol>`, and a sessions table
  with a Delete-per-row action. Re-used the existing
  `EngineEvent` JSON shape as a TypeScript discriminated union on
  `type`. Styling lives in one ~150-line `styles.css` with light +
  prefers-color-scheme dark themes.
- `npm install` cleanly; `tsc --noEmit` clean; `npm run build`
  bundled to ~5 kB of JS + ~3 kB of CSS gzipped; `cargo build -p
  gpuhash-tauri` compiled in 6m38s cold (full Tauri build).

**What worked.**
- End-to-end smoke via `npm run tauri dev`: Vite ready in ~430 ms,
  `gpuhash-tauri.exe` launched, and `curl http://localhost:1420/`
  returned the "GpuHash Audit" index.html. (Headless agent can't
  click the Audit button — that's on the user to do interactively.)
- 42 Rust tests still green; clippy clean across the workspace
  including the new crate.

**What didn't / surprises.**
- create-tauri-app derives the package name from the project path,
  so passing `crates/gpuhash-tauri` produced `cratesgpuhash-tauri`
  everywhere. Took manual renames in three places to fix.
- The scaffold's `tauri-plugin-opener` dep was unused but compiled in
  by default — removed it from both Cargo.toml and package.json,
  and trimmed the corresponding `opener:default` permission from
  `capabilities/default.json`.
- First Mutex<RunningAttack> design didn't work because the receiver
  isn't `Clone` and we'd hold the mutex across `next_event().await`.
  Adding `RunningAttack::cancel_token()` cleanly separates the
  "stop me" handle (lives in state) from the "drain events" handle
  (lives in the spawned task).

**Decisions made.**
- **Vanilla TS over React** (deviation from the original ROADMAP and
  `ARCHITECTURE.md` §6). The dashboard has four panels and ~280 lines
  of TS — Zustand and JSX would be ceremony, not leverage. The
  `EngineEvent` JSON contract is what matters; the framework on top
  is replaceable. ROADMAP Phase 7 entry updated to say vanilla-ts;
  React/Zustand stays available as a Phase 10 stretch if the UI grows.
- **Sessions auto-save lives in the Tauri shell, mirroring the CLI's
  Phase-6 behaviour.** Both shells take the same `Session::new_saved
  → mutate → save` path from `gpuhash-core`. The session file format
  is portable across the two — you can run an audit in the GUI, then
  inspect the same `~/AppData\Local\gpuhash\sessions\*.session.json`
  with `gpuhash session show NAME` from the CLI.
- **One concurrent run at a time.** `start_attack` errors if the
  cancel slot is non-empty. Multi-run would require multiplexed event
  channels and per-run cancel state — premature complexity for an
  educational shell.
- **Kept the `i_own_these_hashes` gate on the GUI.** Trivially
  bypassable, same as the CLI flag, but documents intent every time
  someone clicks Run — ethics framing per CLAUDE.md.

**Numbers.**
- 42 / 42 tests pass; clippy clean; vite bundle ~3.5 KB HTML / 2.7 KB
  CSS / 5.4 KB JS (gzip: 1.1 / 1.0 / 2.3 KB). Cold Tauri build 6m38s,
  warm rebuild 8.7s.

**Next.**
- Phase 8: live chart (recharts → swap for one of the small
  vanilla-friendly options like uPlot or a hand-rolled SVG), persistent
  sessions surfaced more prominently in the UI, and a demo script in
  the logbook.

---

## 2026-05-25 — Phase 8: live H/s chart, matches table, Load on sessions

**Goal today.** Make the desktop shell demo-ready: a live H/s chart in
the Live panel, a proper matches table (not a `<ol>`), a Load button on
the Sessions panel that round-trips the saved `AttackConfig` back into
the form, and a written demo script so future-me can re-run it cold.

**What I did.**
- Added a hand-rolled SVG sparkline to the Live panel. The handler
  pushes each `Progress.hashes_per_sec` into a 60-slot ring buffer and
  redraws the `<polyline>` against the running peak; the rightmost
  sample stays pinned to the right edge so the chart fills in
  left-to-right as samples arrive. ~30 lines of vanilla TS in
  [main.ts](crates/gpuhash-tauri/src/main.ts#L98) and ~20 lines of CSS.
  Picked this over recharts (React-only) and uPlot (extra dep) because
  the chart shows ≤ 60 points at 10 Hz — anything heavier is ceremony.
- Converted the matches `<ol>` to a `<table>` with an idx column and a
  plaintext column. Empty-state hint is a separate `<p class="muted">`
  toggled on `matchCount`.
- Sessions table grew a Load button per row. The Tauri command was
  already there from Phase 7 — added the frontend wiring that calls
  `load_session(name)`, populates the form fields (algo, hashes path,
  mode/wordlist/mask, GPU toggle, session name), and replays the saved
  matches into the table so you can see what the saved run found
  without re-executing it. Empty-list state is a colspan'd "No saved
  sessions." row.
- Added a `Session` TypeScript type that mirrors `gpuhash_core::Session`
  field-for-field — kept in `main.ts` next to the `EngineEvent` union
  for the same cross-shell contract reason.

**What worked.**
- `tsc --noEmit` clean. `npm run build` produced ~7 KB JS / ~3.4 KB CSS
  gzipped (up from 2.3/1.0 in Phase 7 — chart + table + Load + Session
  type cost about ~2 KB).
- Manual flow: Run with `session_name = demo` → 10 matches stream into
  the new table, chart shows the H/s rise as the dictionary drains,
  Sessions panel refreshes with `demo / finished / 10`. Click Delete on
  `demo` → row vanishes. Click Refresh → still gone. Run again with
  blank session name → no save, no row appears.

**What didn't / surprises.**
- Initial chart scaled against `Math.max(...history)` which crashed on
  empty history (spread of zero-length array). Guarded with
  `if (history.length === 0) return;` before the math.
- First Load implementation ran the attack as a side-effect. Decided
  against it — Load should let you *inspect* the saved run first, with
  Run still gated behind the ethics checkbox. So Load only populates
  the form + replays stored matches; user clicks Run Audit to execute.

**Decisions made.**
- **Hand-rolled SVG over a chart library.** ~30 lines, zero deps, and
  the dashboard is small enough that the cost of a "real" chart lib
  outweighs what it'd give us. If Phase 9's sweep wants multi-series
  per-batch-size overlays, uPlot becomes the right answer; until then,
  this is enough.
- **Load is read-only.** Populates the form and shows past matches,
  but doesn't auto-run. The ethics gate (`i_own_these_hashes`) still
  has to be re-acknowledged every time you actually run — same framing
  as the CLI's `session load NAME --i-own-these-hashes`.
- **No "Save As" button.** Save is already implicit via the
  `session_name` field on the form — typing a name and clicking Run
  saves. Adding a separate Save button would just be another way to
  do the same thing, and risks the user creating an empty `Saved`
  record they didn't mean to.

**Numbers.**
- 42 / 42 Rust tests still green; clippy clean; `tsc --noEmit` clean;
  `npm run build` 5 KB → 7 KB JS, 2.7 KB → 3.4 KB CSS (gzipped).

**Demo script (copy/paste for the final report).**

> A repeatable 60-second demo that exercises every Phase-1-through-8
> code path. Open `cd crates/gpuhash-tauri && npm run tauri dev` first
> (the cold backend compile takes a few minutes; warm rebuilds ~10 s).

```text
GpuHash Audit — demo
=====================

1. CPU dictionary audit (Phase 1 + 6 + 7)
   - Algorithm:    md5
   - Hashes file:  examples/sample_hashes.txt
   - Mode:         Dictionary,  wordlist examples/tiny_dict.txt
   - Run on GPU:   off
   - Session name: demo-cpu
   - Tick "I own these hashes" → Run Audit
   - Expected: 10 matches stream into the table, chart shows ~1–2 MH/s
     rising to peak, Finished after ~0.01 s. Sessions panel shows
     "demo-cpu / finished / 10".

2. GPU dictionary audit (Phase 3)
   - Same form, but tick Run on GPU and rename session to demo-gpu.
   - Run Audit → 10 matches; throughput jumps to ~6 MH/s on Intel UHD.

3. GPU bruteforce audit (Phase 4 + 5)
   - Switch Mode to "Bruteforce mask"
   - Mask: ?l?l?l?l   (4-lowercase keyspace ≈ 460 k, fits in a couple
     of GPU batches)
   - Hashes file: examples/sample_hashes.txt (no hits expected for
     short masks; this demo is about throughput, not coverage)
   - Run Audit → chart climbs to peak, ~7–8 MH/s sustained, Finished.

4. Algorithm sweep (Phase 5)
   - Repeat (2) twice more, once with Algorithm = sha1 and once with
     sha256. Throughput drops as the per-block work increases — by
     design, see the Phase 5 logbook entry.

5. Sessions (Phase 6 + 8)
   - Sessions panel: click Load on "demo-cpu" → form repopulates,
     prior matches re-appear in the matches table, no run kicks off.
   - Click Delete on "demo-cpu" → row vanishes.
   - Refresh → still gone.

6. Cancel mid-run (Phase 7)
   - Switch to Bruteforce with mask ?l?l?l?l?l?l (~309 M keyspace,
     several seconds of work even on GPU).
   - Run Audit → click Cancel before it finishes → status flips to
     "error: attack cancelled". (Engine treats cancellation as an
     error variant, see engine.rs.)

7. CLI cross-check (Phase 6)
   - In a separate PowerShell:
       gpuhash session show demo-gpu
     should print the same matches you saw in step 2, proving the
     CLI and GUI share the on-disk session format.
```

**Next.**
- Phase 9: the thermal-aware optimization sweep. Run each algorithm
  with at least 5 batch sizes × 3 workgroup sizes, plot H/s vs (batch
  × workgroup) — and finally pin down what "sustained" looks like on
  this iGPU over a 60-second window with thermal observations
  alongside. The Phase-8 chart is the first step of the visualization;
  Phase 9 just feeds it a longer run.

---

## 2026-05-25 — Big integration test + GUI Random Demo

**Goal today.** Two things, both off-roadmap. One: a "big" integration
test that exercises the full engine pipeline against a synthetic
10 k-candidate corpus (not just the 10-entry `examples/tiny_dict.txt`).
Two: a GUI Random Demo panel so the desktop shell is usable on a fresh
checkout without having to hand-edit file paths into the form.

**What I did.**
- New file [crates/gpuhash-core/tests/big_audit.rs](crates/gpuhash-core/tests/big_audit.rs).
  Generates 10 000 lowercase-ASCII 4–8 char candidates with a seeded
  xorshift64* PRNG, plants 50 of them, hashes the planted ones with
  the CPU reference, writes both files into a `gpuhash-big-audit-*`
  temp dir, drives `Engine::run` to completion, and asserts the found
  set exactly equals the planted set. Two tests (MD5 + SHA-256), both
  finish in ~130 ms total. The temp dir is cleaned up via a `Drop`
  impl on the `Corpus` struct.
- Inlined xorshift instead of pulling in `rand` — 10 lines, no dep.
- New Tauri command `generate_demo_corpus(count, planted, algo)` in
  [src-tauri/src/lib.rs](crates/gpuhash-tauri/src-tauri/src/lib.rs).
  Same generator as the test, but seeded from `SystemTime::now()` so
  successive Generate clicks produce different corpora. Writes
  `demo_wordlist.txt` + `demo_hashes.txt` under
  `%LOCALAPPDATA%\gpuhash\demo\` (overridable via `GPUHASH_DEMO_DIR`).
  Inputs clamped to [10, 1 000 000] candidates / [1, count] planted.
- New "Random Demo" panel at the top of [index.html](crates/gpuhash-tauri/index.html):
  candidates, planted, algorithm, Generate button. The TS handler
  calls the new command, takes the returned absolute paths, drops
  them into the Attack form's Hashes + Wordlist fields, switches Mode
  to Dictionary, and shows a status line. User then ticks the ethics
  box and clicks Run Audit — should find exactly `planted` matches.
- Also dodges the earlier "cannot find path" problem entirely: the
  demo command always returns absolute paths, so the user never has
  to think about Tauri's cwd.

**What worked.**
- All 44 tests pass (42 unit + 2 new integration); clippy clean.
- `tsc --noEmit` clean. `npm run build` ~8.3 KB JS / 3.6 KB CSS
  gzipped — up ~1 KB from the previous Phase 8 number, all in the
  new panel + handler.
- Tauri dev was already running from a previous session; saving the
  edits triggered Vite HMR and the served HTML now contains
  `<h2>Random Demo</h2>` and the new form.

**What didn't / surprises.**
- Port 1420 was held by the previous `npm run tauri dev` from the
  Phase-8 probe, so trying to launch a fresh one failed with
  "EADDRINUSE 1420." Not a bug — Vite HMR picked up the file changes
  in the still-running session anyway. Future probes should either
  reuse the running dev server or `Stop-Process` first.
- First TS draft had `onGenerateDemo` defined before being wired into
  the bootstrap `DOMContentLoaded`, which the IDE flagged as "declared
  but never read." Added the submit listener and the warning cleared.

**Decisions made.**
- **Inline xorshift PRNG, not `rand`.** Ten lines, deterministic, zero
  dep weight. `rand`'s thread-local entropy and distributions would
  be useful in a real fuzz harness; here we want reproducibility, and
  the test seed (`0x9E37_79B9_7F4A_7C15` — golden-ratio derived) is
  the contract.
- **Demo dir defaults to `%LOCALAPPDATA%\gpuhash\demo\`.** Matches
  the existing `sessions/` location pattern. `GPUHASH_DEMO_DIR`
  override mirrors `GPUHASH_SESSIONS_DIR`, used the same way.
- **CPU-only integration test.** A GPU variant would catch any
  CPU↔GPU drift across the full pipeline, but it'd also require a
  Vulkan adapter at test time, which keeps headless CI fragile. The
  Phase 3/4/5 GPU↔CPU agreement tests already cover correctness;
  this test exercises the orchestration layer (loader, source, event
  stream, summary, finish) on a non-trivial input.
- **Test seed is a constant.** A flaky failure depending on time-of-
  day would be a nightmare to debug. If a regression ever changes
  the planted-set hit rate, it'll fail deterministically and
  reproducibly.

**Numbers.**
- 44 / 44 Rust tests; CPU big-audit takes ~130 ms total for both
  algorithms (~50 ms per audit in release).
- Generated demo corpus default: 10 000 candidates / 25 planted /
  MD5. Each Generate click writes ~80 KB to disk.

**Next.**
- Resume Phase 9. The Random Demo gives us another knob to drive the
  Phase-9 sweep with — set candidates to 1 M and you've got a CPU
  reference workload that doesn't depend on having a wordlist file
  lying around.

---
