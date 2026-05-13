//! The engine orchestrates an audit and streams progress events.
//!
//! Phase 1 wired the CPU path; Phase 3 added the dictionary GPU path; Phase 4
//! adds the bruteforce GPU path behind the same `EngineEvent` contract. The
//! consumer (CLI, Tauri shell) sees the same event shape regardless of which
//! backend/mode runs underneath.
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

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    attacks::build_source,
    config::{AttackConfig, AttackMode, Backend},
    digest::digest,
    event::{AttackSummary, EngineEvent},
    gpu::{
        bruteforce_runner::Md5BruteforceRunner,
        buffers::{CandidateSlot, MAX_CANDIDATE_LEN},
        runner::{Md5GpuRunner, DEFAULT_MAX_IN_FLIGHT},
    },
    hash::Algorithm,
    loader::{load_targets, TargetSet},
    mask::Mask,
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

    match &cfg.mode {
        AttackMode::Dictionary { .. } => run_gpu_dict(&cfg, &targets, tx, cancel).await,
        AttackMode::Bruteforce { mask, start, end } => {
            let parsed = Mask::parse(mask).map_err(Error::BadFormat)?;
            let total = parsed.total();
            let start = *start;
            let end = end.unwrap_or(total);
            run_gpu_bruteforce(&cfg, &targets, parsed, start, end, tx, cancel).await
        }
    }
}

async fn run_gpu_dict(
    cfg: &AttackConfig,
    targets: &TargetSet,
    tx: &mpsc::UnboundedSender<EngineEvent>,
    cancel: &CancellationToken,
) -> Result<()> {
    let mut source = build_source(&cfg.mode)?;
    let total = source.estimate_total();

    let _ = tx.send(EngineEvent::Started {
        algo: cfg.algo,
        total,
    });

    let batch_size = DEFAULT_GPU_BATCH;
    let max_matches = batch_size;
    let runner = Md5GpuRunner::new(
        &targets.hashes,
        batch_size,
        max_matches,
        DEFAULT_MAX_IN_FLIGHT,
    )
    .await?;

    let start = Instant::now();
    let mut tested: u64 = 0;
    let mut matches_total: u64 = 0;
    let mut oversize_skipped: u64 = 0;
    let mut last_progress = Instant::now();

    let max_in_flight = runner.max_in_flight();
    let mut pending: VecDeque<PendingDictBatch> = VecDeque::with_capacity(max_in_flight);

    let mut slot_batch: Vec<CandidateSlot> = Vec::with_capacity(batch_size as usize);
    let mut plaintext_batch: Vec<String> = Vec::with_capacity(batch_size as usize);
    let mut next_slot: usize = 0;
    let mut exhausted = false;

    loop {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }

        while pending.len() < max_in_flight && !exhausted {
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
                            oversize_skipped += 1;
                            tested += 1;
                        }
                    },
                }
            }
            if slot_batch.is_empty() {
                break;
            }

            runner.submit(next_slot, &slot_batch)?;
            pending.push_back(PendingDictBatch {
                slot: next_slot,
                plaintexts: std::mem::take(&mut plaintext_batch),
                candidate_count: slot_batch.len() as u64,
            });
            plaintext_batch = Vec::with_capacity(batch_size as usize);
            next_slot = (next_slot + 1) % max_in_flight;
        }

        let Some(batch) = pending.pop_front() else {
            break;
        };

        let matches = runner.read_matches(batch.slot).await?;
        for m in matches {
            let idx = m.candidate_idx as usize;
            if idx >= batch.plaintexts.len() {
                continue;
            }
            let _ = tx.send(EngineEvent::Match {
                plaintext: batch.plaintexts[idx].clone(),
                target_idx: m.target_idx,
            });
            matches_total += 1;
        }
        tested += batch.candidate_count;

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

    finish(tx, tested, matches_total, start);
    Ok(())
}

