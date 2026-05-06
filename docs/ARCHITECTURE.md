# GPU Password Auditing Framework — Architecture & Implementation Guide

## Overview

This document is the architecture & implementation reference for the GPU Password Auditing Framework: a simplified, Hashcat-style **password auditing and benchmarking** tool — *not* an offensive cracker. Educational value sits in three areas: (1) GPU compute via WGSL/wgpu, (2) Rust workspace architecture with a shared core consumed by both a CLI and a Tauri+React GUI, and (3) async pipelines that stream live progress to a UI.

**Framing.** This tool is for auditing hashes the operator owns (e.g. evaluating the strength of stored credentials in your own systems), benchmarking GPU compute throughput, and as a teaching artifact for parallel programming. Wherever a design choice has a defensive vs. offensive trade-off, this guide picks the defensive/audit framing.

**Target environment.** A single **Windows laptop with an Intel CPU/GPU** (typical: Intel UHD Graphics, Iris Xe, or Arc iGPU). **No distributed / multi-machine work** — this whole project lives on one machine.

---

## 0. TL;DR — Is This the Right Method?

**Question:** Is `Rust + wgpu + Tauri + React + clap` the correct approach for a Hashcat-style password auditor that runs on a single Windows Intel laptop?

**Short answer:** **Yes for an educational project, with three caveats.**

| Aspect | Verdict | Reasoning |
|---|---|---|
| Rust core engine | Excellent | Memory safety, modern tooling, ergonomic FFI-free workspace. |
| wgpu compute | Good (with caveat 1+2) | Cross-platform; on Windows + Intel it picks DX12 by default — well-supported drivers. *Caveat:* will not match hand-tuned OpenCL/CUDA throughput. |
| Tauri + React | Excellent | Much lighter than Electron, native Rust backend, ideal for streaming live stats. |
| clap CLI | Excellent | The standard for Rust CLIs; share the same engine crate. |
| Single Windows Intel laptop | Fine target | wgpu has mature DX12 + Vulkan backends; Intel iGPUs run WGSL compute kernels just fine. |
| Distributed / multi-machine | Out of scope | Per the constraint: one laptop only. |

**Caveat 1 (performance honesty).** A WGSL MD5/SHA implementation on an **Intel iGPU** will likely run at **5–25%** of the throughput a tuned OpenCL kernel would reach on the same chip, and *much* less than Hashcat on a discrete NVIDIA card. Realistic Intel-iGPU expectations are in §4.1.2.

**Caveat 2 (school teaches OpenCL — should you switch?).** OpenCL is the canonical Hashcat-aligned choice and matches your course material. WGSL is the modern Rust-first choice — safer, lets you reuse the rest of the Rust ecosystem (Tauri, serde, clap) without FFI. Both are defensible; §4.1.1 below gives a structured comparison so you can pick deliberately (or build a small dual-backend abstraction and demonstrate both — high educational payoff for moderate extra effort).

**Caveat 3 (scope discipline).** Stick to the roadmap in §11. A working single-machine CPU prototype → GPU prototype → CLI → GUI → optimizations is far more valuable than ten half-finished features.

**Recommendation.** Build it. The stack is sound. Stay disciplined about scope, be honest about throughput vs. Hashcat in the final report, and present the project as **password auditing + GPU compute education** — never as a cracker.

---

## 1. Overall Architecture

```
                 +----------------------------------------+
                 |              gpuhash-core              |   (library crate)
                 |  +----------------------------------+  |
                 |  |  Engine (public API)             |  |
                 |  |   |- Scheduler                   |  |
                 |  |   |- AttackRunner (trait)        |  |
                 |  |   |   |- DictionaryAttack        |  |
                 |  |   |   `- BruteforceAttack        |  |
                 |  |   |- HashLoader                  |  |
                 |  |   |- Benchmark                   |  |
                 |  |   |- Session (save/load)         |  |
                 |  |   `- GpuContext (wgpu)           |  |
                 |  +----------------------------------+  |
                 |  Async API:  tokio mpsc channels       |
                 |  Events:    EngineEvent { Progress... }|
                 +--------+----------------------+--------+
                          |                      |
        +-----------------+                      +----------------+
        v                                                          v
+-------------------+                              +--------------------------+
|  gpuhash-cli      |                              |     gpuhash-tauri        |
|  - clap parser    |                              |  Rust side:              |
|  - writes stdout  |                              |   - #[tauri::command]    |
|  - exit codes     |                              |   - event::emit_all      |
+-------------------+                              |  Frontend (React+TS):    |
                                                   |   - invoke / listen      |
                                                   |   - Dashboard, Charts    |
                                                   +--------------------------+
```

**Key idea — single source of truth.** Both the CLI and the Tauri app are *thin* shells around `gpuhash-core`. They never duplicate logic; they translate user intent into engine API calls and translate `EngineEvent`s back into terminal output or UI updates respectively.

**Communication boundaries.**
- **Within `gpuhash-core`:** synchronous Rust function calls; long-running work runs on `tokio` tasks and emits `EngineEvent`s via an `mpsc` channel.
- **CLI ↔ core:** the CLI binary owns a `tokio` runtime, calls `engine.run(...)`, and prints from the event stream.
- **Tauri ↔ core:** Tauri commands are `async fn`s; they hold an `Arc<EngineHandle>` in `tauri::State`. Events flow back to the frontend via `app_handle.emit_all("engine_event", payload)`.
- **Frontend ↔ Tauri:** `invoke("start_attack", { config })` (commands) and `listen("engine_event", cb)` (events).

