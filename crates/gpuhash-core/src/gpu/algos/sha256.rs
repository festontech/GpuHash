//! SHA-256 GPU kernel spec.

use crate::gpu::kernel_spec::{BruteforceKernelSpec, DictKernelSpec, Endianness};

pub const FUNCS: &str = include_str!("../shaders/sha256/funcs.wgsl");
pub const DICT_ENTRY: &str = include_str!("../shaders/sha256/dict.wgsl");
pub const BRUTE_ENTRY: &str = include_str!("../shaders/sha256/bruteforce.wgsl");

pub const DICT_SPEC: DictKernelSpec = DictKernelSpec {
    funcs: FUNCS,
    entry: DICT_ENTRY,
    entry_point: "sha256_attack",
    pipeline_label: "sha256-dict",
    digest_bytes: 32,
    target_words: 8,
    target_endian: Endianness::Big,
};

pub const BRUTE_SPEC: BruteforceKernelSpec = BruteforceKernelSpec {
    funcs: FUNCS,
    entry: BRUTE_ENTRY,
    entry_point: "sha256_bruteforce",
    pipeline_label: "sha256-brute",
    digest_bytes: 32,
    target_words: 8,
    target_endian: Endianness::Big,
};
