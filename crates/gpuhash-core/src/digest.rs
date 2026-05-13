//! CPU reference implementations of the supported hash algorithms.
//!
//! These are the *correctness baseline*: the GPU shaders must agree with these
//! byte-for-byte. RFC 1321 (MD5) / NIST FIPS 180-4 (SHA-1, SHA-256) test vectors
//! live as inline tests so `cargo test -p gpuhash-core` catches regressions
//! immediately.

use crate::{Algorithm, Result};
use md5::{Digest as _, Md5};
use sha1::Sha1;
use sha2::Sha256;

/// Compute `algo`'s digest of `input` and return it as raw bytes.
pub fn digest(algo: Algorithm, input: &[u8]) -> Result<Vec<u8>> {
    Ok(match algo {
        Algorithm::Md5 => {
            let mut h = Md5::new();
            h.update(input);
            h.finalize().to_vec()
        }
        Algorithm::Sha1 => {
            let mut h = Sha1::new();
            h.update(input);
            h.finalize().to_vec()
        }
        Algorithm::Sha256 => {
            let mut h = Sha256::new();
            h.update(input);
            h.finalize().to_vec()
        }
    })
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

    /// NIST FIPS 180-4 / RFC 3174 SHA-1 test vectors.
    #[test]
    fn nist_sha1_vectors() {
        let cases = [
            ("", "da39a3ee5e6b4b0d3255bfef95601890afd80709"),
            ("a", "86f7e437faa5a7fce15d1ddcb9eaeaea377667b8"),
            ("abc", "a9993e364706816aba3e25717850c26c9cd0d89d"),
            (
                "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
                "84983e441c3bd26ebaae4aa1f95129e5e54670f1",
            ),
        ];
        for (input, want) in cases {
            let got = digest(Algorithm::Sha1, input.as_bytes()).unwrap();
            assert_eq!(to_hex(&got), want, "input = {input:?}");
        }
    }

    /// NIST FIPS 180-4 SHA-256 test vectors.
    #[test]
    fn nist_sha256_vectors() {
        let cases = [
            (
                "",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                "abc",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
                "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
            ),
        ];
        for (input, want) in cases {
            let got = digest(Algorithm::Sha256, input.as_bytes()).unwrap();
            assert_eq!(to_hex(&got), want, "input = {input:?}");
        }
    }
}
