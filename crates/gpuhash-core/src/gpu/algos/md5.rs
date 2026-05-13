//! MD5 GPU kernel spec.

use crate::gpu::kernel_spec::{BruteforceKernelSpec, DictKernelSpec, Endianness};

pub const FUNCS: &str = include_str!("../shaders/md5/funcs.wgsl");
pub const DICT_ENTRY: &str = include_str!("../shaders/md5/dict.wgsl");
pub const BRUTE_ENTRY: &str = include_str!("../shaders/md5/bruteforce.wgsl");

pub const DICT_SPEC: DictKernelSpec = DictKernelSpec {
    funcs: FUNCS,
    entry: DICT_ENTRY,
    entry_point: "md5_attack",
    pipeline_label: "md5-dict",
    digest_bytes: 16,
    target_words: 4,
    target_endian: Endianness::Little,
};

pub const BRUTE_SPEC: BruteforceKernelSpec = BruteforceKernelSpec {
    funcs: FUNCS,
    entry: BRUTE_ENTRY,
    entry_point: "md5_bruteforce",
    pipeline_label: "md5-brute",
    digest_bytes: 16,
    target_words: 4,
    target_endian: Endianness::Little,
};
