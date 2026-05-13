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
