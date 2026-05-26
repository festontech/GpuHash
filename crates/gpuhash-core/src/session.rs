//! Persistent session storage (Phase 6).
//!
//! A `Session` captures a named audit: the `AttackConfig` that defined it, any
//! matches that were found, and the final `AttackSummary`. Sessions are stored
//! as JSON files under a per-user directory so the CLI's `session list/save/
//! load/delete` subcommands have somewhere to point at.
//!
//! Sessions are a CLI/UI concern — the engine itself never reads or writes
//! them. The CLI collects `EngineEvent`s from a run and calls [`Session::save`]
//! once at the end; `session load NAME` reads the file back and replays the
//! stored `AttackConfig` through the engine.
//!
//! # Storage layout
//!
//! Files live under:
//!
//! - **Windows** (the project's target): `%LOCALAPPDATA%\gpuhash\sessions\`
//! - **macOS**: `$HOME/Library/Application Support/gpuhash/sessions/`
//! - **Linux**: `$XDG_DATA_HOME/gpuhash/sessions/` or
//!   `$HOME/.local/share/gpuhash/sessions/`
//!
//! Override with the `GPUHASH_SESSIONS_DIR` environment variable (used by tests).
//!
//! Each session is one file: `<sessions_dir>/<name>.session.json`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::AttackConfig;
use crate::event::AttackSummary;
use crate::{Error, Result};

const FILE_SUFFIX: &str = ".session.json";

/// One stored audit. Public so the Tauri shell can later read the same shape.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    /// Logical name, used to derive the file name on disk.
    pub name: String,
    /// Status at the time of the last save.
    pub status: SessionStatus,
    /// The configuration the audit was run with (or was set up with, for
    /// `Saved` sessions that haven't been executed yet).
    pub config: AttackConfig,
    /// Matches found during the run. Empty for `Saved` sessions.
    #[serde(default)]
    pub matches: Vec<SessionMatch>,
    /// Final summary, present once the engine has emitted `Finished`.
    #[serde(default)]
    pub summary: Option<AttackSummary>,
    /// Unix seconds at first save.
    pub created_at: u64,
    /// Unix seconds at most recent save.
    pub updated_at: u64,
}

/// Lifecycle state of a stored session.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// Configured but never executed (`session save`).
    Saved,
    /// The engine ran to completion (`Finished` event).
    Finished,
    /// The engine reported an error.
    Error,
}

/// One row in `Session::matches`. Mirrors `EngineEvent::Match` but storable.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionMatch {
    pub plaintext: String,
    pub target_idx: u32,
}

/// Lightweight row returned from [`Session::list`] — avoids deserializing
/// every match into memory just to print a directory listing.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionListEntry {
    pub name: String,
    pub status: SessionStatus,
    pub updated_at: u64,
    pub matches_total: u64,
}

impl Session {
    /// Build a `Saved` session from an `AttackConfig`, without executing it.
    pub fn new_saved(name: impl Into<String>, config: AttackConfig) -> Result<Self> {
        let name = name.into();
        validate_name(&name)?;
        let now = unix_now();
        Ok(Self {
            name,
            status: SessionStatus::Saved,
            config,
            matches: Vec::new(),
            summary: None,
            created_at: now,
            updated_at: now,
        })
    }

    /// Path this session lives at (or would live at) on disk.
    pub fn path(&self) -> Result<PathBuf> {
        session_path(&self.name)
    }

    /// Write the session to disk, creating the sessions directory if needed.
    /// Updates `updated_at` and preserves `created_at` if a previous version
    /// of this session already exists.
    pub fn save(&mut self) -> Result<PathBuf> {
        validate_name(&self.name)?;
        let path = session_path(&self.name)?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        if path.exists() {
            // Preserve original creation timestamp if we're overwriting.
            if let Ok(prior) = read_session(&path) {
                self.created_at = prior.created_at;
            }
        }
        self.updated_at = unix_now();

        let json = serde_json::to_string_pretty(self)
            .map_err(|e| Error::BadFormat(format!("serialize session: {e}")))?;
        fs::write(&path, json)?;
        Ok(path)
    }

