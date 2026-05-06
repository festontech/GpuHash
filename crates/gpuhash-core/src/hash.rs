//! Supported hash algorithms.

use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Algorithm {
    Md5,
    Sha1,
    Sha256,
}

impl Algorithm {
    /// Number of bytes in the algorithm's raw digest.
    pub fn digest_bytes(self) -> usize {
        match self {
            Algorithm::Md5 => 16,
            Algorithm::Sha1 => 20,
            Algorithm::Sha256 => 32,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Algorithm::Md5 => "md5",
            Algorithm::Sha1 => "sha1",
            Algorithm::Sha256 => "sha256",
        }
    }
}

impl std::fmt::Display for Algorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl std::str::FromStr for Algorithm {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "md5" => Ok(Algorithm::Md5),
            "sha1" => Ok(Algorithm::Sha1),
            "sha256" => Ok(Algorithm::Sha256),
            other => Err(format!("unknown algorithm `{other}` — expected md5|sha1|sha256")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_algorithms() {
        assert_eq!("md5".parse::<Algorithm>().unwrap(), Algorithm::Md5);
        assert_eq!("SHA1".parse::<Algorithm>().unwrap(), Algorithm::Sha1);
        assert_eq!("Sha256".parse::<Algorithm>().unwrap(), Algorithm::Sha256);
        assert!("bcrypt".parse::<Algorithm>().is_err());
    }

    #[test]
    fn digest_sizes() {
        assert_eq!(Algorithm::Md5.digest_bytes(), 16);
        assert_eq!(Algorithm::Sha1.digest_bytes(), 20);
        assert_eq!(Algorithm::Sha256.digest_bytes(), 32);
    }
}
