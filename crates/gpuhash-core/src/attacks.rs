//! Attack-mode implementations.
//!
//! Phase 1 added the dictionary `WordlistSource`. Phase 4 adds a CPU
//! bruteforce `MaskSource` that walks the keyspace defined by a
//! hashcat-style mask string. The GPU bruteforce path doesn't go
//! through `CandidateSource` at all — see `gpu::bruteforce_runner`.

use crate::{config::AttackMode, mask::Mask, Error, Result};
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

/// Enumerates every candidate produced by walking a parsed `Mask` over
/// `[start, end)` of its keyspace. Used for the CPU bruteforce path.
pub struct MaskSource {
    mask: Mask,
    next: u64,
    end: u64,
}

impl MaskSource {
    pub fn new(mask: Mask, start: u64, end: Option<u64>) -> Result<Self> {
        let total = mask.total();
        let end = end.unwrap_or(total);
        if start > end {
            return Err(Error::BadFormat(format!(
                "mask range start={start} > end={end}"
            )));
        }
        if end > total {
            return Err(Error::BadFormat(format!(
                "mask range end={end} > keyspace size {total}"
            )));
        }
        Ok(Self {
            mask,
            next: start,
            end,
        })
    }

    /// Borrowed access to the underlying mask (e.g. for the GPU bruteforce path
    /// which needs the parsed mask directly).
    pub fn mask(&self) -> &Mask {
        &self.mask
    }
}

impl CandidateSource for MaskSource {
    fn next_candidate(&mut self) -> Result<Option<String>> {
        if self.next >= self.end {
            return Ok(None);
        }
        let bytes = self.mask.candidate_at(self.next);
        self.next += 1;
        // Mask candidates are pure ASCII by construction (supported tokens emit
        // ASCII bytes, literals are validated as ASCII at parse time), so this
        // never fails; bubble it as an io error for the unlikely defect.
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|e| Error::BadFormat(format!("mask emitted non-utf8 bytes: {e}")))
    }

    fn estimate_total(&self) -> Option<u64> {
        Some(self.end.saturating_sub(self.next))
    }
}

pub fn build_source(mode: &AttackMode) -> Result<Box<dyn CandidateSource>> {
    match mode {
        AttackMode::Dictionary { wordlist } => Ok(Box::new(WordlistSource::open(wordlist)?)),
        AttackMode::Bruteforce { mask, start, end } => {
            let parsed = Mask::parse(mask).map_err(Error::BadFormat)?;
            Ok(Box::new(MaskSource::new(parsed, *start, *end)?))
        }
    }
}