**GPU compute pipeline (zoomed in).**
```
CPU candidate generator (rayon)  -+
                                  |  write_buffer
                                  v
                            +--------------+
                            | GPU staging  |
                            +------+-------+
                                   |  copy
                                   v
   target hashes (uniform) --> +------------------+
                               |  Compute Shader  |  hash(candidate) == target?
                               |  (WGSL)          |  if yes: atomicAdd(&found_count, 1)
                               +--------+---------+
                                        |
                            +-----------v-----------+
                            | output buffer + map   |
                            +-----------+-----------+
                                        |  poll & map_async
                                        v
                              EngineEvent::Match { idx, plaintext }
```

---

## 2. Folder / Project Structure

```
GpuHash/
├── Cargo.toml                    # workspace root
├── README.md
├── LICENSE                       # MIT or Apache-2.0
├── docs/
│   ├── ARCHITECTURE.md
│   ├── ROADMAP.md
│   ├── ETHICS.md
│   └── LOGBOOK.md
├── crates/
│   ├── gpuhash-core/
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs            # re-exports public API
│   │   │   ├── engine.rs         # top-level Engine struct
│   │   │   ├── error.rs          # thiserror enums
│   │   │   ├── event.rs          # EngineEvent variants
│   │   │   ├── scheduler.rs      # batch sizing & dispatch loop
│   │   │   ├── benchmark.rs
│   │   │   ├── session.rs        # serde save/load
│   │   │   ├── attacks/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── dictionary.rs
│   │   │   │   └── bruteforce.rs
│   │   │   ├── hash/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── md5.rs
│   │   │   │   ├── sha1.rs
│   │   │   │   └── sha256.rs
│   │   │   └── gpu/
│   │   │       ├── mod.rs
│   │   │       ├── context.rs
│   │   │       ├── pipeline.rs
│   │   │       ├── buffers.rs
│   │   │       └── shaders/
│   │   │           ├── md5.wgsl
│   │   │           ├── sha1.wgsl
│   │   │           └── sha256.wgsl
│   │   └── tests/
│   │       ├── known_vectors.rs
│   │       └── round_trip.rs
│   ├── gpuhash-cli/
│   │   ├── Cargo.toml
│   │   └── src/main.rs
│   └── gpuhash-tauri/            # added in Phase 7
│       ├── tauri.conf.json
│       ├── src/                  # Rust backend
│       └── ui/                   # React + TS frontend
└── examples/
    ├── sample_hashes.txt
    └── tiny_dict.txt
```

**Workspace `Cargo.toml`:**
```toml
[workspace]
resolver = "2"
members = ["crates/gpuhash-core", "crates/gpuhash-cli"]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT OR Apache-2.0"

[workspace.dependencies]
tokio   = { version = "1", features = ["rt-multi-thread", "sync", "macros", "time"] }
serde   = { version = "1", features = ["derive"] }
wgpu    = "22"
bytemuck = { version = "1", features = ["derive"] }
clap    = { version = "4", features = ["derive"] }
anyhow  = "1"
thiserror = "1"
tracing = "0.1"
rayon   = "1"
```

---

## 3. Rust Engine Design

### 3.1 Public surface (`lib.rs`)
```rust
pub mod engine;
pub mod attacks;
pub mod hash;
pub mod gpu;
pub mod scheduler;
pub mod benchmark;
pub mod session;
pub mod event;
pub mod error;

pub use engine::{Engine, EngineHandle, AttackConfig};
pub use event::EngineEvent;
pub use error::{Error, Result};
```

### 3.2 `Engine` (the orchestrator)
```rust
pub struct Engine {
    gpu: Arc<GpuContext>,
    tx_event: mpsc::UnboundedSender<EngineEvent>,
    cancel: CancellationToken,
}

impl Engine {
    pub async fn new() -> Result<(Self, EventStream)> { /* ... */ }
    pub async fn run_attack(&self, cfg: AttackConfig) -> Result<AttackSummary> { /* ... */ }
    pub async fn benchmark(&self, algo: Algorithm) -> Result<BenchmarkReport> { /* ... */ }
    pub fn cancel(&self) { self.cancel.cancel(); }
}
```

### 3.3 Attack scheduler (`scheduler.rs`)
The scheduler owns the *dispatch loop*. Its job: keep the GPU saturated by overlapping CPU candidate generation with GPU compute.

```rust
pub struct Scheduler {
    batch_size: u32,           // candidates per dispatch (e.g. 1<<20 — smaller on iGPU)
    max_in_flight: u32,        // 2 or 3 (double/triple buffered)
}

impl Scheduler {
    pub async fn drive<S: CandidateSource>(
        &self,
        mut source: S,
        ctx: &GpuContext,
        pipeline: &HashPipeline,
        targets: &TargetSet,
        events: &mpsc::UnboundedSender<EngineEvent>,
        cancel: &CancellationToken,
    ) -> Result<AttackSummary> {
        // 1. fill ring of staging buffers
        // 2. submit compute work; do NOT await each submission
        // 3. when oldest fence resolves, copy results out, emit events
        // 4. refill that buffer slot with next batch
        // 5. exit on cancel or source exhaustion
    }
}
```

