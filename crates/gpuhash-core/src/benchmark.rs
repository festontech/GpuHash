//! Synthetic-load throughput benchmark for the GPU backend.
//!
//! Drives the bruteforce runner over a fixed mask (`?l?l?l?l?l?l`) with one
//! never-matching target and counts how many candidates clear the pipeline
//! within the requested time window. The Phase-9 sustained-throughput sweep
//! will replace this with a proper thermal-aware harness; for Phase 5 this is
//! enough to print three honest numbers for *this* GPU.

use std::time::Instant;

use crate::gpu::{
    algos::{md5 as md5_kernel, sha1 as sha1_kernel, sha256 as sha256_kernel},
    bruteforce_runner::BruteforceRunner,
    kernel_spec::BruteforceKernelSpec,
    runner::{DEFAULT_MAX_IN_FLIGHT, DEFAULT_WORKGROUP_SIZE},
};
use crate::mask::Mask;
use crate::{Algorithm, Error, Result};

/// Per-algorithm benchmark result.
#[derive(Clone, Debug)]
pub struct BenchmarkReport {
    pub algo: Algorithm,
    pub batch_size: u32,
    pub workgroup_size: u32,
    pub candidates_tested: u64,
    pub elapsed_secs: f64,
    pub hashes_per_sec: f64,
}

/// Bench knobs. Sensible defaults are baked in; callers override via the CLI.
#[derive(Copy, Clone, Debug)]
pub struct BenchmarkConfig {
    /// Total wall-clock budget (seconds). Defaults to 5 — short enough for the
    /// `benchmark` CLI smoke check, long enough to drown out cold-start tax.
    pub secs: u64,
    /// GPU batch size override. `None` → engine default.
    pub batch_size: Option<u32>,
    /// GPU workgroup size override. `None` → engine default.
    pub workgroup_size: Option<u32>,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            secs: 5,
            batch_size: None,
            workgroup_size: None,
        }
    }
}

/// Run the GPU bruteforce loop for `cfg.secs` and report `hashes_per_sec`.
///
/// Always uses the bruteforce runner (host doesn't ship candidate bytes, so
/// the measurement is GPU-bound rather than I/O-bound — matches the workload
/// real users care about).
pub async fn benchmark_algo(algo: Algorithm, cfg: BenchmarkConfig) -> Result<BenchmarkReport> {
    let spec: BruteforceKernelSpec = match algo {
        Algorithm::Md5 => md5_kernel::BRUTE_SPEC,
        Algorithm::Sha1 => sha1_kernel::BRUTE_SPEC,
        Algorithm::Sha256 => sha256_kernel::BRUTE_SPEC,
    };

    // Mask whose keyspace is large enough for a few seconds at the chosen
    // defaults. ?l^6 = 308 915 776; re-base back to 0 once we've walked it,
    // looping until the time budget is up.
    let mask = Mask::parse("?l?l?l?l?l?l").map_err(Error::BadFormat)?;
    let span = mask.total() as u32;

    // One bogus target that nothing in the keyspace will match. Length =
    // spec.digest_bytes; bytes 0..=N as a deterministic filler.
    let bogus_target: Vec<u8> = (0..spec.digest_bytes as u8).collect();
    let targets = vec![bogus_target];

    let batch_size = cfg.batch_size.unwrap_or(crate::engine::DEFAULT_GPU_BATCH);
    let workgroup_size = cfg.workgroup_size.unwrap_or(DEFAULT_WORKGROUP_SIZE);
    let max_matches: u32 = 16; // can't happen, but the runner requires > 0

    let runner = BruteforceRunner::new(
        spec,
        &mask,
        &targets,
        batch_size,
        workgroup_size,
        max_matches,
        DEFAULT_MAX_IN_FLIGHT,
    )
    .await?;
    let max_in_flight = runner.max_in_flight();

    let deadline_secs = cfg.secs as f64;
    let start = Instant::now();
    let mut tested: u64 = 0;
    let mut cursor: u32 = 0;

    // Same ring-scheduler pattern as the engine: prime up to max_in_flight, then
    // pop+read and refill. Loop the cursor back to 0 when it laps the keyspace.
    let mut pending: std::collections::VecDeque<(usize, u32)> =
        std::collections::VecDeque::with_capacity(max_in_flight);
    let mut next_slot: usize = 0;

    loop {
        // Refill.
        while pending.len() < max_in_flight && start.elapsed().as_secs_f64() < deadline_secs {
            let remaining = span - cursor;
            let count = remaining.min(batch_size);
            runner.submit(next_slot, cursor, count)?;
            pending.push_back((next_slot, count));
            cursor = cursor.checked_add(count).unwrap_or(span);
            if cursor >= span {
                cursor = 0;
            }
            next_slot = (next_slot + 1) % max_in_flight;
        }

        let Some((slot, count)) = pending.pop_front() else {
            break;
        };
        let _ = runner.read_matches(slot).await?;
        tested += count as u64;
    }

    let elapsed_secs = start.elapsed().as_secs_f64().max(1e-9);
    Ok(BenchmarkReport {
        algo,
        batch_size,
        workgroup_size,
        candidates_tested: tested,
        elapsed_secs,
        hashes_per_sec: tested as f64 / elapsed_secs,
    })
}
