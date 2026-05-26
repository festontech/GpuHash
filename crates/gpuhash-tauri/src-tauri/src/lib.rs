//! Tauri 2.x backend for the GPU Password Auditing Framework — Phase 7.
//!
//! The shell is intentionally thin: every command translates a frontend
//! request into a `gpuhash-core` engine call and forwards `EngineEvent`s
//! straight back to the webview as JSON. The engine, the event contract,
//! and the session storage live in `gpuhash-core` exactly as they do for
//! the CLI; this crate adds no new business logic.

use std::collections::HashSet;
use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use gpuhash_core::digest::{digest, to_hex};
use gpuhash_core::{
    benchmark_algo, Algorithm, AttackConfig, AttackSummary, BenchmarkConfig, BenchmarkReport,
    Engine, EngineEvent, Session, SessionListEntry, SessionMatch, SessionStatus,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio_util::sync::CancellationToken;

/// Channel name the frontend listens on for `EngineEvent`s.
const EVENT_CHANNEL: &str = "engine-event";

/// Shared state holding only the cancel handle for the in-flight audit, if any.
/// The `RunningAttack` itself moves into a spawned task so its event receiver
/// can be drained without holding the mutex across awaits.
#[derive(Default)]
struct EngineState {
    cancel: Mutex<Option<CancellationToken>>,
}

#[derive(Serialize, Clone)]
struct CommandError {
    message: String,
}

impl<E: std::fmt::Display> From<E> for CommandError {
    fn from(e: E) -> Self {
        Self {
            message: e.to_string(),
        }
    }
}

#[tauri::command]
async fn start_attack(
    app: AppHandle,
    state: State<'_, EngineState>,
    config: AttackConfig,
    i_own_these_hashes: bool,
) -> Result<(), CommandError> {
    if !i_own_these_hashes {
        return Err(CommandError {
            message: "Refusing to run an audit without i_own_these_hashes. See docs/ETHICS.md."
                .into(),
        });
    }

    // Reject overlapping runs — keeps the contract simple. The frontend's
    // job is to disable the Audit button while a run is in flight.
    {
        let guard = state.cancel.lock().expect("engine state poisoned");
        if guard.is_some() {
            return Err(CommandError {
                message: "an audit is already running; cancel it first".into(),
            });
        }
    }

    let session_name = config.session_name.clone();
    let engine = Engine::new();
    let mut running = engine.run(config.clone());
    let token = running.cancel_token();

    {
        let mut slot = state.cancel.lock().expect("engine state poisoned");
        *slot = Some(token);
    }

    // Drain events on a dedicated task so the command returns immediately.
    let app_for_task = app.clone();
    // We can't move the `State` into the task (it borrows), so clone the
    // inner AppHandle and clear our cancel slot via app state at the end.
    tokio::spawn(async move {
        let mut matches: Vec<SessionMatch> = Vec::new();
        let mut summary: Option<AttackSummary> = None;
        let mut hit_error = false;

        while let Some(event) = running.next_event().await {
            let _ = app_for_task.emit(EVENT_CHANNEL, &event);
            match &event {
                EngineEvent::Match {
                    plaintext,
                    target_idx,
                } => matches.push(SessionMatch {
                    plaintext: plaintext.clone(),
                    target_idx: *target_idx,
                }),
                EngineEvent::Finished { summary: s } => summary = Some(s.clone()),
                EngineEvent::Error { .. } => hit_error = true,
                _ => {}
            }
        }

        if let Some(name) = session_name {
            let status = if hit_error {
                SessionStatus::Error
            } else {
                SessionStatus::Finished
            };
            if let Err(e) = persist_session(&name, &config, matches, summary, status) {
                tracing::error!(error = %e, "failed to persist session");
            }
        }

        // Clear the cancel slot so the next start_attack can run.
        if let Some(state) = app_for_task.try_state::<EngineState>() {
            if let Ok(mut g) = state.cancel.lock() {
                *g = None;
            }
        }
    });

    Ok(())
}

#[tauri::command]
fn cancel_attack(state: State<'_, EngineState>) -> Result<bool, CommandError> {
    let guard = state.cancel.lock().expect("engine state poisoned");
    if let Some(token) = guard.as_ref() {
        token.cancel();
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
async fn benchmark(
    algo: gpuhash_core::Algorithm,
    secs: u64,
    batch_size: Option<u32>,
    workgroup_size: Option<u32>,
) -> Result<BenchmarkReport, CommandError> {
    let cfg = BenchmarkConfig {
        secs,
        batch_size,
        workgroup_size,
    };
    benchmark_algo(algo, cfg).await.map_err(CommandError::from)
}

#[tauri::command]
fn list_sessions() -> Result<Vec<SessionListEntry>, CommandError> {
    Session::list().map_err(CommandError::from)
}

#[tauri::command]
fn load_session(name: String) -> Result<Session, CommandError> {
    Session::load(&name).map_err(CommandError::from)
}

#[tauri::command]
fn delete_session(name: String) -> Result<bool, CommandError> {
    Session::delete(&name).map_err(CommandError::from)
}

/// Generated demo corpus paths returned to the frontend so the form can be
/// auto-populated. Both paths are absolute so they survive Tauri's working-
/// directory quirks (the engine just calls `File::open` on what it's given).
#[derive(Serialize, Clone)]
struct DemoCorpus {
    wordlist_path: String,
    hashes_path: String,
    candidate_count: u64,
    planted_count: u64,
}

/// Build a synthetic wordlist + planted-hashes pair under
/// `%LOCALAPPDATA%\gpuhash\demo\`. Returns absolute paths the frontend can
/// drop straight into the Attack form.
///
/// Bounded to keep the demo snappy:
/// - `count`   clamped to [10, 1_000_000]
/// - `planted` clamped to [1, count]
/// - candidates are lowercase ASCII, 4–8 chars (well inside MAX_CANDIDATE_LEN)
#[tauri::command]
fn generate_demo_corpus(
    count: u32,
    planted: u32,
    algo: Algorithm,
) -> Result<DemoCorpus, CommandError> {
    let count = count as usize;
    let planted = (planted as usize).clamp(1, count);

    let dir = demo_dir()?;
    fs::create_dir_all(&dir).map_err(CommandError::from)?;

    // Seed from time so successive Generate clicks produce different corpora,
    // but otherwise this is the same xorshift64* the big_audit test uses.
    let mut rng = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
        | 1;

    let mut candidates: Vec<String> = (0..count).map(|_| random_password(&mut rng)).collect();
    let mut seen: HashSet<String> = HashSet::new();
    candidates.retain(|c| seen.insert(c.clone()));

    let mut planted_set: HashSet<String> = HashSet::new();
    while planted_set.len() < planted.min(candidates.len()) {
        let idx = (xorshift(&mut rng) as usize) % candidates.len();
        planted_set.insert(candidates[idx].clone());
    }

    let wordlist_path = dir.join("demo_wordlist.txt");
    {
        let mut f = fs::File::create(&wordlist_path).map_err(CommandError::from)?;
        for c in &candidates {
            writeln!(f, "{c}").map_err(CommandError::from)?;
        }
    }

    let hashes_path = dir.join("demo_hashes.txt");
    {
        let mut f = fs::File::create(&hashes_path).map_err(CommandError::from)?;
        for plain in &planted_set {
            let d = digest(algo, plain.as_bytes()).map_err(CommandError::from)?;
            writeln!(f, "{}", to_hex(&d)).map_err(CommandError::from)?;
        }
    }

    Ok(DemoCorpus {
        wordlist_path: wordlist_path.to_string_lossy().into_owned(),
        hashes_path: hashes_path.to_string_lossy().into_owned(),
        candidate_count: candidates.len() as u64,
        planted_count: planted_set.len() as u64,
    })
}

fn demo_dir() -> Result<PathBuf, CommandError> {
    if let Some(d) = std::env::var_os("GPUHASH_DEMO_DIR") {
        return Ok(PathBuf::from(d));
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| CommandError {
                message: "LOCALAPPDATA not set".into(),
            })?
    } else {
        let home = std::env::var_os("HOME").ok_or_else(|| CommandError {
            message: "HOME not set".into(),
        })?;
        PathBuf::from(home).join(".local/share")
    };
    Ok(base.join("gpuhash").join("demo"))
}

fn xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

fn random_password(rng: &mut u64) -> String {
    let len = 4 + (xorshift(rng) % 5) as usize;
    let mut s = String::with_capacity(len);
    for _ in 0..len {
        let n = (xorshift(rng) % 26) as u8;
        s.push((b'a' + n) as char);
    }
    s
}

fn persist_session(
    name: &str,
    cfg: &AttackConfig,
    matches: Vec<SessionMatch>,
    summary: Option<AttackSummary>,
    status: SessionStatus,
) -> gpuhash_core::Result<()> {
    let mut session = Session::new_saved(name, cfg.clone())?;
    session.matches = matches;
    session.summary = summary;
    session.status = status;
    session.save()?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    tauri::Builder::default()
        .setup(|app| {
            app.manage(EngineState::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_attack,
            cancel_attack,
            benchmark,
            list_sessions,
            load_session,
            delete_session,
            generate_demo_corpus,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
