//! gpuhash-core — engine library for the GPU Password Auditing Framework.
//!
//! This crate is the single source of truth for hash-auditing logic and is consumed by
//! both `gpuhash-cli` (terminal frontend) and `gpuhash-tauri` (desktop GUI, Phase 7+)
//! without code duplication.
//!
//! See `docs/ARCHITECTURE.md` for the full design.

pub mod attacks;
pub mod benchmark;
pub mod config;
pub mod digest;
pub mod engine;
pub mod error;
pub mod event;
pub mod gpu;
pub mod hash;
pub mod loader;
pub mod mask;

// Phase 4+ scheduler / benchmark / session modules:
// pub mod scheduler;
// pub mod benchmark;
// pub mod session;

pub use benchmark::{benchmark_algo, BenchmarkConfig, BenchmarkReport};
pub use config::{AttackConfig, AttackMode, Backend, GpuTuning};
pub use engine::{Engine, RunningAttack};
pub use error::{Error, Result};
pub use event::{AttackSummary, EngineEvent};
pub use hash::Algorithm;