### 3.4 Dictionary attack (`attacks/dictionary.rs`)
```rust
pub struct DictionaryAttack {
    wordlist: PathBuf,
    rules: Option<Vec<Rule>>,   // bonus feature; start with None
}

impl AttackRunner for DictionaryAttack {
    fn candidates(&self) -> Box<dyn CandidateSource> {
        Box::new(WordlistSource::open(&self.wordlist).unwrap())
    }
}

struct WordlistSource { reader: BufReader<File> }
impl CandidateSource for WordlistSource {
    fn fill_batch(&mut self, buf: &mut CandidateBatch) -> Result<usize> {
        // read up to `buf.capacity()` lines, pack into fixed-stride buffer
    }
}
```

### 3.5 Brute-force attack (`attacks/bruteforce.rs`)
Mask-driven (e.g. `?l?l?l?l?d?d` = 4 lowercase + 2 digits). The candidate index is just an integer; the GPU shader can derive the candidate string from the index *on-device*, eliminating the CPU bottleneck.

```rust
pub struct BruteforceAttack {
    mask: Mask,           // parsed charset positions
    start: u64,           // for resume
    end: Option<u64>,
}
```

### 3.6 Hash loading (`hash/mod.rs`)
```rust
pub enum Algorithm { Md5, Sha1, Sha256 }

pub struct TargetSet {
    pub algo: Algorithm,
    pub hashes: Vec<[u8; 32]>,    // pad shorter digests with zeros
    pub gpu_buffer: wgpu::Buffer, // uploaded once, reused
}

pub fn load_targets(path: &Path, algo: Algorithm) -> Result<TargetSet> {
    // parse one-hex-digest-per-line; reject malformed
}
```

### 3.7 Benchmarking (`benchmark.rs`)
Synthetic load: generate N random candidates, compute `hash(candidate)`, time wall-clock. Report **H/s**, **GPU info** (`wgpu::Adapter::get_info`), and **batch latency**.

```rust
pub struct BenchmarkReport {
    pub algo: Algorithm,
    pub hashes_per_sec: f64,
    pub batch_size: u32,
    pub batch_latency_ms: f64,
    pub adapter: String,
}
```

### 3.8 Result handling (`event.rs`)
```rust
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type")]
pub enum EngineEvent {
    Started { algo: Algorithm, total: Option<u64> },
    Progress { tested: u64, hashes_per_sec: f64, eta_secs: Option<f64> },
    Match { plaintext: String, target_idx: u32 },
    Finished { summary: AttackSummary },
    Error { message: String },
}
```
Tagged enum serialization makes this both Rust-friendly and JS-friendly: in TypeScript it appears as a discriminated union on `type`.

### 3.9 Error handling
- **Library errors:** `thiserror` enum in `error.rs` (`Io`, `Wgpu`, `BadFormat`, `Cancelled`, …).
- **Application binaries** (CLI / Tauri shell): `anyhow::Result` is fine for wiring.

```rust
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("io error: {0}")] Io(#[from] std::io::Error),
    #[error("wgpu error: {0}")] Wgpu(String),
    #[error("invalid hash file: {0}")] BadFormat(String),
    #[error("cancelled")] Cancelled,
}
```

---

## 4. GPU Acceleration

### 4.1 Why wgpu?
- **Cross-vendor + cross-OS.** One Rust codebase runs on Vulkan, Metal, DX12, and WebGPU. Hashcat needs separate OpenCL/CUDA paths; wgpu collapses that to one.
- **Pure Rust + safe API.** No `unsafe` FFI to debug at 2am.
- **Modern shading language (WGSL).** Stricter type system than GLSL/HLSL.
- **Production-grade.** Used by Firefox, Bevy, Deno's GPU support.
- **Educational fit.** Lower abstraction than Vulkan, but still real GPU concepts (bind groups, queues, fences).
- **On Windows + Intel iGPU specifically:** wgpu defaults to **DX12**, which Intel ships first-class drivers for. Falls back to Vulkan if asked. No CUDA needed (good — Intel iGPUs have none).

### 4.1.1 WGSL/wgpu vs OpenCL — the choice your course is implicitly asking you to defend

Your school teaches **OpenCL**. This plan uses **WGSL via wgpu**. Both are valid. The honest side-by-side:

| Dimension | OpenCL (school) | WGSL via wgpu (this plan) |
|---|---|---|
| Language | C99 dialect with vendor extensions | Rust-flavored shading language |
| Host API | C / C++ (or `opencl3` from Rust) | Pure Rust (`wgpu`) |
| Maturity for compute | Industry standard ~15 years; Hashcat is OpenCL | Compute path stabilized 2023–2024 |
| Vendor support on Windows + Intel | Excellent — Intel ships its own runtime | Excellent — DX12 backend is Microsoft+Intel maintained |
| Subgroup / warp ops | Mature (`get_sub_group_id`, shuffle) | Newer (gated behind features in wgpu 22+) |
| Memory model | Explicit, more knobs | Simpler, fewer footguns |
| Throughput ceiling | Higher (closer to vendor metal) | Lower (~10–30% off due to safer abstractions) |
| FFI / build system | C interop, vendor SDKs | None — `cargo build` and done |
| Web reuse | None | Same WGSL runs in a browser via WebGPU |
| Course alignment | Direct | Translation needed |
| Industry alignment with Hashcat | Same toolchain | Different toolchain |

