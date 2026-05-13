//! `gpuhash` — terminal frontend for the GPU Password Auditing Framework.
//!
//! Phase 1: drives the CPU engine end-to-end for dictionary MD5 audits. Renders
//! progress to stderr (or NDJSON to stdout with `--json`).

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use gpuhash_core::{
    benchmark_algo, Algorithm, AttackConfig, AttackMode, Backend, BenchmarkConfig, Engine,
    EngineEvent, GpuTuning,
};

#[derive(Parser, Debug)]
#[command(
    name = "gpuhash",
    version,
    about = "Educational GPU password auditor",
    long_about = "An educational password-auditing and GPU compute benchmarking tool. \
                  Use against hashes you own. See docs/ETHICS.md."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run an audit against your own hash file.
    Attack {
        /// Hash algorithm: md5 | sha1 | sha256.
        #[arg(long, value_parser = parse_algo)]
        algo: Algorithm,

        /// Path to a file containing one hex-encoded digest per line.
        #[arg(long)]
        hashes: PathBuf,

        /// Path to a wordlist (one candidate per line). Mutually exclusive with --mask.
        #[arg(long, group = "mode")]
        wordlist: Option<PathBuf>,

        /// Hashcat-style mask, e.g. "?l?l?l?l?d?d". Mutually exclusive with --wordlist.
        #[arg(long, group = "mode")]
        mask: Option<String>,

        /// Optional session name for save/resume.
        #[arg(long)]
        session: Option<String>,

        /// Required acknowledgement that the hashes belong to you.
        ///
        /// This is trivially bypassable but exists to document intent — see docs/ETHICS.md.
        #[arg(long)]
        i_own_these_hashes: bool,

        /// Run the audit on the GPU (WGSL via wgpu). MD5 only in Phase 3.
        #[arg(long)]
        gpu: bool,

        /// Override GPU batch size (default 65536 = 1<<16).
        #[arg(long, value_name = "N")]
        gpu_batch: Option<u32>,

        /// Override GPU workgroup size (allowed: 32, 64, 128, 256; default 64).
        #[arg(long, value_name = "N")]
        gpu_workgroup: Option<u32>,

        /// Emit NDJSON `EngineEvent`s on stdout instead of human-readable progress.
        #[arg(long)]
        json: bool,
    },

    /// Benchmark hash throughput on the local GPU.
    Benchmark {
        /// Specific algorithm to benchmark; omit to benchmark all supported algorithms.
        #[arg(long, value_parser = parse_algo)]
        algo: Option<Algorithm>,

        /// Sustained measurement window, in seconds (default 5; Phase 9 sweep
        /// uses ≥ 60 for thermal-aware sustained throughput).
        #[arg(long, default_value_t = 5)]
        secs: u64,

        /// Override GPU batch size (default 1<<18).
        #[arg(long, value_name = "N")]
        gpu_batch: Option<u32>,

        /// Override GPU workgroup size (32, 64, 128, or 256; default 256).
        #[arg(long, value_name = "N")]
        gpu_workgroup: Option<u32>,
    },
}

