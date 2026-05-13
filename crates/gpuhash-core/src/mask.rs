//! Hashcat-style mask parsing and per-index candidate generation.
//!
//! Supported tokens in Phase 4:
//!   - `?l` — any lowercase letter (a–z, width 26)
//!   - `?u` — any uppercase letter (A–Z, width 26)
//!   - `?d` — any decimal digit (0–9, width 10)
//!   - any other ASCII byte — literal (width 1)
//!
//! Keyspace size = product of widths. The Phase-4 GPU kernel runs the
//! bruteforce index in u32, so keyspaces above `u32::MAX` (~4.3 B) are
//! refused; that covers e.g. `?l^7` (8 B). Lift later if needed.
//!
//! Position numbering: position 0 is the leftmost character of the mask
//! (highest order), the last position is least-significant. `candidate_at(0)`
//! is therefore "all positions take their charset's 0th character" — e.g.
//! `?d?d?d` at index 0 is `"000"`, at index 1 is `"001"`, at index 999 is
//! `"999"`.

use std::fmt;

/// One position in a mask: either a fixed charset class or a literal byte.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Position {
    Charset(CharsetKind),
    Literal(u8),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CharsetKind {
    /// 'a'..='z' (26)
    Lower,
    /// 'A'..='Z' (26)
    Upper,
    /// '0'..='9' (10)
    Digit,
}

impl CharsetKind {
    pub fn width(self) -> u32 {
        match self {
            CharsetKind::Lower | CharsetKind::Upper => 26,
            CharsetKind::Digit => 10,
        }
    }

    /// Byte at `c`-th index into the charset. Panics in debug if out of range.
    pub fn byte_at(self, c: u32) -> u8 {
        debug_assert!(c < self.width());
        match self {
            CharsetKind::Lower => b'a' + c as u8,
            CharsetKind::Upper => b'A' + c as u8,
            CharsetKind::Digit => b'0' + c as u8,
        }
    }
}

impl Position {
    pub fn width(self) -> u32 {
        match self {
            Position::Charset(c) => c.width(),
            Position::Literal(_) => 1,
        }
    }

    pub fn byte_at(self, c: u32) -> u8 {
        match self {
            Position::Charset(cs) => cs.byte_at(c),
            Position::Literal(b) => {
                debug_assert!(c == 0);
                b
            }
        }
    }
}

/// Parsed mask string.
#[derive(Clone, Debug)]
pub struct Mask {
    positions: Vec<Position>,
    /// Pre-computed total keyspace. Always `<= u32::MAX as u64` (validated).
    total: u64,
}

impl Mask {
    /// Parse a hashcat-style mask string. See module docs for supported tokens.
    pub fn parse(s: &str) -> Result<Self, String> {
        let bytes = s.as_bytes();
        let mut positions: Vec<Position> = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            if b == b'?' {
                if i + 1 >= bytes.len() {
                    return Err("dangling '?' at end of mask".into());
                }
                let kind = match bytes[i + 1] {
                    b'l' => CharsetKind::Lower,
                    b'u' => CharsetKind::Upper,
                    b'd' => CharsetKind::Digit,
                    other => {
                        return Err(format!(
                            "unsupported mask token `?{}`. Phase 4 supports `?l`, `?u`, `?d`",
                            other as char
                        ));
                    }
                };
                positions.push(Position::Charset(kind));
                i += 2;
            } else if !b.is_ascii() {
                return Err(format!(
                    "non-ASCII byte 0x{b:02x} at column {} not supported",
                    i + 1
                ));
            } else {
                positions.push(Position::Literal(b));
                i += 1;
            }
        }

        if positions.is_empty() {
            return Err("mask must have at least one position".into());
        }
        if positions.len() > MAX_MASK_POSITIONS {
            return Err(format!(
                "mask too long: {} positions, max is {MAX_MASK_POSITIONS}",
                positions.len()
            ));
        }

        // Compute total. Use checked u128 math so we can detect overflow past
        // u32::MAX cleanly.
        let mut total: u128 = 1;
        for p in &positions {
            total = total
                .checked_mul(p.width() as u128)
                .ok_or_else(|| "mask keyspace overflows u128 (??!)".to_string())?;
        }
        if total > u32::MAX as u128 {
            return Err(format!(
                "mask keyspace {total} exceeds u32::MAX; Phase 4 GPU kernel uses a 32-bit \
                 candidate index. Shorten the mask or lift this limit in a later phase.",
            ));
        }

