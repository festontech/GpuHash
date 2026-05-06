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

## 2026-05-?? — CPU MD5 baseline (Phase 1)

**Goal today.** End-to-end CPU dictionary attack working with RFC 1321 test vectors passing. Record baseline CPU H/s.

*(fill in when done)*

---
