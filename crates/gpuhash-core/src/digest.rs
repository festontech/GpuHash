//! CPU reference implementations of the supported hash algorithms.
//!
//! These are the *correctness baseline*: the GPU shaders added in Phase 3 must agree
//! with these byte-for-byte. The RFC 1321 / NIST test vectors live as inline tests so
//! `cargo test -p gpuhash-core` catches regressions immediately.

use crate::{Algorithm, Error, Result};
use md5::{Digest as _, Md5};

/// Compute `algo`'s digest of `input` and return it as raw bytes.
pub fn digest(algo: Algorithm, input: &[u8]) -> Result<Vec<u8>> {
    match algo {
        Algorithm::Md5 => {
            let mut hasher = Md5::new();
            hasher.update(input);
            Ok(hasher.finalize().to_vec())
        }
        Algorithm::Sha1 => Err(Error::NotImplemented("sha1 (lands in Phase 5)")),
        Algorithm::Sha256 => Err(Error::NotImplemented("sha256 (lands in Phase 5)")),
    }
}

/// Hex-format a digest (lowercase, no separators). Useful in logs and tests.
pub fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 1321 §A.5 test suite.
    #[test]
    fn rfc1321_md5_vectors() {
        let cases = [
            ("", "d41d8cd98f00b204e9800998ecf8427e"),
            ("a", "0cc175b9c0f1b6a831c399e269772661"),
            ("abc", "900150983cd24fb0d6963f7d28e17f72"),
            ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            (
                "abcdefghijklmnopqrstuvwxyz",
                "c3fcd3d76192e4007dfb496cca67e13b",
            ),
            (
                "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
                "d174ab98d277d9f5a5611c2c9f419d9f",
            ),
            (
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "57edf4a22be3c955ac49da2e2107b67a",
            ),
        ];
        for (input, want) in cases {
            let got = digest(Algorithm::Md5, input.as_bytes()).unwrap();
            assert_eq!(to_hex(&got), want, "input = {input:?}");
        }
    }

    #[test]
    fn unsupported_algorithms_return_not_implemented() {
        assert!(matches!(
            digest(Algorithm::Sha1, b"x"),
            Err(Error::NotImplemented(_))
        ));
        assert!(matches!(
            digest(Algorithm::Sha256, b"x"),
            Err(Error::NotImplemented(_))
        ));
    }
}
