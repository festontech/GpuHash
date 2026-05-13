//! Per-algorithm GPU kernel specs.
//!
//! Each submodule bundles:
//! - the algorithm's WGSL function library (`FUNCS`),
//! - the dict and bruteforce entry-point WGSL,
//! - the [`crate::gpu::kernel_spec::DictKernelSpec`] / [`BruteforceKernelSpec`]
//!   constants the runners consume.
//!
//! Adding a new algorithm = adding one file here, one folder under
//! `shaders/<algo>/`, and one match arm in `engine::run_gpu`. Nothing else in
//! the GPU stack is algorithm-aware.

pub mod md5;
pub mod sha1;
pub mod sha256;