async fn run_gpu_bruteforce(
    cfg: &AttackConfig,
    targets: &TargetSet,
    mask: Mask,
    range_start: u64,
    range_end: u64,
    tx: &mpsc::UnboundedSender<EngineEvent>,
    cancel: &CancellationToken,
) -> Result<()> {
    let total_keyspace = mask.total();
    if range_start > range_end {
        return Err(Error::BadFormat(format!(
            "bruteforce range start={range_start} > end={range_end}"
        )));
    }
    if range_end > total_keyspace {
        return Err(Error::BadFormat(format!(
            "bruteforce end={range_end} exceeds mask keyspace {total_keyspace}"
        )));
    }
    // The Phase-4 shader uses a u32 candidate index, which `Mask::parse` already
    // enforces by refusing keyspaces > u32::MAX. Subranges therefore also fit.
    if range_end > u32::MAX as u64 {
        return Err(Error::Gpu(format!(
            "bruteforce end={range_end} exceeds u32; Phase 4 shader uses 32-bit indices"
        )));
    }

    let span = range_end - range_start;
    let _ = tx.send(EngineEvent::Started {
        algo: cfg.algo,
        total: Some(span),
    });

    let batch_size = DEFAULT_GPU_BATCH;
    let max_matches = batch_size;
    let runner = Md5BruteforceRunner::new(
        &mask,
        &targets.hashes,
        batch_size,
        max_matches,
        DEFAULT_MAX_IN_FLIGHT,
    )
    .await?;

    let start = Instant::now();
    let mut tested: u64 = 0;
    let mut matches_total: u64 = 0;
    let mut last_progress = Instant::now();

    let max_in_flight = runner.max_in_flight();
    let mut pending: VecDeque<PendingBruteBatch> = VecDeque::with_capacity(max_in_flight);

    let mut cursor: u64 = range_start;
    let mut next_slot: usize = 0;

    loop {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }

        // Prime / refill.
        while pending.len() < max_in_flight && cursor < range_end {
            let remaining = range_end - cursor;
            let count = remaining.min(batch_size as u64) as u32;
            let base = cursor as u32;
            runner.submit(next_slot, base, count)?;
            pending.push_back(PendingBruteBatch {
                slot: next_slot,
                base,
                count,
            });
            cursor += count as u64;
            next_slot = (next_slot + 1) % max_in_flight;
        }

        let Some(batch) = pending.pop_front() else {
            break;
        };

        let matches = runner.read_matches(batch.slot).await?;
        for m in matches {
            if m.candidate_idx >= batch.count {
                continue;
            }
            let abs_idx = batch.base as u64 + m.candidate_idx as u64;
            let bytes = mask.candidate_at(abs_idx);
            // Mask emits ASCII bytes only (parser validates literals are ASCII,
            // charsets are inherently ASCII), so this conversion is infallible
            // in practice.
            let plaintext = String::from_utf8(bytes).unwrap_or_else(|_| {
                tracing::error!("mask emitted non-utf8 bytes — should be unreachable");
                String::new()
            });
            let _ = tx.send(EngineEvent::Match {
                plaintext,
                target_idx: m.target_idx,
            });
            matches_total += 1;
        }
        tested += batch.count as u64;

        if last_progress.elapsed() >= Duration::from_millis(100) {
            emit_progress(tx, tested, Some(span), &start);
            last_progress = Instant::now();
        }
    }

    finish(tx, tested, matches_total, start);
    Ok(())
}

fn finish(
    tx: &mpsc::UnboundedSender<EngineEvent>,
    tested: u64,
    matches_total: u64,
    start: Instant,
) {
    let elapsed_secs = start.elapsed().as_secs_f64();
    let _ = tx.send(EngineEvent::Finished {
        summary: AttackSummary {
            tested_total: tested,
            matches_total,
            elapsed_secs,
        },
    });
}

/// One in-flight dictionary batch waiting for its results.
struct PendingDictBatch {
    slot: usize,
    plaintexts: Vec<String>,
    candidate_count: u64,
}

/// One in-flight bruteforce batch — no plaintexts stored, we reconstruct from
/// `mask.candidate_at(base + match.candidate_idx)` when a match comes back.
struct PendingBruteBatch {
    slot: usize,
    base: u32,
    count: u32,
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
