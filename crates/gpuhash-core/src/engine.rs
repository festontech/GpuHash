//! The engine orchestrates an audit and streams progress events.
//!
//! Phase 1 is **CPU-only**, single-threaded. Parallelism (rayon, GPU) is wired in
//! Phases 3–4 without changing this public surface.
//!
//! # Lifecycle
//!
//! ```text
//!   Engine::new()
//!       └─ engine.run(cfg)  ──►  RunningAttack { events, cancel() }
//!                                    │
//!                                    │  consumer awaits .next_event()
//!                                    │  until Finished or Error
//!                                    ▼
//!                              tokio task drains the wordlist
//! ```
//!
//! `engine.run` requires a tokio runtime context (it calls `tokio::spawn`). The CLI
//! runs under `#[tokio::main]`; the Tauri shell runs Tauri's tokio runtime.

use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    attacks::build_source,
    config::AttackConfig,
    digest::digest,
    event::{AttackSummary, EngineEvent},
    loader::load_targets,
    Error, Result,
};

#[derive(Default)]
pub struct Engine;

impl Engine {
    pub fn new() -> Self {
        Engine
    }

    /// Spawn an audit on a tokio task. Returns a handle that streams events and lets
    /// the caller cancel.
    pub fn run(&self, cfg: AttackConfig) -> RunningAttack {
        let (tx, rx) = mpsc::unbounded_channel();
        let cancel = CancellationToken::new();
        let cancel_inner = cancel.clone();

        tokio::spawn(async move {
            if let Err(e) = run_inner(cfg, &tx, &cancel_inner).await {
                let _ = tx.send(EngineEvent::Error {
                    message: e.to_string(),
                });
            }
        });

        RunningAttack { events: rx, cancel }
    }
}

/// Handle to a running audit. Drop or call `cancel()` to abort early.
pub struct RunningAttack {
    pub events: mpsc::UnboundedReceiver<EngineEvent>,
    cancel: CancellationToken,
}

impl RunningAttack {
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub async fn next_event(&mut self) -> Option<EngineEvent> {
        self.events.recv().await
    }
}

async fn run_inner(
    cfg: AttackConfig,
    tx: &mpsc::UnboundedSender<EngineEvent>,
    cancel: &CancellationToken,
) -> Result<()> {
    let targets = load_targets(&cfg.hashes_path, cfg.algo)?;
    let mut source = build_source(&cfg.mode)?;
    let total = source.estimate_total();

    let _ = tx.send(EngineEvent::Started {
        algo: cfg.algo,
        total,
    });

    let start = Instant::now();
    let mut tested: u64 = 0;
    let mut matches_total: u64 = 0;
    let mut last_progress = Instant::now();

    while let Some(candidate) = source.next_candidate()? {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }

        let d = digest(cfg.algo, candidate.as_bytes())?;

        for (idx, target) in targets.hashes.iter().enumerate() {
            if d == *target {
                let _ = tx.send(EngineEvent::Match {
                    plaintext: candidate.clone(),
                    target_idx: idx as u32,
                });
                matches_total += 1;
            }
        }

        tested += 1;

        // Throttle Progress events to ~10 Hz.
        if last_progress.elapsed() >= Duration::from_millis(100) {
            let elapsed = start.elapsed().as_secs_f64().max(1e-9);
            let rate = tested as f64 / elapsed;
            let eta_secs = total.map(|t| (t.saturating_sub(tested) as f64) / rate.max(1.0));
            let _ = tx.send(EngineEvent::Progress {
                tested,
                hashes_per_sec: rate,
                eta_secs,
            });
            last_progress = Instant::now();
        }
    }

    let elapsed_secs = start.elapsed().as_secs_f64();
    let _ = tx.send(EngineEvent::Finished {
        summary: AttackSummary {
            tested_total: tested,
            matches_total,
            elapsed_secs,
        },
    });
    Ok(())
}