**Pick OpenCL (`opencl3` crate) when:** your grading weights "applies course material"; you want lines transferable to Hashcat-style work later.

**Pick WGSL/wgpu when:** you value an entirely Rust codebase; you want the GUI/CLI/engine plumbing to be the focus and the GPU layer to be "just another module."

**Third option — small dual-backend abstraction.** Define a Rust trait `HashKernel { fn dispatch(...); }` with two implementations: `WgpuKernel` (WGSL) and `OpenClKernel` (`opencl3`). Have the CLI accept `--backend wgpu|opencl`. About a week of extra work, and you get a *killer* demo: a head-to-head benchmark of WGSL vs OpenCL on the same Intel iGPU. That graph alone is publication-quality.

**Recommendation:** start with WGSL (this plan ships you working code fastest), then in Phase 8/10 add an OpenCL backend. You finish on time *and* hit the course material.

### 4.1.2 What to expect on a Windows Intel laptop

Realistic ballparks for **MD5** on Intel integrated graphics, single laptop:

| Hardware tier | Expected wgpu/WGSL MD5 | Tuned OpenCL | Hashcat on RTX 4070 (reference) |
|---|---|---|---|
| Intel UHD 620 (older U-series) | ~50–150 MH/s | ~150–400 MH/s | ~25 GH/s |
| Iris Xe (Tiger / Alder Lake) | ~300 MH/s – 1 GH/s | ~1–3 GH/s | ~25 GH/s |
| Arc A370M / A770M (laptop discrete) | ~2–6 GH/s | ~5–15 GH/s | ~25 GH/s |

Order-of-magnitude only — actual numbers vary with thermals, drivers, batch size. The *point* of recording these in your logbook is to show that you measured your real platform.

Tuning notes for an Intel iGPU:
- **Smaller batches.** Start at `batch_size = 1<<16` (65k), not `1<<20`. iGPUs share memory with the CPU; oversize batches stall the system.
- **Workgroup size 32 or 64.** Don't go to 256 on Intel.
- **Watch thermals.** A laptop iGPU thermal-throttles within ~30 seconds. Run benchmarks for ≥ 60 s and report sustained, not peak, H/s.
- **Plug in the laptop.** Battery power often clocks the iGPU down significantly.

### 4.2 Compute shader workflow

```
                 device.create_shader_module(WGSL source)
                                     |
                                     v
                       device.create_compute_pipeline
                                     |
                                     v
              device.create_bind_group_layout / bind_group
                                     |
                                     v
   encoder = device.create_command_encoder()
   {
       pass = encoder.begin_compute_pass();
       pass.set_pipeline(&pipeline);
       pass.set_bind_group(0, &bind_group, &[]);
       pass.dispatch_workgroups(num_groups_x, 1, 1);
   }
   queue.submit(once(encoder.finish()));
   queue.on_submitted_work_done(callback)   // or buffer.map_async
```

### 4.3 WGSL basics — minimal MD5 dispatcher

```wgsl
// crates/gpuhash-core/src/gpu/shaders/md5.wgsl
// (Sketch — real MD5 needs the 64-round constants/shift table.)

struct Candidate {
    len: u32,
    data: array<u32, 16>,    // up to 64 bytes packed little-endian
};

struct Match {
    candidate_idx: u32,
    target_idx: u32,
};

@group(0) @binding(0) var<storage, read>        candidates : array<Candidate>;
@group(0) @binding(1) var<storage, read>        targets    : array<vec4<u32>>; // 16-byte digests
@group(0) @binding(2) var<storage, read_write>  out_count  : atomic<u32>;
@group(0) @binding(3) var<storage, read_write>  out_matches: array<Match>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid : vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&candidates)) { return; }

    let digest = md5(candidates[i]);    // helper fn defined in same file

    let n_targets = arrayLength(&targets);
    for (var t = 0u; t < n_targets; t = t + 1u) {
        if (all(digest == targets[t])) {
            let slot = atomicAdd(&out_count, 1u);
            out_matches[slot] = Match(i, t);
        }
    }
}
```

**Key concepts:** `@group(0) @binding(N)` (binding numbers must match the bind-group layout on the Rust side), `var<storage, read>` vs `read_write`, `atomic<u32>` + `atomicAdd` for lock-free coordination across thousands of invocations, `@compute @workgroup_size(64)` (tune 32/64/128 — see §4.1.2).

### 4.4 Workgroups & dispatch
A workgroup is a *team of invocations* that share local memory and can synchronize. For independent hash computations there's no inter-invocation chatter, so a flat grid is fine:

```rust
let batch_size: u32 = 1 << 16;          // Intel iGPU sweet spot; 1 << 20 on discrete
let workgroup_size: u32 = 64;
let num_workgroups = (batch_size + workgroup_size - 1) / workgroup_size;
pass.dispatch_workgroups(num_workgroups, 1, 1);
```

