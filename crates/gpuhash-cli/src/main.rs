//! `gpuhash` — terminal frontend for the GPU Password Auditing Framework.
//!
//! Drives the engine end-to-end, renders progress to stderr or NDJSON to stdout
//! (with `--json`), and persists named sessions under the per-user data dir.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use gpuhash_core::{
    benchmark_algo, sessions_dir, Algorithm, AttackConfig, AttackMode, Backend, BenchmarkConfig,
    Engine, EngineEvent, GpuTuning, Session, SessionMatch, SessionStatus,
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

        /// Optional session name. The audit's config, matches, and summary are
        /// written to `<sessions_dir>/<name>.session.json` on completion.
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

    /// Manage named sessions (list / save / load / show / delete).
    Session {
        #[command(subcommand)]
        action: SessionCmd,
    },
}

#[derive(Subcommand, Debug)]
enum SessionCmd {
    /// List every saved session, newest-updated first.
    List,

    /// Save an `AttackConfig` to disk under NAME without executing it.
    /// Useful for prepping a reproducible audit run.
    Save {
        /// Session name (a-z, A-Z, 0-9, '-', '_', '.'). Stored as
        /// `<sessions_dir>/<name>.session.json`.
        #[arg(long)]
        name: String,

        /// Hash algorithm: md5 | sha1 | sha256.
        #[arg(long, value_parser = parse_algo)]
        algo: Algorithm,

        /// Path to a file containing one hex-encoded digest per line.
        #[arg(long)]
        hashes: PathBuf,

        /// Path to a wordlist. Mutually exclusive with --mask.
        #[arg(long, group = "mode")]
        wordlist: Option<PathBuf>,

        /// Hashcat-style mask. Mutually exclusive with --wordlist.
        #[arg(long, group = "mode")]
        mask: Option<String>,

        /// Store `Backend::Gpu` in the session (defaults to CPU).
        #[arg(long)]
        gpu: bool,

        /// Override GPU batch size.
        #[arg(long, value_name = "N")]
        gpu_batch: Option<u32>,

        /// Override GPU workgroup size.
        #[arg(long, value_name = "N")]
        gpu_workgroup: Option<u32>,
    },

    /// Execute a saved session's `AttackConfig`. Updates the session file
    /// with the resulting matches and summary.
    Load {
        /// Session name to execute.
        name: String,

        /// Required acknowledgement — same gate as `attack`.
        #[arg(long)]
        i_own_these_hashes: bool,

        /// NDJSON output mode (same shape as `attack --json`).
        #[arg(long)]
        json: bool,
    },

    /// Pretty-print the contents of a saved session.
    Show {
        /// Session name to inspect.
        name: String,
    },

    /// Remove a saved session. Idempotent — exits 0 if it didn't exist.
    Delete {
        /// Session name to remove.
        name: String,
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
            let cfg = build_config(
                algo,
                hashes,
                wordlist,
                mask,
                gpu,
                gpu_batch,
                gpu_workgroup,
                session.clone(),
            )?;
            run_attack(cfg, json).await
        }

        Cmd::Benchmark {
            algo,
            secs,
            gpu_batch,
            gpu_workgroup,
        } => run_benchmark(algo, secs, gpu_batch, gpu_workgroup).await,

        Cmd::Session { action } => run_session(action).await,
    }
}