    /// Load a session by name from the sessions directory.
    pub fn load(name: &str) -> Result<Self> {
        validate_name(name)?;
        let path = session_path(name)?;
        read_session(&path)
    }

    /// Delete the on-disk session file. Returns `Ok(false)` if it didn't
    /// exist (so callers can treat delete as idempotent).
    pub fn delete(name: &str) -> Result<bool> {
        validate_name(name)?;
        let path = session_path(name)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(Error::Io(e)),
        }
    }

    /// List every session file in the sessions directory. Returns rows
    /// sorted by `updated_at` (newest first).
    pub fn list() -> Result<Vec<SessionListEntry>> {
        let dir = sessions_dir()?;
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut out: Vec<SessionListEntry> = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let Some(name) = file_stem_session_name(&path) else {
                continue;
            };
            // Don't let one corrupt file kill the whole listing.
            match read_session(&path) {
                Ok(s) => out.push(SessionListEntry {
                    name,
                    status: s.status,
                    updated_at: s.updated_at,
                    matches_total: s.matches.len() as u64,
                }),
                Err(e) => {
                    tracing::warn!(file = %path.display(), error = %e, "skipping unreadable session");
                }
            }
        }
        out.sort_by_key(|row| std::cmp::Reverse(row.updated_at));
        Ok(out)
    }
}

/// Directory sessions are stored in. Honors `GPUHASH_SESSIONS_DIR` (used by
/// tests) before falling back to platform conventions.
pub fn sessions_dir() -> Result<PathBuf> {
    if let Some(override_dir) = std::env::var_os("GPUHASH_SESSIONS_DIR") {
        return Ok(PathBuf::from(override_dir));
    }

    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| Error::BadFormat("LOCALAPPDATA not set".into()))?
    } else if cfg!(target_os = "macos") {
        let home =
            std::env::var_os("HOME").ok_or_else(|| Error::BadFormat("HOME not set".into()))?;
        PathBuf::from(home).join("Library/Application Support")
    } else {
        if let Some(d) = std::env::var_os("XDG_DATA_HOME") {
            PathBuf::from(d)
        } else {
            let home =
                std::env::var_os("HOME").ok_or_else(|| Error::BadFormat("HOME not set".into()))?;
            PathBuf::from(home).join(".local/share")
        }
    };
    Ok(base.join("gpuhash").join("sessions"))
}

fn session_path(name: &str) -> Result<PathBuf> {
    validate_name(name)?;
    Ok(sessions_dir()?.join(format!("{name}{FILE_SUFFIX}")))
}

fn read_session(path: &Path) -> Result<Session> {
    let raw = fs::read_to_string(path)?;
    serde_json::from_str(&raw)
        .map_err(|e| Error::BadFormat(format!("parse session {}: {e}", path.display())))
}

fn file_stem_session_name(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    name.strip_suffix(FILE_SUFFIX).map(|s| s.to_owned())
}

