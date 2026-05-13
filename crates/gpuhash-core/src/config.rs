//! Public configuration types passed into the engine.
//!
//! These are part of the contract with both `gpuhash-cli` and the Tauri frontend, so
//! they need stable serde representations.

use crate::hash::Algorithm;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttackConfig {
    pub algo: Algorithm,
    pub hashes_path: PathBuf,
    pub mode: AttackMode,
    /// Compute backend. Defaults to `Cpu` when missing in deserialized JSON so
    /// session files written before Phase 3 still load.
    #[serde(default)]
    pub backend: Backend,
    /// GPU tuning knobs (Phase 4+). When fields are `None`, the engine picks
    /// the iGPU-tuned defaults from `docs/ARCHITECTURE.md`.
    #[serde(default)]
    pub gpu_tuning: GpuTuning,
    /// Optional name for save/resume. If set, the engine persists progress under
    /// `<sessions_dir>/<session_name>.session.json`.
    pub session_name: Option<String>,
}

/// GPU-side tuning parameters. Overrides the engine's iGPU defaults when set.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuTuning {
    /// Candidates per dispatch. Default 1<<16 (65 536).
    #[serde(default)]
    pub batch_size: Option<u32>,
    /// WGSL `@workgroup_size`. Must be one of the kernel-supported sizes
    /// (32, 64, 128, 256). Default 64.
    #[serde(default)]
    pub workgroup_size: Option<u32>,
}

/// Which compute path the engine should drive a run on.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// Single-threaded CPU reference path (Phase 1). Always available.
    #[default]
    Cpu,
    /// WGSL-on-wgpu GPU path (Phase 3+). Currently MD5 only.
    Gpu,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AttackMode {
    Dictionary {
        wordlist: PathBuf,
    },
    Bruteforce {
        /// Hashcat-style mask string (e.g. "?l?l?l?l?d?d").
        mask: String,
        /// Resume index (inclusive). Defaults to 0.
        #[serde(default)]
        start: u64,
        /// Stop index (exclusive). `None` runs to the end of the keyspace.
        #[serde(default)]
        end: Option<u64>,
    },
}
