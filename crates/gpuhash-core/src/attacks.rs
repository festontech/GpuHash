//! Attack-mode implementations.
//!
//! Phase 1: dictionary attacks only. Brute-force lands in Phase 4.

use crate::{config::AttackMode, Error, Result};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// A producer of candidate plaintexts. The engine drives one of these to feed the
/// hashing core.
pub trait CandidateSource: Send {
    /// Yield the next candidate, or `Ok(None)` when the source is exhausted.
    fn next_candidate(&mut self) -> Result<Option<String>>;

    /// Cheap upper-bound estimate of the total candidate count, if the source can
    /// supply one. Used to compute ETA in `EngineEvent::Progress`.
    fn estimate_total(&self) -> Option<u64>;
}

pub struct WordlistSource {
    reader: BufReader<File>,
    /// Pre-counted line total. For multi-GB lists this should move to a background
    /// thread (Phase 4 follow-up).
    total: u64,
}

impl WordlistSource {
    pub fn open(path: &Path) -> Result<Self> {
        let total = BufReader::new(File::open(path)?).lines().count() as u64;
        let reader = BufReader::new(File::open(path)?);
        Ok(WordlistSource { reader, total })
    }
}

impl CandidateSource for WordlistSource {
    fn next_candidate(&mut self) -> Result<Option<String>> {
        let mut buf = String::new();
        match self.reader.read_line(&mut buf) {
            Ok(0) => Ok(None),
            Ok(_) => {
                while buf.ends_with('\n') || buf.ends_with('\r') {
                    buf.pop();
                }
                Ok(Some(buf))
            }
            Err(e) => Err(e.into()),
        }
    }

    fn estimate_total(&self) -> Option<u64> {
        Some(self.total)
    }
}

pub fn build_source(mode: &AttackMode) -> Result<Box<dyn CandidateSource>> {
    match mode {
        AttackMode::Dictionary { wordlist } => Ok(Box::new(WordlistSource::open(wordlist)?)),
        AttackMode::Bruteforce { .. } => Err(Error::NotImplemented("bruteforce (Phase 4)")),
    }
}