/// Reject names that would escape the sessions directory or collide with
/// reserved Windows characters. Session names are user-supplied identifiers,
/// not free-form paths.
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::BadFormat("session name must not be empty".into()));
    }
    if name.len() > 64 {
        return Err(Error::BadFormat(
            "session name must be 64 characters or fewer".into(),
        ));
    }
    for ch in name.chars() {
        let ok = ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.');
        if !ok {
            return Err(Error::BadFormat(format!(
                "session name contains invalid character {ch:?}; allowed: a-z A-Z 0-9 - _ ."
            )));
        }
    }
    if name.starts_with('.') || name == ".." {
        return Err(Error::BadFormat(
            "session name must not start with '.'".into(),
        ));
    }
    Ok(())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AttackMode, Backend, GpuTuning};
    use crate::hash::Algorithm;
    use std::sync::Mutex;

    // Tests share the same process-wide env var, so serialize them.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_tempdir<F: FnOnce(&Path)>(test: F) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = std::env::temp_dir().join(format!(
            "gpuhash-session-test-{}-{}",
            std::process::id(),
            unix_now()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let prev = std::env::var_os("GPUHASH_SESSIONS_DIR");
        std::env::set_var("GPUHASH_SESSIONS_DIR", &tmp);

        // Run the test; restore env even on panic by catching it.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| test(&tmp)));

        match prev {
            Some(v) => std::env::set_var("GPUHASH_SESSIONS_DIR", v),
            None => std::env::remove_var("GPUHASH_SESSIONS_DIR"),
        }
        let _ = std::fs::remove_dir_all(&tmp);
        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
    }

    fn sample_config() -> AttackConfig {
        AttackConfig {
            algo: Algorithm::Md5,
            hashes_path: PathBuf::from("examples/sample_hashes.txt"),
            mode: AttackMode::Dictionary {
                wordlist: PathBuf::from("examples/tiny_dict.txt"),
            },
            backend: Backend::Cpu,
            gpu_tuning: GpuTuning::default(),
            session_name: Some("unit-test".into()),
        }
    }

    #[test]
    fn save_and_load_roundtrip() {
        with_tempdir(|_dir| {
            let mut s = Session::new_saved("alpha", sample_config()).unwrap();
            s.matches.push(SessionMatch {
                plaintext: "hunter2".into(),
                target_idx: 0,
            });
            let path = s.save().unwrap();
            assert!(path.exists());

            let loaded = Session::load("alpha").unwrap();
            assert_eq!(loaded.name, "alpha");
            assert_eq!(loaded.matches.len(), 1);
            assert_eq!(loaded.matches[0].plaintext, "hunter2");
            assert_eq!(loaded.status, SessionStatus::Saved);
        });
    }

    #[test]
    fn list_returns_saved_sessions_newest_first() {
        with_tempdir(|_dir| {
            let mut a = Session::new_saved("a", sample_config()).unwrap();
            a.save().unwrap();
            // Force differing updated_at: write b second, then re-save a to put it later.
            std::thread::sleep(std::time::Duration::from_millis(1100));
            let mut b = Session::new_saved("b", sample_config()).unwrap();
            b.save().unwrap();

            let rows = Session::list().unwrap();
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].name, "b");
            assert_eq!(rows[1].name, "a");
        });
    }

    #[test]
    fn delete_is_idempotent() {
        with_tempdir(|_dir| {
            let mut s = Session::new_saved("gone", sample_config()).unwrap();
            s.save().unwrap();
            assert!(Session::delete("gone").unwrap());
            assert!(!Session::delete("gone").unwrap());
            assert!(Session::load("gone").is_err());
        });
    }

    #[test]
    fn rejects_path_traversal_and_separators() {
        with_tempdir(|_dir| {
            for bad in ["..", "../etc/passwd", "foo/bar", "foo\\bar", "", ".hidden"] {
                let err = Session::new_saved(bad, sample_config()).err();
                assert!(
                    err.is_some(),
                    "expected `{bad}` to be rejected, but Session::new_saved accepted it"
                );
            }
        });
    }

    #[test]
    fn list_is_empty_when_dir_missing() {
        with_tempdir(|dir| {
            // Remove the dir we just created — list() should still succeed.
            std::fs::remove_dir_all(dir).unwrap();
            let rows = Session::list().unwrap();
            assert!(rows.is_empty());
        });
    }

    #[test]
    fn save_preserves_created_at_across_overwrites() {
        with_tempdir(|_dir| {
            let mut s = Session::new_saved("persist", sample_config()).unwrap();
            s.save().unwrap();
            let first_created = s.created_at;
            std::thread::sleep(std::time::Duration::from_millis(1100));

            let mut again = Session::new_saved("persist", sample_config()).unwrap();
            again.save().unwrap();
            assert_eq!(again.created_at, first_created);
            assert!(again.updated_at > first_created);
        });
    }
}