### 4.5 Memory buffers
| Buffer | Usage flags | Size |
|---|---|---|
| `candidates` | `STORAGE \| COPY_DST` | `batch_size * sizeof(Candidate)` |
| `targets`    | `STORAGE \| COPY_DST` | `n_targets * 32 bytes` |
| `out_count`  | `STORAGE \| COPY_SRC` | `4 bytes` |
| `out_matches`| `STORAGE \| COPY_SRC` | `cap_matches * sizeof(Match)` |
| `staging_read` | `MAP_READ \| COPY_DST` | mirrors out_matches |

Always **map** through a staging buffer; never map a STORAGE buffer directly.

### 4.6 CPU vs GPU workload distribution

| Task | Where | Why |
|---|---|---|
| Read wordlist from disk | CPU (rayon) | I/O bound |
| Generate brute-force candidates | **GPU** (in shader from `gid.x`) | Avoids CPU↔GPU bandwidth wall |
| Apply mutation rules | CPU first, GPU later | Easier to validate on CPU |
| Compute hash + compare | GPU | The whole point |
| Write match results | GPU → small staging → CPU | Tiny payload, infrequent |
| Aggregate stats | CPU | Trivial cost |

### 4.7 Optimization considerations
1. **Batch size sweep.** Plot H/s vs batch size; pick the elbow.
2. **Pipeline reuse.** Compile each compute pipeline *once* per algorithm.
3. **No per-batch buffer reallocation.** Allocate once, `queue.write_buffer` to overwrite.
4. **Don't `await` the GPU after every dispatch.** Use a ring of N=2 or 3 batch slots.
5. **Move brute-force candidate generation onto the GPU.** Saves >50% bandwidth.
6. **Avoid `MAP_READ` on hot path buffers.** Only map small results buffer.
7. **Profile.** RenderDoc or Intel GPA to confirm actual occupancy.

---

## 5. Tauri Integration

### 5.1 Communication primitives
- **Commands** — `#[tauri::command] async fn ...`. Frontend calls `invoke("name", args)`. One-shot request/response.
- **Events** — `app.emit_all("topic", payload)`. Frontend calls `listen("topic", cb)`. Many-to-many push.

Use **commands** to *start* / *cancel* / *configure*. Use **events** to *stream live progress*.

### 5.2 Command handlers
```rust
use gpuhash_core::{AttackConfig, EngineHandle, EngineEvent};
use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;

pub struct AppState {
    pub engine: Mutex<Option<EngineHandle>>,
}

#[tauri::command]
pub async fn start_attack(
    cfg: AttackConfig,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    let mut guard = state.engine.lock().await;
    let handle = guard.get_or_insert_with(EngineHandle::spawn).clone();

    tokio::spawn(async move {
        let mut events = handle.run_attack(cfg).await;
        while let Some(ev) = events.recv().await {
            let _ = app.emit_all("engine_event", &ev);
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn cancel_attack(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(h) = state.engine.lock().await.as_ref() { h.cancel(); }
    Ok(())
}
```

### 5.3 Wiring in `main.rs`
```rust
fn main() {
    tracing_subscriber::fmt::init();

    tauri::Builder::default()
        .manage(AppState { engine: Default::default() })
        .invoke_handler(tauri::generate_handler![
            commands::start_attack,
            commands::cancel_attack,
            commands::benchmark,
        ])
        .run(tauri::generate_context!())
        .expect("error launching Tauri app");
}
```

### 5.4 Async architecture
- The Rust backend runs on Tauri's built-in tokio runtime.
- Long jobs **must not** block the command's `async fn`. Spawn with `tokio::spawn` and return immediately.
- The engine emits events through an `mpsc` channel; a forwarder task pushes to `app.emit_all`.
- Cancellation uses a `CancellationToken` (from `tokio_util`) checked at scheduler boundaries.

---

## 6. React Frontend

### 6.1 UI structure
```
Dashboard
├── Header (status pill: idle/running, GPU adapter name, cancel button)
├── AttackPanel
│   ├── Algorithm dropdown (md5/sha1/sha256)
│   ├── Mode tabs: [Dictionary] [Brute-force] [Benchmark]
│   ├── HashFile picker
│   ├── Wordlist picker / Mask input
│   └── [Audit] button → invoke("start_attack", cfg)
├── LiveStats
│   ├── current H/s (big number)
│   ├── tested / total + progress bar
│   ├── ETA
│   └── matches found counter
├── HashRateChart  (recharts line chart, last 60 samples)
├── MatchesTable   (audited plaintexts as they stream in)
└── SessionList
```

### 6.2 Wrapping Tauri APIs
```ts
// ui/api/tauri.ts
import { invoke } from "@tauri-apps/api/tauri";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

export type EngineEvent =
  | { type: "Started"; algo: string; total?: number }
  | { type: "Progress"; tested: number; hashes_per_sec: number; eta_secs?: number }
  | { type: "Match"; plaintext: string; target_idx: number }
  | { type: "Finished"; summary: AttackSummary }
  | { type: "Error"; message: string };

export const startAttack = (cfg: AttackConfig) => invoke<void>("start_attack", { cfg });
export const cancelAttack = () => invoke<void>("cancel_attack");
export const benchmark = (algo: string) => invoke<string>("benchmark", { algo });
export const onEngineEvent = (cb: (e: EngineEvent) => void): Promise<UnlistenFn> =>
    listen<EngineEvent>("engine_event", (e) => cb(e.payload));
```

