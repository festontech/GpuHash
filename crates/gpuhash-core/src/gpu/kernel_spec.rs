//! Generic kernel-spec types and shared WGSL fragments.
//!
//! Per-algorithm constants (MD5_DICT_SPEC, SHA1_BRUTE_SPEC, ...) live in
//! `crate::gpu::algos`, one module per hash algorithm.
//!
//! Each spec names the WGSL fragments that get concatenated to form a complete
//! shader module:
//!
//!   dict   = common/match + common/dict       + <algo>/funcs + <algo>/dict
//!   brute  = common/match + common/bruteforce + <algo>/funcs + <algo>/bruteforce
//!
//! `common/match` (bindings 1 & 2, plus `rotl` / `byteswap`) is included in
//! every pipeline. `common/dict` and `common/bruteforce` carry mode-specific
//! bindings (binding 0 + binding 3) and shared helpers (the mask decomposition
//! lives in `common/bruteforce`). The per-algorithm `<algo>/funcs.wgsl` files
//! only hold what is actually algorithm-specific: K/S tables, the block
//! function, padding, and the digest-comparison scan_targets.

/// Byte order used to pack a target digest into the GPU's target buffer.
///
/// MD5 emits little-endian state words; SHA-1 / SHA-256 emit big-endian.
/// The host packs targets in the algorithm's natural order so the shader's
/// per-word comparison is a flat u32==u32.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Endianness {
    Little,
    Big,
}

/// Shared WGSL fragments included by every pipeline of the matching mode.
pub const MATCH_COMMON: &str = include_str!("shaders/common/match.wgsl");
pub const DICT_COMMON: &str = include_str!("shaders/common/dict.wgsl");
pub const BRUTE_COMMON: &str = include_str!("shaders/common/bruteforce.wgsl");

#[derive(Copy, Clone)]
pub struct DictKernelSpec {
    /// Per-algorithm function library (e.g. `algos::md5::FUNCS`).
    pub funcs: &'static str,
    /// Per-(algo, mode) entry point (e.g. `algos::md5::DICT_ENTRY`).
    pub entry: &'static str,
    pub entry_point: &'static str,
    pub pipeline_label: &'static str,
    pub digest_bytes: usize,
    pub target_words: u32,
    pub target_endian: Endianness,
}

impl DictKernelSpec {
    /// Concatenate WGSL fragments in dependency order: common scaffolding
    /// (bindings + helpers) → algorithm functions → entry point.
    pub fn assemble_shader(&self) -> String {
        format!(
            "{}\n{}\n{}\n{}",
            MATCH_COMMON, DICT_COMMON, self.funcs, self.entry,
        )
    }
}

#[derive(Copy, Clone)]
pub struct BruteforceKernelSpec {
    pub funcs: &'static str,
    pub entry: &'static str,
    pub entry_point: &'static str,
    pub pipeline_label: &'static str,
    pub digest_bytes: usize,
    pub target_words: u32,
    pub target_endian: Endianness,
}

impl BruteforceKernelSpec {
    pub fn assemble_shader(&self) -> String {
        format!(
            "{}\n{}\n{}\n{}",
            MATCH_COMMON, BRUTE_COMMON, self.funcs, self.entry,
        )
    }
}

/// Pack a target digest into the GPU's `array<u32>` target buffer per the
/// algorithm's endianness convention.
pub(crate) fn pack_target_words(
    targets: &[Vec<u8>],
    digest_bytes: usize,
    endian: Endianness,
) -> Vec<u32> {
    debug_assert_eq!(digest_bytes % 4, 0);
    let words_per_target = digest_bytes / 4;
    let mut out: Vec<u32> = Vec::with_capacity(targets.len() * words_per_target);
    for t in targets {
        debug_assert_eq!(t.len(), digest_bytes);
        for chunk in t.chunks_exact(4) {
            let bytes = [chunk[0], chunk[1], chunk[2], chunk[3]];
            let w = match endian {
                Endianness::Little => u32::from_le_bytes(bytes),
                Endianness::Big => u32::from_be_bytes(bytes),
            };
            out.push(w);
        }
    }
    out
}
