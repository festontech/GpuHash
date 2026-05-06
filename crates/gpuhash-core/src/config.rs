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
    /// Optional name for save/resume. If set, the engine persists progress under
    /// `<sessions_dir>/<session_name>.session.json`.
    pub session_name: Option<String>,
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
