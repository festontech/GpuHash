# GPU Password Auditing Framework

An educational password-auditing and GPU-compute benchmarking tool, built in Rust.

> **Defensive use only.** Use against hashes you own — for example, evaluating the strength of credentials in your own systems. Use against systems or data you do not have explicit authorization to test is illegal in most jurisdictions and is not supported by this project. See [docs/ETHICS.md](docs/ETHICS.md).

## What this is

A school project that recreates a simplified, Hashcat-style password auditing tool to teach:

- GPU compute via WGSL on [`wgpu`](https://wgpu.rs).
- Rust workspace architecture — a shared `gpuhash-core` engine consumed by both a CLI and a Tauri+React GUI.
- Async pipelines that stream live progress to a UI.

## What this is *not*

- Not a Hashcat replacement (it will run at a fraction of Hashcat's throughput on the same hardware — that is the point of measuring it).
- Not for cracking hashes you do not own.
- Not a distributed cracker. Target environment is a single laptop.

## Target environment

A single **Windows laptop with an Intel CPU/GPU**. `wgpu` will pick the DX12 backend on Windows; Intel iGPUs run WGSL compute kernels well, with the realistic throughput numbers documented in [docs/ARCHITECTURE.md §4.1.2](docs/ARCHITECTURE.md).

## Documentation

| File | Purpose |
|---|---|
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Full architecture & implementation guide — start here. |
| [docs/ROADMAP.md](docs/ROADMAP.md) | Phased plan, Phase 0 → Phase 10, with a checkbox tracker. |
| [docs/ETHICS.md](docs/ETHICS.md) | Defensive framing & rules of engagement. |
| [docs/LOGBOOK.md](docs/LOGBOOK.md) | Append-only build log. |

## Prerequisites (one-time, Windows)

1. **Rust toolchain** — install rustup from <https://rustup.rs> (default `stable-msvc`).
2. **MSVC build tools** — Visual Studio Build Tools 2022 with the *Desktop development with C++* workload.
3. **WebView2 runtime** — required by Tauri (already present on Windows 11; install from Microsoft otherwise).
4. **Node.js ≥ 20** — only needed when starting Phase 7 (Tauri/React frontend).

## Building

```powershell
cargo check --workspace
cargo build --workspace
```

## Running (Phase 0 stub)

```powershell
cargo run -p gpuhash-cli -- --help
```

The CLI currently prints "Phase 0 stub" for `attack`/`benchmark` — see [docs/ROADMAP.md](docs/ROADMAP.md).

## Project layout

```
GpuHash/
├── Cargo.toml                # workspace root
├── docs/                     # architecture, roadmap, ethics, logbook
└── crates/
    ├── gpuhash-core/         # the engine library (shared)
    └── gpuhash-cli/          # clap-based terminal frontend
```

The `gpuhash-tauri` crate is added in Phase 7 via `npm create tauri-app` — see [docs/ROADMAP.md](docs/ROADMAP.md).

## License

MIT OR Apache-2.0.