#[allow(clippy::too_many_arguments)]
fn build_config(
    algo: Algorithm,
    hashes: PathBuf,
    wordlist: Option<PathBuf>,
    mask: Option<String>,
    gpu: bool,
    gpu_batch: Option<u32>,
    gpu_workgroup: Option<u32>,
    session_name: Option<String>,
) -> Result<AttackConfig> {
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

    Ok(AttackConfig {
        algo,
        hashes_path: hashes,
        mode,
        backend: if gpu { Backend::Gpu } else { Backend::Cpu },
        gpu_tuning: GpuTuning {
            batch_size: gpu_batch,
            workgroup_size: gpu_workgroup,
        },
        session_name,
    })
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

async fn run_attack(cfg: AttackConfig, json: bool) -> Result<ExitCode> {
    let session_name = cfg.session_name.clone();
    let engine = Engine::new();
    let mut running = engine.run(cfg.clone());

    let mut matches_found: u64 = 0;
    let mut hit_error = false;
    let mut session_matches: Vec<SessionMatch> = Vec::new();
    let mut final_summary = None;

    while let Some(event) = running.next_event().await {
        if json {
            let line = serde_json::to_string(&event)?;
            println!("{line}");
        } else {
            render_human(&event, &mut matches_found, &mut hit_error);
        }

        // Mirror state into the session record (independent of render path).
        match &event {
            EngineEvent::Match {
                plaintext,
                target_idx,
            } => {
                session_matches.push(SessionMatch {
                    plaintext: plaintext.clone(),
                    target_idx: *target_idx,
                });
                if json {
                    // human path already incremented matches_found
                    matches_found += 1;
                }
            }
            EngineEvent::Finished { summary } => {
                final_summary = Some(summary.clone());
            }
            EngineEvent::Error { .. } => {
                hit_error = true;
            }
            _ => {}
        }
    }

    if !json {
        // Newline after the carriage-returned progress line.
        let _ = writeln!(std::io::stderr());
    }

    if let Some(name) = session_name {
        let status = if hit_error {
            SessionStatus::Error
        } else {
            SessionStatus::Finished
        };
        persist_session(&name, &cfg, session_matches, final_summary, status)?;
    }

    if hit_error {
        return Ok(ExitCode::from(2));
    }
    // Exit code 1 = audit "failed open" (matches found). docs/ARCHITECTURE.md §7.4.
    if matches_found > 0 {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn persist_session(
    name: &str,
    cfg: &AttackConfig,
    matches: Vec<SessionMatch>,
    summary: Option<gpuhash_core::AttackSummary>,
    status: SessionStatus,
) -> Result<()> {
    let mut session = Session::new_saved(name, cfg.clone())?;
    session.matches = matches;
    session.summary = summary;
    session.status = status;
    let path = session.save()?;
    eprintln!("session saved: {}", path.display());
    Ok(())
}

async fn run_session(cmd: SessionCmd) -> Result<ExitCode> {
    match cmd {
        SessionCmd::List => {
            let rows = Session::list()?;
            if rows.is_empty() {
                let dir = sessions_dir().ok();
                eprintln!(
                    "no saved sessions{}",
                    dir.map(|d| format!(" in {}", d.display()))
                        .unwrap_or_default()
                );
                return Ok(ExitCode::SUCCESS);
            }
            println!(
                "{:<24} {:<10} {:>8}  updated_at",
                "NAME", "STATUS", "MATCHES"
            );
            for row in rows {
                println!(
                    "{:<24} {:<10} {:>8}  {}",
                    row.name,
                    status_label(row.status),
                    row.matches_total,
                    row.updated_at,
                );
            }
            Ok(ExitCode::SUCCESS)
        }

        SessionCmd::Save {
            name,
            algo,
            hashes,
            wordlist,
            mask,
            gpu,
            gpu_batch,
            gpu_workgroup,
        } => {
            let cfg = build_config(
                algo,
                hashes,
                wordlist,
                mask,
                gpu,
                gpu_batch,
                gpu_workgroup,
                Some(name.clone()),
            )?;
            let mut session = Session::new_saved(&name, cfg)?;
            let path = session.save()?;
            println!("session saved: {}", path.display());
            Ok(ExitCode::SUCCESS)
        }

        SessionCmd::Load {
            name,
            i_own_these_hashes,
            json,
        } => {
            if !i_own_these_hashes {
                bail!(
                    "Refusing to run a saved session without --i-own-these-hashes. \
                     See docs/ETHICS.md for why this flag exists."
                );
            }
            let session = Session::load(&name)?;
            let mut cfg = session.config;
            // Make sure auto-save writes back to the same session file.
            cfg.session_name = Some(name);
            run_attack(cfg, json).await
        }

        SessionCmd::Show { name } => {
            let session = Session::load(&name)?;
            let json = serde_json::to_string_pretty(&session)?;
            println!("{json}");
            Ok(ExitCode::SUCCESS)
        }

        SessionCmd::Delete { name } => {
            let removed = Session::delete(&name)?;
            if removed {
                eprintln!("deleted session: {name}");
            } else {
                eprintln!("no such session: {name}");
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn status_label(s: SessionStatus) -> &'static str {
    match s {
        SessionStatus::Saved => "saved",
        SessionStatus::Finished => "finished",
        SessionStatus::Error => "error",
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
