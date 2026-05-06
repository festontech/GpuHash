//! Parse hash target files (one hex-encoded digest per line).
//!
//! Format: one digest per line, optionally with `#` comments and blank lines.
//! Each digest must be exactly the hex length expected by the algorithm
//! (32 chars for MD5, 40 for SHA-1, 64 for SHA-256).

use crate::{Algorithm, Error, Result};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Clone, Debug)]
pub struct TargetSet {
    pub algo: Algorithm,
    pub hashes: Vec<Vec<u8>>,
}

pub fn load_targets(path: &Path, algo: Algorithm) -> Result<TargetSet> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let expected_bytes = algo.digest_bytes();

    let mut hashes = Vec::new();
    for (line_no, line_res) in reader.lines().enumerate() {
        let line = line_res?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let bytes = parse_hex(trimmed, expected_bytes)
            .map_err(|e| Error::BadFormat(format!("line {}: {}", line_no + 1, e)))?;
        hashes.push(bytes);
    }

    if hashes.is_empty() {
        return Err(Error::BadFormat("no targets loaded".into()));
    }

    Ok(TargetSet { algo, hashes })
}

fn parse_hex(s: &str, expected_bytes: usize) -> std::result::Result<Vec<u8>, String> {
    if s.len() != expected_bytes * 2 {
        return Err(format!(
            "expected {} hex chars, got {}",
            expected_bytes * 2,
            s.len()
        ));
    }
    (0..expected_bytes)
        .map(|i| {
            u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
                .map_err(|_| format!("invalid hex at column {}", i * 2 + 1))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn write_temp(name: &str, contents: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("gpuhash-loader-test-{}-{name}", std::process::id()));
        let mut f = File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        path
    }

    #[test]
    fn loads_md5_with_comments_and_blanks() {
        let path = write_temp(
            "md5",
            "# the canonical empty-string md5\n\
             d41d8cd98f00b204e9800998ecf8427e\n\
             \n\
             # md5 of \"a\"\n\
             0cc175b9c0f1b6a831c399e269772661\n",
        );
        let set = load_targets(&path, Algorithm::Md5).unwrap();
        assert_eq!(set.hashes.len(), 2);
        assert_eq!(set.hashes[0].len(), 16);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_wrong_length() {
        let path = write_temp("badlen", "deadbeef\n");
        let err = load_targets(&path, Algorithm::Md5).unwrap_err();
        assert!(matches!(err, Error::BadFormat(_)), "{err:?}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_invalid_hex() {
        let path = write_temp(
            "badhex",
            "ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ\n",
        );
        let err = load_targets(&path, Algorithm::Md5).unwrap_err();
        assert!(matches!(err, Error::BadFormat(_)));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_empty_file() {
        let path = write_temp("empty", "# only a comment\n\n");
        let err = load_targets(&path, Algorithm::Md5).unwrap_err();
        assert!(matches!(err, Error::BadFormat(_)));
        let _ = std::fs::remove_file(path);
    }
}
