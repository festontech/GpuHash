//! GPU-side buffer types shared with `shaders/md5.wgsl`.
//!
//! Layout discipline: every `#[repr(C)]` here must match the WGSL struct of the
//! same name. WGSL storage-buffer "default" layout uses natural alignment (4-byte
//! for u32, no extra rounding), which matches `#[repr(C)]` with `u32`/`[u32; _]`
//! fields. Uniform buffers (here: `Params`) additionally require a total size that
//! is a multiple of 16; the explicit `_pad` keeps that invariant visible.

use bytemuck::{Pod, Zeroable};

/// Maximum candidate length supported by the single-block MD5 kernel.
///
/// MD5 processes a 64-byte block. The padding rules consume 9 bytes (the 0x80
/// marker + 8 bytes of bit-length), leaving 55 bytes for input. The kernel
/// assumes the host upholds this; longer candidates must be rejected before
/// packing (or, in a later phase, dispatched through a multi-block kernel).
pub const MAX_CANDIDATE_LEN: usize = 55;

/// One candidate slot in the GPU candidate buffer.
///
/// `bytes` holds the input little-endian–packed into 14 `u32` words (56 bytes of
/// capacity, of which at most `MAX_CANDIDATE_LEN` is meaningful — see the WGSL).
/// The shader appends the 0x80 byte and the bit-length itself.
#[repr(C)]
#[derive(Pod, Zeroable, Clone, Copy, Debug, Default)]
pub struct CandidateSlot {
    pub len: u32,
    pub bytes: [u32; 14],
}

impl CandidateSlot {
    /// Pack a byte slice into a slot. Returns `None` if `input.len() > MAX_CANDIDATE_LEN`.
    pub fn pack(input: &[u8]) -> Option<Self> {
        if input.len() > MAX_CANDIDATE_LEN {
            return None;
        }
        let mut bytes = [0u32; 14];
        // Pack bytes little-endian into u32 words; trailing positions stay zero.
        for (i, &b) in input.iter().enumerate() {
            let word = i / 4;
            let shift = (i % 4) * 8;
            bytes[word] |= (b as u32) << shift;
        }
        Some(Self {
            len: input.len() as u32,
            bytes,
        })
    }
}

/// Uniform-buffer parameters consumed by the kernel each dispatch.
#[repr(C)]
#[derive(Pod, Zeroable, Clone, Copy, Debug)]
pub struct Params {
    pub num_candidates: u32,
    pub num_targets: u32,
    pub max_matches: u32,
    pub _pad: u32,
}

/// Host-side view of one match record (decoded from the match buffer's `pairs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchRecord {
    pub candidate_idx: u32,
    pub target_idx: u32,
}

// ---- Bruteforce-kernel buffer types ----

/// One mask position on the GPU. Mirrors the WGSL `MaskPos` in
/// `shaders/md5_bruteforce.wgsl`.
///
/// `kind` is one of:
///   - 0 = literal byte (`value` holds the byte)
///   - 1 = lowercase a–z
///   - 2 = uppercase A–Z
///   - 3 = digit 0–9
#[repr(C)]
#[derive(Pod, Zeroable, Clone, Copy, Debug, Default)]
pub struct MaskPosGpu {
    pub kind: u32,
    pub value: u32,
}

impl MaskPosGpu {
    pub fn literal(b: u8) -> Self {
        Self {
            kind: 0,
            value: b as u32,
        }
    }
    pub fn lowercase() -> Self {
        Self { kind: 1, value: 0 }
    }
    pub fn uppercase() -> Self {
        Self { kind: 2, value: 0 }
    }
    pub fn digit() -> Self {
        Self { kind: 3, value: 0 }
    }
}

/// Uniform-buffer parameters for the bruteforce kernel.
#[repr(C)]
#[derive(Pod, Zeroable, Clone, Copy, Debug)]
pub struct BruteforceParams {
    pub num_positions: u32,
    pub num_candidates: u32,
    pub num_targets: u32,
    pub max_matches: u32,
    pub base_index: u32,
    pub _pad0: u32,
    pub _pad1: u32,
    pub _pad2: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_short_input_is_little_endian() {
        // "abc" → bytes 'a','b','c' → first u32 = 0x00636261 (little-endian).
        let slot = CandidateSlot::pack(b"abc").unwrap();
        assert_eq!(slot.len, 3);
        assert_eq!(slot.bytes[0], 0x0063_6261);
        assert_eq!(slot.bytes[1], 0);
    }

    #[test]
    fn pack_rejects_oversize() {
        let oversize = vec![b'x'; MAX_CANDIDATE_LEN + 1];
        assert!(CandidateSlot::pack(&oversize).is_none());
    }

    #[test]
    fn pack_max_length_is_accepted() {
        let max = vec![b'x'; MAX_CANDIDATE_LEN];
        assert!(CandidateSlot::pack(&max).is_some());
    }

    #[test]
    fn slot_size_matches_wgsl() {
        // len: u32 + bytes: [u32; 14] = 60 bytes. WGSL default storage layout
        // gives the same stride for `array<CandidateSlot>`.
        assert_eq!(std::mem::size_of::<CandidateSlot>(), 60);
        assert_eq!(std::mem::align_of::<CandidateSlot>(), 4);
    }

    #[test]
    fn params_is_uniform_safe() {
        // Uniform buffers require a size that's a multiple of 16.
        assert_eq!(std::mem::size_of::<Params>(), 16);
    }

    #[test]
    fn bruteforce_params_is_uniform_safe() {
        assert_eq!(std::mem::size_of::<BruteforceParams>(), 32);
    }

    #[test]
    fn mask_pos_gpu_size() {
        // u32 kind + u32 value = 8 bytes; WGSL `MaskPos` matches.
        assert_eq!(std::mem::size_of::<MaskPosGpu>(), 8);
    }
}