        Ok(Self {
            positions,
            total: total as u64,
        })
    }

    pub fn positions(&self) -> &[Position] {
        &self.positions
    }

    pub fn num_positions(&self) -> usize {
        self.positions.len()
    }

    /// Total keyspace size. Always `<= u32::MAX as u64`.
    pub fn total(&self) -> u64 {
        self.total
    }

    /// Materialize the candidate at index `index` (must be `< total()`).
    /// Returns the candidate as raw bytes.
    pub fn candidate_at(&self, index: u64) -> Vec<u8> {
        debug_assert!(index < self.total);
        let mut out = vec![0u8; self.positions.len()];
        let mut remaining = index;
        for p in (0..self.positions.len()).rev() {
            let pos = self.positions[p];
            let w = pos.width() as u64;
            let c = (remaining % w) as u32;
            remaining /= w;
            out[p] = pos.byte_at(c);
        }
        out
    }
}

impl fmt::Display for Mask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for p in &self.positions {
            match p {
                Position::Charset(CharsetKind::Lower) => f.write_str("?l")?,
                Position::Charset(CharsetKind::Upper) => f.write_str("?u")?,
                Position::Charset(CharsetKind::Digit) => f.write_str("?d")?,
                Position::Literal(b) => f.write_str(&(*b as char).to_string())?,
            }
        }
        Ok(())
    }
}

/// Maximum number of positions in a mask. Matches the GPU shader's fixed-size
/// `MaskPos` array.
pub const MAX_MASK_POSITIONS: usize = 16;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_tokens() {
        let m = Mask::parse("?l?u?d").unwrap();
        assert_eq!(m.num_positions(), 3);
        assert_eq!(m.total(), 26 * 26 * 10);
    }

    #[test]
    fn parses_literals() {
        let m = Mask::parse("x?l").unwrap();
        assert_eq!(m.positions()[0], Position::Literal(b'x'));
        assert_eq!(m.positions()[1], Position::Charset(CharsetKind::Lower));
        assert_eq!(m.total(), 26);
    }

    #[test]
    fn three_digits_walks_keyspace_in_order() {
        let m = Mask::parse("?d?d?d").unwrap();
        assert_eq!(m.candidate_at(0), b"000");
        assert_eq!(m.candidate_at(1), b"001");
        assert_eq!(m.candidate_at(10), b"010");
        assert_eq!(m.candidate_at(100), b"100");
        assert_eq!(m.candidate_at(999), b"999");
    }

    #[test]
    fn lowercase_at_index_zero_is_a() {
        let m = Mask::parse("?l?l").unwrap();
        assert_eq!(m.candidate_at(0), b"aa");
        assert_eq!(m.candidate_at(1), b"ab");
        assert_eq!(m.candidate_at(25), b"az");
        assert_eq!(m.candidate_at(26), b"ba");
    }

    #[test]
    fn literal_then_digit_only_charset_varies() {
        let m = Mask::parse("x?d").unwrap();
        assert_eq!(m.candidate_at(0), b"x0");
        assert_eq!(m.candidate_at(9), b"x9");
        assert_eq!(m.total(), 10);
    }

    #[test]
    fn unsupported_token_is_rejected() {
        assert!(Mask::parse("?z").is_err());
        assert!(Mask::parse("?s").is_err());
    }

    #[test]
    fn dangling_question_mark_is_rejected() {
        assert!(Mask::parse("?l?").is_err());
        assert!(Mask::parse("?").is_err());
    }

    #[test]
    fn empty_mask_is_rejected() {
        assert!(Mask::parse("").is_err());
    }

    #[test]
    fn oversize_keyspace_is_rejected() {
        // ?l^7 = 8 031 810 176 > u32::MAX = 4 294 967 295
        assert!(Mask::parse("?l?l?l?l?l?l?l").is_err());
        // ?l^6 = 308 915 776 — fine.
        assert!(Mask::parse("?l?l?l?l?l?l").is_ok());
    }

    #[test]
    fn display_round_trips() {
        for s in ["?l?l?l", "?u?d", "x?l", "abc?d"] {
            let m = Mask::parse(s).unwrap();
            assert_eq!(format!("{m}"), s);
        }
    }
}