### 6.3 Live state with Zustand
```ts
import { create } from "zustand";
import { EngineEvent } from "../api/tauri";

interface State {
  status: "idle" | "running" | "done" | "error";
  hashRate: number;
  history: { t: number; rate: number }[];
  tested: number;
  total?: number;
  matches: { plaintext: string; idx: number }[];
  push: (e: EngineEvent) => void;
}

export const useEngine = create<State>((set) => ({
  status: "idle", hashRate: 0, history: [], tested: 0, matches: [],
  push: (e) => set((s) => {
    switch (e.type) {
      case "Started":  return { ...s, status: "running", total: e.total };
      case "Progress": return {
        ...s, tested: e.tested, hashRate: e.hashes_per_sec,
        history: [...s.history.slice(-59), { t: Date.now(), rate: e.hashes_per_sec }],
      };
      case "Match":    return { ...s, matches: [...s.matches, { plaintext: e.plaintext, idx: e.target_idx }] };
      case "Finished": return { ...s, status: "done" };
      case "Error":    return { ...s, status: "error" };
    }
  }),
}));
```

### 6.4 Charts
`recharts` is the easiest fit (declarative, great defaults). One `<LineChart>` driven by `history`. Sample at most ~10/s on the Rust side and throttle further on the React side if needed.

### 6.5 Session management
- Save: collect `cfg + summary + matches` and `invoke("save_session", { name })`.
- Load: list `~/AppData/Local/gpuhash/sessions/*.json` (Windows path), render in `SessionList`.

---

## 7. CLI Mode

### 7.1 Same engine, no duplication
`gpuhash-cli/Cargo.toml`:
```toml
[dependencies]
gpuhash-core = { path = "../gpuhash-core" }
clap         = { workspace = true }
tokio        = { workspace = true }
anyhow       = { workspace = true }
tracing-subscriber = "0.3"
```

### 7.2 clap setup
```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "gpuhash", about = "Educational GPU password auditor")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run an audit against your own hash file.
    Attack {
        #[arg(long)] algo: String,         // md5 | sha1 | sha256
        #[arg(long)] hashes: PathBuf,
        #[arg(long, group = "mode")] wordlist: Option<PathBuf>,
        #[arg(long, group = "mode")] mask: Option<String>,
        #[arg(long)] session: Option<String>,
        /// Required acknowledgement that the hashes belong to you.
        #[arg(long)] i_own_these_hashes: bool,
    },
    Benchmark {
        #[arg(long)] algo: Option<String>,
        #[arg(long, default_value_t = 5)] secs: u64,
    },
    Session {
        #[command(subcommand)] action: SessionCmd,
    },
}
```

### 7.3 Benchmarking commands
```
PS> cargo run -p gpuhash-cli -- benchmark --algo md5
md5    : 920 MH/s  (batch=65536, latency=0.07 ms, adapter="Intel(R) Iris(R) Xe Graphics")

PS> cargo run -p gpuhash-cli -- benchmark
md5    : 920 MH/s
sha1   : 410 MH/s
sha256 : 165 MH/s
```

### 7.4 Scripting support
- `--json` produces NDJSON of `EngineEvent`s (pipe into `jq` or PowerShell `ConvertFrom-Json`).
- Exit codes: `0` = ran cleanly, `1` = found at least one match (audit "failed open"), `2` = error.
- `--quiet` suppresses progress, only emits final summary — useful for CI.

---

## 8. Recommended Crates and Libraries

| Crate | Why |
|---|---|
| **wgpu** | The compute backbone. Cross-platform safe Rust API over Vulkan/Metal/DX12/WebGPU. Pin the major version because the API evolves quickly. |
| **clap** | Standard Rust CLI parser. The derive macro builds a typed `Cli` struct, handles `--help`, validation, and subcommands automatically. |
| **serde** | (De)serialization. Annotate `EngineEvent`, `AttackConfig`, `Session` with `#[derive(Serialize, Deserialize)]`. |
| **tokio** | Async runtime for non-blocking I/O, channels, timers, `tokio::spawn`. Tauri itself uses tokio under the hood. |
| **rayon** | Data-parallel CPU work (parsing wordlists, applying mutation rules). Drop-in: `iter()` → `par_iter()`. |
| **tauri** | The desktop shell. Bundles the Rust backend with a webview hosting React, plus the command/event IPC glue. Far smaller than Electron. |
| **tracing** | Structured logging. Replace `println!` with `tracing::info!("dispatched batch", batch_id, n)`. |
| **anyhow** | Ergonomic error type for *binaries* (CLI / Tauri shell). Don't use it inside the library crate. |
| **thiserror** | Ergonomic *typed* errors for *libraries* (`gpuhash-core`). |

Supporting picks worth knowing:
- **bytemuck** — safe `Pod`/`Zeroable` for sending `#[repr(C)]` structs to the GPU.
- **tokio-util** — `CancellationToken` for clean cancellation.
- **md-5 / sha1 / sha2** — RustCrypto reference impls for the CPU prototype + test vectors.

---

## 9. Example End-to-End Data Flow

User clicks **Audit** for an MD5 dictionary attack:

