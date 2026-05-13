//! SHA-1 GPU kernel spec.

use crate::gpu::kernel_spec::{BruteforceKernelSpec, DictKernelSpec, Endianness};

pub const FUNCS: &str = include_str!("../shaders/sha1/funcs.wgsl");
pub const DICT_ENTRY: &str = include_str!("../shaders/sha1/dict.wgsl");
pub const BRUTE_ENTRY: &str = include_str!("../shaders/sha1/bruteforce.wgsl");

pub const DICT_SPEC: DictKernelSpec = DictKernelSpec {
    funcs: FUNCS,
    entry: DICT_ENTRY,
    entry_point: "sha1_attack",
    pipeline_label: "sha1-dict",
    digest_bytes: 20,
    target_words: 5,
    target_endian: Endianness::Big,
};

pub const BRUTE_SPEC: BruteforceKernelSpec = BruteforceKernelSpec {
    funcs: FUNCS,
    entry: BRUTE_ENTRY,
    entry_point: "sha1_bruteforce",
    pipeline_label: "sha1-brute",
    digest_bytes: 20,
    target_words: 5,
    target_endian: Endianness::Big,
};
