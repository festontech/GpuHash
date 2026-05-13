//! Live progress events emitted by the engine.
//!
//! Tagged-union serialization means these events are also a TypeScript discriminated
//! union on the `type` field — the same JSON shape works for both the Rust CLI consumer
//! and the React frontend (added in Phase 7).

use crate::hash::Algorithm;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttackSummary {
    pub tested_total: u64,
    pub matches_total: u64,
    pub elapsed_secs: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EngineEvent {
    /// Engine has accepted the configuration and started work.
    Started {
        algo: Algorithm,
        /// Total candidate count if known up-front (e.g. wordlist length); `None` for
        /// unbounded sources or when the source can't cheaply count.
        total: Option<u64>,
    },

    /// Periodic progress update. Emitted at most ~10 Hz from the engine; consumers
    /// (UI) may further throttle.
    Progress {
        tested: u64,
        hashes_per_sec: f64,
        eta_secs: Option<f64>,
    },

    /// A candidate's digest matched a target hash.
    ///
    /// `target_idx` is the zero-based index into the input hash file (so the consumer
    /// can map back to the original line if it needs to).
    Match { plaintext: String, target_idx: u32 },

    /// The audit ran to completion (or was cancelled cleanly).
    Finished { summary: AttackSummary },

    /// Unrecoverable error during the audit. The engine will not emit further events
    /// on this channel after `Error`.
    Error { message: String },
}
