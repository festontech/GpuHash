# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this project is

An educational, **defensive** password-auditing and GPU-compute benchmarking tool, written in Rust. It is *not* a cracker. Every artifact (code, CLI strings, UI labels, docs) reinforces "auditing/benchmarking" framing — "Audit" / "Run Audit", never "Crack" / "Hack". Internal type names like `AttackRunner` are technical jargon; keep them out of user-facing strings. See [docs/ETHICS.md](docs/ETHICS.md) for the rules of engagement and what the project will refuse to add (no breach-DB integrations, no leak ingestion, no anti-detection).

Target environment is a **single Windows laptop with an Intel CPU/iGPU**. Distributed/multi-machine work is explicitly out of scope.

## Common commands

```powershell
cargo check --workspace                  # fast verify
cargo build --workspace                  # debug build
cargo build --workspace --release        # for benchmarking — use this for any H/s number
cargo test --workspace                   # all tests (RFC 1321 vectors live in core)
cargo test -p gpuhash-core digest::      # run digest tests only
cargo fmt --check && cargo clippy -- -D warnings   # pre-demo smoke check

cargo run -p gpuhash-cli -- --help

# Phase 1 end-to-end audit (the canonical smoke test):
cargo run -p gpuhash-cli -- attack `
  --algo md5 `
  --hashes examples/sample_hashes.txt `
  --wordlist examples/tiny_dict.txt `
  --i-own-these-hashes

# NDJSON for scripting (pipe into ConvertFrom-Json or jq):
cargo run -p gpuhash-cli -- attack ... --i-own-these-hashes --json
```

CLI exit codes (per [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) §7.4): **0** = ran clean, no matches; **1** = matches found (audit "failed open"); **2** = error or refusal.

If you change [examples/tiny_dict.txt](examples/tiny_dict.txt), regenerate [examples/sample_hashes.txt](examples/sample_hashes.txt) — see the PowerShell snippet in [examples/README.md](examples/README.md). The first version of `sample_hashes.txt` was hand-typed and had a wrong digest; trust the script, not your fingers.

## Architecture: the load-bearing ideas

**Single source of truth — `gpuhash-core` is consumed by everyone.** Both [crates/gpuhash-cli/](crates/gpuhash-cli/) and (Phase 7+) `gpuhash-tauri` are thin shells. They translate user intent into engine API calls and translate `EngineEvent`s into terminal output / UI updates. Never duplicate logic across shells.

**The `EngineEvent` contract is the bridge.** [crates/gpuhash-core/src/event.rs](crates/gpuhash-core/src/event.rs) defines a tagged-union `EngineEvent` (`Started` / `Progress` / `Match` / `Finished` / `Error`) with `#[serde(tag = "type")]`. The same JSON shape is consumed by the CLI's `--json` mode *and* (later) by the React frontend as a TypeScript discriminated union. Changing this enum is changing a public contract — update the CLI renderer and Tauri/TS types in the same PR.

**Lifecycle.** `Engine::new()` is sync and cheap; `engine.run(cfg)` spawns a `tokio::spawn`'d task and returns a `RunningAttack { events, cancel }`. Consumers drain `events.recv().await` until `Finished` or `Error`. `engine.run` requires a tokio runtime context — the CLI uses `#[tokio::main]`; the Tauri shell will use Tauri's runtime.

**Errors split by crate type.** Library (`gpuhash-core`) uses `thiserror` (`Error::{Io, BadFormat, Gpu, Cancelled, NotImplemented}`). Binaries use `anyhow`. **Don't add `anyhow` to `gpuhash-core`.**

**Crate boundary discipline.** `clap` stays out of `gpuhash-core`. The CLI has a small `parse_algo` adapter that calls `Algorithm::from_str` rather than letting clap drive the engine's types. If you're tempted to put a CLI concern into the engine, put it in the CLI instead.

**Progress throttling.** Engine emits `Progress` events at most ~10 Hz ([engine.rs](crates/gpuhash-core/src/engine.rs) checks `last_progress.elapsed() >= 100ms`). Don't remove this throttle when adding GPU dispatch — the UI/stdout cannot keep up with millions of events/sec.

## Phasing — where we are and what's allowed

The project has a strict phased roadmap in [docs/ROADMAP.md](docs/ROADMAP.md). **Phase 0 + Phase 1 are complete** (CPU MD5 prototype, single-threaded, ~1.67 MH/s baseline). Phase 2 (GPU smoke test) is next.

Implications when modifying code:
- `wgpu` and `bytemuck` are commented out in [crates/gpuhash-core/Cargo.toml](crates/gpuhash-core/Cargo.toml) — uncomment them only when starting Phase 2. Same for `rayon` (Phase 4), `sha1`/`sha2` (Phase 5).
- `gpu`, `scheduler`, `benchmark`, `session` modules are commented out in [lib.rs](crates/gpuhash-core/src/lib.rs). Add them as the roadmap reaches them; don't pre-stub.
- The `Bruteforce` mode currently returns `Error::NotImplemented("bruteforce (Phase 4)")` — that is intentional. Brute-force candidate generation moves onto the GPU in Phase 4 (derived from `gid.x` in WGSL), not the CPU.
- The CLI's `Benchmark` subcommand is a stub until Phase 2+.

When you finish a phase: tick the checkboxes in [docs/ROADMAP.md](docs/ROADMAP.md) **and** append a dated entry to [docs/LOGBOOK.md](docs/LOGBOOK.md) using the template at the top of that file. The logbook is append-only.

## GPU specifics (Phase 2+, not yet active)

- Pick **WGSL via `wgpu`**, not OpenCL. Decision and trade-offs are documented in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) §4.1.1. The course teaches OpenCL; the planned bake-off (Phase 10 stretch) reconciles this.
- Tune for an **Intel iGPU**: start at `batch_size = 1<<16` (not `1<<20`), workgroup size 32 or 64 (not 256). Benchmarks must run **≥ 60 s** to capture sustained-throughput-after-thermal-throttle, not peak.
- Build pipelines once per algorithm; reuse `wgpu::Buffer`s across batches; never `MAP_READ` a hot path buffer; only the small results buffer travels device→host.
- For GPU-bound structs use `bytemuck` `#[repr(C)] #[derive(Pod, Zeroable)]` and `cast_slice` directly into `queue.write_buffer`.
- Validation gate when GPU lands: bit-exact match between `--cpu` and `--gpu` paths on the same input. Don't trust an H/s number from a kernel that hasn't passed RFC/NIST vectors.

## Style and conventions

- `tracing` from day one — no `println!` in library code.
- Every workspace member denies `unsafe_code` and warns on `rust_2018_idioms` (see [Cargo.toml](crates/gpuhash-core/Cargo.toml)). Keep it that way; `wgpu` is a safe wrapper, you should not need `unsafe`.
- Test vectors first, performance later. RFC 1321 / NIST vectors must pass on both CPU and GPU paths before optimizing throughput.
- The `--i-own-these-hashes` gate is trivially bypassable but **must stay** — it documents intent and is part of the ethics framing.