```
[1] React: <button onClick={() => startAttack(cfg)} />

[2] @tauri-apps/api invoke("start_attack", { cfg })  ──► Tauri IPC

[3] commands::start_attack(cfg, state, app)
        │
        │ tokio::spawn(async move {
        │     let mut events = engine.run_attack(cfg).await;
        │     while let Some(ev) = events.recv().await {
        │         app.emit_all("engine_event", &ev);
        │     }
        │ });
        ▼
[4] Engine::run_attack
        ├─ load TargetSet (parse hash file)
        ├─ open Wordlist
        └─ scheduler.drive(...)

[5] Scheduler loop (per batch):
        ├─ rayon: pack next 64K candidates into staging buffer
        ├─ queue.write_buffer(candidates_buf, …)
        ├─ encoder.dispatch_workgroups(num_groups, 1, 1)
        ├─ queue.submit
        ├─ device.poll(Maintain::Poll)
        ├─ map out_count + out_matches asynchronously
        └─ for each match: tx_event.send(EngineEvent::Match { … })
                            tx_event.send(EngineEvent::Progress { … })

[6] Forwarder task pushes each EngineEvent onto the "engine_event" topic.

[7] React: useEffect(() => { onEngineEvent(useEngine.getState().push); }, [])

[8] Zustand store updates → components re-render:
        ├─ LiveStats shows new H/s
        ├─ HashRateChart appends sample
        └─ MatchesTable lists newly-audited plaintexts
```

This pipeline is **back-pressure-free** because the GPU drives the cadence — the UI just observes.

---

## 10. Performance Considerations

1. **Batching.** A `dispatch_workgroups` call has tens-of-microseconds of overhead. At MD5 rates of MH/s on iGPU that's still nothing if the batch is 64K+. At <1K candidates per batch you'd be CPU-bound on launches.
2. **Memory transfers.** Treat PCIe / shared-memory copies as a bottleneck. Don't copy the wordlist back from GPU. Don't read the candidates back. Only the small results buffer travels device→host.
3. **Minimizing GPU stalls.** Submit batch N+1 *before* you map batch N's results. The classic shape is a ring of `(staging, command_buffer, fence)` slots.
4. **Threading.** One `tokio` task drives the dispatch loop. `rayon` parallelizes CPU candidate generation. Never await GPU map calls on the UI thread.
5. **Async pipelines.** Use `mpsc` channels to decouple stages (file reader → mutator → packer → GPU dispatcher → result reader).
6. **Avoid surprise reallocations.** Pre-size `Vec`s used per-batch. Reuse `wgpu::Buffer`s.
7. **Power and thermals.** A laptop iGPU thermal-throttles within ~30 seconds. Document hardware + thermal envelope alongside results.

---

## 11. Roadmap

See [ROADMAP.md](ROADMAP.md) for the phased plan and checkbox tracker.

---

## 12. Ethical / Security Considerations

See [ETHICS.md](ETHICS.md) for the full defensive framing.

---

## 13. Bonus Features (Pick 1–2)

All features are **single-machine** — distributed cracking is out of scope.

| Feature | Difficulty | Payoff |
|---|---|---|
| **Live GPU metrics** | Low–medium | `wgpu::Adapter::get_info` + `wgpu::QuerySet` timestamps. On Windows + Intel, also poll Performance Counters via the `windows` crate to plot iGPU utilization alongside H/s. |
| **WGSL ↔ OpenCL backend bake-off** | Medium–high | Same MD5 kernel in both WGSL (wgpu) and OpenCL (`opencl3`) behind a `HashKernel` trait; benchmark on the same Intel iGPU. Bridges school OpenCL material with the Rust stack. **Highest value for the course report.** |
| **Hybrid CPU/GPU scheduler** | Medium | rayon CPU hashers alongside GPU batches; weight by measured per-device throughput. With shared CPU↔iGPU memory the trade-off is genuinely interesting. |
| **Save/load sessions** | Low | Serde JSON, resume from `tested` index. |
| **Rule-based mutations** | Medium–high | Small DSL of hashcat-style rules (append/prepend/capitalize/leetspeak). Keep examples focused on common-password classes so framing stays auditing-oriented. |
| **Adaptive batch sizer** | Low–medium | Grow batch until per-batch latency exceeds 50 ms, shrink if frames drop. Self-tunes to whatever Intel iGPU is present. |

---

## 14. Logbook

Maintained at [LOGBOOK.md](LOGBOOK.md). Append-only, dated entries.

---

## 15. Critical Files

- [Cargo.toml](../Cargo.toml) — workspace definition.
- [crates/gpuhash-core/src/lib.rs](../crates/gpuhash-core/src/lib.rs) — the public API surface.
- `crates/gpuhash-core/src/engine.rs` *(Phase 1+)* — the orchestrator.
- `crates/gpuhash-core/src/scheduler.rs` *(Phase 4)* — the dispatch loop; this file determines throughput.
- [crates/gpuhash-core/src/event.rs](../crates/gpuhash-core/src/event.rs) — the contract between engine, CLI, and React.
- `crates/gpuhash-core/src/gpu/pipeline.rs` *(Phase 3)* — bind groups & compute pipeline construction.
- `crates/gpuhash-core/src/gpu/shaders/md5.wgsl` *(Phase 3)* — start here.
- [crates/gpuhash-cli/src/main.rs](../crates/gpuhash-cli/src/main.rs) — clap surface.
- `crates/gpuhash-tauri/src/commands.rs` *(Phase 7)* — IPC boundary.
- `crates/gpuhash-tauri/ui/api/tauri.ts` *(Phase 7)* — typed wrapper that mirrors `EngineEvent`.

