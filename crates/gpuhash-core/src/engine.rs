//! The engine orchestrates an audit and streams progress events.
//!
//! Phase 1 wired the CPU path; Phase 3 adds a GPU path behind the same
//! `EngineEvent` contract — the consumer (CLI, Tauri shell) sees the same event
//! shape regardless of backend.
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
//!                              tokio task drives CPU or GPU loop
//! ```
//!
//! `engine.run` requires a tokio runtime context (it calls `tokio::spawn`). The
//! CLI runs under `#[tokio::main]`; the Tauri shell runs Tauri's tokio runtime.

use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    attacks::build_source,
    config::{AttackConfig, Backend},
    digest::digest,
    event::{AttackSummary, EngineEvent},
    gpu::{
        buffers::{CandidateSlot, MAX_CANDIDATE_LEN},
        runner::Md5GpuRunner,
    },
    hash::Algorithm,
    loader::{load_targets, TargetSet},
    Error, Result,
};

/// Default GPU batch size when none is otherwise specified. Sized for an Intel
/// iGPU per the architecture doc: 65 536 candidates × 60-byte slot ≈ 3.75 MB of
/// device memory for the candidate buffer.
const DEFAULT_GPU_BATCH: u32 = 1 << 16;

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
            let result = match cfg.backend {
                Backend::Cpu => run_cpu(cfg, &tx, &cancel_inner).await,
                Backend::Gpu => run_gpu(cfg, &tx, &cancel_inner).await,
            };
            if let Err(e) = result {
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

async fn run_cpu(
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
            emit_progress(tx, tested, total, &start);
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

async fn run_gpu(
    cfg: AttackConfig,
    tx: &mpsc::UnboundedSender<EngineEvent>,
    cancel: &CancellationToken,
) -> Result<()> {
    if cfg.algo != Algorithm::Md5 {
        return Err(Error::NotImplemented(
            "GPU backend currently supports md5 only (sha1/sha256 land in Phase 5)",
        ));
    }

    let targets: TargetSet = load_targets(&cfg.hashes_path, cfg.algo)?;
    let mut source = build_source(&cfg.mode)?;
    let total = source.estimate_total();

    let _ = tx.send(EngineEvent::Started {
        algo: cfg.algo,
        total,
    });

    let batch_size = DEFAULT_GPU_BATCH;
    // Each candidate produces one digest, so per dispatch at most one match per
    // candidate against this target set (collision-resistant hash). batch_size is
    // therefore a safe upper bound on matches-per-dispatch.
    let max_matches = batch_size;
    let runner = Md5GpuRunner::new(&targets.hashes, batch_size, max_matches).await?;

    let start = Instant::now();
    let mut tested: u64 = 0;
    let mut matches_total: u64 = 0;
    let mut oversize_skipped: u64 = 0;
    let mut last_progress = Instant::now();

    let mut slot_batch: Vec<CandidateSlot> = Vec::with_capacity(batch_size as usize);
    let mut plaintext_batch: Vec<String> = Vec::with_capacity(batch_size as usize);
    let mut exhausted = false;

    while !exhausted {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }

        // Fill one batch.
        slot_batch.clear();
        plaintext_batch.clear();
        while slot_batch.len() < batch_size as usize {
            match source.next_candidate()? {
                None => {
                    exhausted = true;
                    break;
                }
                Some(candidate) => match CandidateSlot::pack(candidate.as_bytes()) {
                    Some(slot) => {
                        slot_batch.push(slot);
                        plaintext_batch.push(candidate);
                    }
                    None => {
                        // Single-block MD5 caps candidates at MAX_CANDIDATE_LEN bytes.
                        // We still count toward `tested` so progress totals remain
                        // accurate, but the candidate is not dispatched.
                        oversize_skipped += 1;
                        tested += 1;
                    }
                },
            }
        }

        if slot_batch.is_empty() {
            break;
        }

        let matches = runner.dispatch_batch(&slot_batch).await?;
        for m in matches {
            let idx = m.candidate_idx as usize;
            // Defence-in-depth: a malformed kernel could in principle emit an
            // out-of-range index. Skip silently rather than panic.
            if idx >= plaintext_batch.len() {
                continue;
            }
            let _ = tx.send(EngineEvent::Match {
                plaintext: plaintext_batch[idx].clone(),
                target_idx: m.target_idx,
            });
            matches_total += 1;
        }
        tested += slot_batch.len() as u64;

        if last_progress.elapsed() >= Duration::from_millis(100) {
            emit_progress(tx, tested, total, &start);
            last_progress = Instant::now();
        }
    }

    if oversize_skipped > 0 {
        tracing::warn!(
            skipped = oversize_skipped,
            max_len = MAX_CANDIDATE_LEN,
            "skipped candidates longer than the single-block MD5 limit; multi-block lands later"
        );
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

fn emit_progress(
    tx: &mpsc::UnboundedSender<EngineEvent>,
    tested: u64,
    total: Option<u64>,
    start: &Instant,
) {
    let elapsed = start.elapsed().as_secs_f64().max(1e-9);
    let rate = tested as f64 / elapsed;
    let eta_secs = total.map(|t| (t.saturating_sub(tested) as f64) / rate.max(1.0));
    let _ = tx.send(EngineEvent::Progress {
        tested,
        hashes_per_sec: rate,
        eta_secs,
    });
}