fn parse_algo(s: &str) -> std::result::Result<Algorithm, String> {
    s.parse()
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    match dispatch(cli).await {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}

async fn dispatch(cli: Cli) -> Result<ExitCode> {
    match cli.cmd {
        Cmd::Attack {
            i_own_these_hashes: false,
            ..
        } => {
            bail!(
                "Refusing to run an audit without --i-own-these-hashes. \
                 See docs/ETHICS.md for why this flag exists."
            );
        }

        Cmd::Attack {
            algo,
            hashes,
            wordlist,
            mask,
            session,
            gpu,
            gpu_batch,
            gpu_workgroup,
            json,
            i_own_these_hashes: true,
        } => {
            run_attack(
                algo,
                hashes,
                wordlist,
                mask,
                session,
                gpu,
                gpu_batch,
                gpu_workgroup,
                json,
            )
            .await
        }

        Cmd::Benchmark {
            algo,
            secs,
            gpu_batch,
            gpu_workgroup,
        } => run_benchmark(algo, secs, gpu_batch, gpu_workgroup).await,
    }
}

async fn run_benchmark(
    algo: Option<Algorithm>,
    secs: u64,
    gpu_batch: Option<u32>,
    gpu_workgroup: Option<u32>,
) -> Result<ExitCode> {
    let algos: Vec<Algorithm> = match algo {
        Some(a) => vec![a],
        None => vec![Algorithm::Md5, Algorithm::Sha1, Algorithm::Sha256],
    };
    let cfg = BenchmarkConfig {
        secs,
        batch_size: gpu_batch,
        workgroup_size: gpu_workgroup,
    };

    for a in algos {
        let report = benchmark_algo(a, cfg).await?;
        println!(
            "{:<7} {:>10.1} MH/s   ({} candidates in {:.2}s, batch={}, wg={})",
            format!("{a}:"),
            report.hashes_per_sec / 1e6,
            report.candidates_tested,
            report.elapsed_secs,
            report.batch_size,
            report.workgroup_size,
        );
    }

    Ok(ExitCode::SUCCESS)
}

#[allow(clippy::too_many_arguments)]
async fn run_attack(
    algo: Algorithm,
    hashes: PathBuf,
    wordlist: Option<PathBuf>,
    mask: Option<String>,
    session_name: Option<String>,
    gpu: bool,
    gpu_batch: Option<u32>,
    gpu_workgroup: Option<u32>,
    json: bool,
) -> Result<ExitCode> {
    let mode = match (wordlist, mask) {
        (Some(wordlist), None) => AttackMode::Dictionary { wordlist },
        (None, Some(mask)) => AttackMode::Bruteforce {
            mask,
            start: 0,
            end: None,
        },
        (None, None) => bail!("one of --wordlist or --mask is required"),
        (Some(_), Some(_)) => bail!("--wordlist and --mask are mutually exclusive"),
    };

    let cfg = AttackConfig {
        algo,
        hashes_path: hashes,
        mode,
        backend: if gpu { Backend::Gpu } else { Backend::Cpu },
        gpu_tuning: GpuTuning {
            batch_size: gpu_batch,
            workgroup_size: gpu_workgroup,
        },
        session_name,
    };

    let engine = Engine::new();
    let mut running = engine.run(cfg);

    let mut matches_found: u64 = 0;
    let mut hit_error = false;

    while let Some(event) = running.next_event().await {
        if json {
            let line = serde_json::to_string(&event)?;
            println!("{line}");
        } else {
            render_human(&event, &mut matches_found, &mut hit_error);
        }
    }

    // Newline after the carriage-returned progress line so the shell prompt isn't
    // glued to it.
    if !json {
        let _ = writeln!(std::io::stderr());
    }

    if hit_error {
        return Ok(ExitCode::from(2));
    }
    // Exit code 1 = audit "failed open" (matches found). Per docs/ARCHITECTURE.md §7.4.
    if matches_found > 0 {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn render_human(event: &EngineEvent, matches_found: &mut u64, hit_error: &mut bool) {
    let mut err = std::io::stderr().lock();
    match event {
        EngineEvent::Started { algo, total } => {
            let _ = writeln!(
                err,
                "Audit started: algo={algo}, total={}",
                total
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| "unknown".into())
            );
        }
        EngineEvent::Progress {
            tested,
            hashes_per_sec,
            eta_secs,
        } => {
            let eta = eta_secs
                .map(|s| format!("{s:.1}s"))
                .unwrap_or_else(|| "?".into());
            let _ = write!(
                err,
                "\r{tested:>10}  {hashes_per_sec:>12.0} H/s  ETA {eta:<8}"
            );
        }
        EngineEvent::Match {
            plaintext,
            target_idx,
        } => {
            let _ = writeln!(err, "\nmatch[{target_idx}]: {plaintext}");
            *matches_found += 1;
        }
        EngineEvent::Finished { summary } => {
            let rate = summary.tested_total as f64 / summary.elapsed_secs.max(1e-9);
            let _ = writeln!(
                err,
                "\nDone. tested={} matches={} elapsed={:.2}s rate={:.0} H/s",
                summary.tested_total, summary.matches_total, summary.elapsed_secs, rate
            );
        }
        EngineEvent::Error { message } => {
            let _ = writeln!(err, "\nengine error: {message}");
            *hit_error = true;
        }
    }
}