---

## 16. Verification Strategy

After each phase, verify end-to-end:

**Phase 1 (CPU prototype).**
- `cargo test -p gpuhash-core` passes RFC 1321 test vectors for MD5.
- `cargo run -p gpuhash-cli -- attack --algo md5 --hashes examples/sample_hashes.txt --wordlist examples/tiny_dict.txt --i-own-these-hashes` prints expected matches.

**Phase 3 (GPU dispatch).**
- The same command with `--gpu` produces the same matches as `--cpu` (bit-exact).
- `cargo run -p gpuhash-cli -- benchmark --algo md5` reports a number > 0 H/s.

**Phase 5 (multi-algo).** All three algorithms pass NIST / RFC test vectors on both CPU and GPU.

**Phase 7 (Tauri).** `npm run tauri dev` opens the app; clicking **Audit** runs the same workload as the CLI; the chart updates live. Cancel mid-run leaves no orphaned tokio task.

**Phase 9 (optimization).** A graph in the report shows H/s vs (batch size × workgroup size) for at least one algorithm, on *your* Intel iGPU.

**Smoke check before any demo.**
1. `cargo fmt --check && cargo clippy -- -D warnings` clean.
2. `cargo test --workspace` green.
3. `cargo run -p gpuhash-cli -- benchmark` finishes.
4. `cargo tauri dev` opens, runs an audit, shows a chart.

---

## 17. Best Practices Recap

- **One source of truth for types.** Define `EngineEvent` and `AttackConfig` in `gpuhash-core`, derive `Serialize/Deserialize`, and let TypeScript mirror them via hand-written types or `ts-rs`.
- **Library crate is `no-anyhow`.** Use `thiserror` + typed errors there; reserve `anyhow` for binaries.
- **Don't recompile pipelines.** Build all `wgpu::ComputePipeline`s at startup.
- **Lean on `bytemuck` for GPU-bound structs.** `#[repr(C)] #[derive(Pod, Zeroable)]` lets you `cast_slice` directly into `write_buffer`.
- **Throttle UI updates.** Engine emits at most 10 `Progress` events per second.
- **Tracing > println!.** From day one.
- **Test vectors first, performance later.** A fast wrong hash is worse than a slow correct one.
- **Defensive framing, always.** UI strings, README, demo script — every artifact reinforces "auditing/benchmarking", not "cracking."

---

## Appendix A — Smallest possible thing that works (Phase 2 starter)

This compiles; it dispatches a no-op WGSL kernel and reads back a `1`. Use this as your first wgpu commit — *before* even attempting MD5 — to make sure the plumbing is correct.

```rust
// crates/gpuhash-core/src/gpu/smoke.rs
use wgpu::util::DeviceExt;

pub async fn smoke() -> anyhow::Result<u32> {
    let instance = wgpu::Instance::default();
    let adapter  = instance.request_adapter(&Default::default()).await.unwrap();
    let (device, queue) = adapter.request_device(&Default::default(), None).await?;

    const SHADER: &str = r#"
        @group(0) @binding(0) var<storage, read_write> data : array<u32>;
        @compute @workgroup_size(1) fn main() { data[0] = 1u; }
    "#;

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("smoke"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });

    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("data"), contents: bytemuck::cast_slice(&[0u32]),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("staging"), size: 4,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None, layout: None, module: &module, entry_point: "main",
        compilation_options: Default::default(), cache: None,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None, layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
    });

    let mut enc = device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    enc.copy_buffer_to_buffer(&buffer, 0, &staging, 0, 4);
    queue.submit(Some(enc.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = futures::channel::oneshot::channel();
    slice.map_async(wgpu::MapMode::Read, |r| { tx.send(r).unwrap(); });
    device.poll(wgpu::Maintain::Wait);
    rx.await??;
    let view = slice.get_mapped_range();
    Ok(bytemuck::cast_slice::<u8, u32>(&view)[0])
}
```

If this returns `Ok(1)` on the laptop, the foundations are sound — proceed to MD5.

---

## Appendix B — Honest Comparison With Hashcat

| Dimension | Hashcat | This project |
|---|---|---|
| Language | C + OpenCL/CUDA/HIP | Rust + WGSL via wgpu |
| Algorithms | 350+ | 3 (md5, sha1, sha256) — by design |
| Per-vendor tuning | Yes (years of work) | No (portable WGSL) |
| Throughput | Reference industry tool | Expect 5–25% of tuned-OpenCL on the same iGPU |
| Safety | Manual memory mgmt | Memory-safe Rust + safe wgpu wrapper |
| Distributed | Limited (workload-distribution server) | **Out of scope** — single Windows Intel laptop only |
| Target hardware | Datacenter / enthusiast NVIDIA GPUs | Intel iGPU on a student laptop |
| Goal | Production auditing | **Education + benchmarking** |

This is the framing for the final report. The project is *not* trying to beat Hashcat; it is trying to teach you how a Hashcat-shaped system works, in a modern stack you can read top to bottom.
