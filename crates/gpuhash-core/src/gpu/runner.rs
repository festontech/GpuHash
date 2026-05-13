//! `DictRunner` — owns the wgpu device, a dictionary-attack compute pipeline,
//! and per-slot persistent buffers for overlapped batched dispatches.
//!
//! Phase-4 introduced the slot/ring discipline; Phase 5 made the runner generic
//! over the hash algorithm via [`DictKernelSpec`]. The same Rust code drives
//! MD5, SHA-1, and SHA-256; only the shader source + target packing differ.
//!
//! Buffer reuse policy (see `docs/ARCHITECTURE.md` and `CLAUDE.md`):
//! - Pipeline + bind-group layout built once per runner.
//! - Per-slot storage buffers allocated once at the batch-size cap and reused.
//! - The targets buffer is shared across all slots (written once at construction).
//! - Only the small match-staging buffers are `MAP_READ`. Hot-path buffers never
//!   round-trip to the CPU.

use bytemuck;
use wgpu::util::DeviceExt;

use crate::gpu::buffers::{CandidateSlot, MatchRecord, Params};
use crate::gpu::kernel_spec::{pack_target_words, DictKernelSpec};
use crate::{Error, Result};

/// Phase-4 sweep on Intel UHD Graphics (Vulkan) at batch=1<<18 found:
///   wg=32  → 159 MH/s
///   wg=64  → 211 MH/s
///   wg=128 → 235 MH/s
///   wg=256 → 263 MH/s   ← chosen default
/// CLAUDE.md's a-priori recommendation was 32/64, which the data didn't support.
pub const DEFAULT_WORKGROUP_SIZE: u32 = 256;
pub const ALLOWED_WORKGROUP_SIZES: &[u32] = &[32, 64, 128, 256];

/// Default number of batches kept in flight on the queue. Two is sufficient to
/// keep an Intel iGPU's command queue non-empty across a readback wait without
/// inflating device-memory use.
pub const DEFAULT_MAX_IN_FLIGHT: usize = 2;

/// Generic dictionary-attack runner. The algorithm is selected by the
/// `DictKernelSpec` passed to [`DictRunner::new`].
pub struct DictRunner {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    // Held for ownership: every slot's bind group already references it, but
    // dropping the buffer here would needlessly invalidate the GPU resource on
    // some backends. Never read directly through the runner after construction.
    #[allow(dead_code)]
    targets_buf: wgpu::Buffer,
    slots: Vec<SlotBuffers>,

    batch_size: u32,
    workgroup_size: u32,
    max_matches: u32,
    num_targets: u32,
    match_buf_size: u64,
}

/// All per-slot device resources. Each slot is independently usable so two (or
/// more) dispatches can be in flight at once without aliasing buffers.
struct SlotBuffers {
    candidates_buf: wgpu::Buffer,
    match_buf: wgpu::Buffer,
    match_staging: wgpu::Buffer,
    params_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl DictRunner {
    /// Construct a runner targeting the host's preferred GPU adapter and
    /// pre-upload the target hashes.
    ///
    /// - `spec` — algorithm-specific shader and target-layout details.
    /// - `targets` — one digest per entry; each must be `spec.digest_bytes` bytes.
    /// - `batch_size` — maximum number of candidates per submitted batch.
    /// - `workgroup_size` — WGSL `@workgroup_size`; must be in
    ///   `ALLOWED_WORKGROUP_SIZES`.
    /// - `max_matches` — per-batch capacity of the match-record buffer.
    /// - `max_in_flight` — number of slot buffer sets to allocate (≥ 1).
    pub async fn new(
        spec: DictKernelSpec,
        targets: &[Vec<u8>],
        batch_size: u32,
        workgroup_size: u32,
        max_matches: u32,
        max_in_flight: usize,
    ) -> Result<Self> {
        if batch_size == 0 {
            return Err(Error::Gpu("batch_size must be > 0".into()));
        }
        if !ALLOWED_WORKGROUP_SIZES.contains(&workgroup_size) {
            return Err(Error::Gpu(format!(
                "workgroup_size {workgroup_size} not in {ALLOWED_WORKGROUP_SIZES:?}"
            )));
        }
        if max_matches == 0 {
            return Err(Error::Gpu("max_matches must be > 0".into()));
        }
        if max_in_flight == 0 {
            return Err(Error::Gpu("max_in_flight must be >= 1".into()));
        }
        if targets.is_empty() {
            return Err(Error::Gpu("at least one target hash required".into()));
        }
        for (i, t) in targets.iter().enumerate() {
            if t.len() != spec.digest_bytes {
                return Err(Error::Gpu(format!(
                    "target {i} is {} bytes, expected {} for {}",
                    t.len(),
                    spec.digest_bytes,
                    spec.pipeline_label,
                )));
            }
        }

        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok_or_else(|| Error::Gpu("no compatible GPU adapter found".into()))?;

        let info = adapter.get_info();
        tracing::info!(
            name = %info.name,
            backend = ?info.backend,
            driver_info = %info.driver_info,
            max_in_flight,
            batch_size,
            workgroup_size,
            label = spec.pipeline_label,
            "dict runner: adapter selected"
        );

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default(), None)
            .await
            .map_err(|e| Error::Gpu(format!("request_device: {e}")))?;

        // ---- pipeline ----
        // Assemble the shader from common + algo + entry, then patch the
        // workgroup size literal.
        let shader_src = spec.assemble_shader().replace(
            "@workgroup_size(64)",
            &format!("@workgroup_size({workgroup_size})"),
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(spec.pipeline_label),
            source: wgpu::ShaderSource::Wgsl(shader_src.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(spec.pipeline_label),
            layout: None,
            module: &module,
            entry_point: spec.entry_point,
            compilation_options: Default::default(),
            cache: None,
        });
        let bgl = pipeline.get_bind_group_layout(0);

        // ---- shared targets buffer ----
        let target_words = pack_target_words(targets, spec.digest_bytes, spec.target_endian);
        let targets_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{}-targets", spec.pipeline_label)),
            contents: bytemuck::cast_slice(&target_words),
            usage: wgpu::BufferUsages::STORAGE,
        });

        // ---- per-slot buffers ----
        let candidates_size = (batch_size as u64) * (std::mem::size_of::<CandidateSlot>() as u64);
        let match_buf_size: u64 = 16 + 8 * (max_matches as u64);

        let mut slots = Vec::with_capacity(max_in_flight);
        for slot_idx in 0..max_in_flight {
            let candidates_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("{}-cand-{slot_idx}", spec.pipeline_label)),
                size: candidates_size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let match_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("{}-match-{slot_idx}", spec.pipeline_label)),
                size: match_buf_size,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let match_staging = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("{}-staging-{slot_idx}", spec.pipeline_label)),
                size: match_buf_size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let params_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("{}-params-{slot_idx}", spec.pipeline_label)),
                size: std::mem::size_of::<Params>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("{}-bind-{slot_idx}", spec.pipeline_label)),
                layout: &bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: candidates_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: targets_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: match_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: params_buf.as_entire_binding(),
                    },
                ],
            });
            slots.push(SlotBuffers {
                candidates_buf,
                match_buf,
                match_staging,
                params_buf,
                bind_group,
            });
        }

        Ok(Self {
            device,
            queue,
            pipeline,
            targets_buf,
            slots,
            batch_size,
            workgroup_size,
            max_matches,
            num_targets: targets.len() as u32,
            match_buf_size,
        })
    }

    /// Submit one batch into `slot` without waiting for results.
    pub fn submit(&self, slot: usize, candidates: &[CandidateSlot]) -> Result<()> {
        if slot >= self.slots.len() {
            return Err(Error::Gpu(format!(
                "slot {slot} out of range (max_in_flight={})",
                self.slots.len()
            )));
        }
        if candidates.len() as u32 > self.batch_size {
            return Err(Error::Gpu(format!(
                "batch of {} exceeds runner capacity {}",
                candidates.len(),
                self.batch_size
            )));
        }
        if candidates.is_empty() {
            return Err(Error::Gpu("submit called with empty batch".into()));
        }
        let s = &self.slots[slot];

        self.queue
            .write_buffer(&s.candidates_buf, 0, bytemuck::cast_slice(candidates));

        let header_zero = [0u32; 4];
        self.queue
            .write_buffer(&s.match_buf, 0, bytemuck::cast_slice(&header_zero));

        let params = Params {
            num_candidates: candidates.len() as u32,
            num_targets: self.num_targets,
            max_matches: self.max_matches,
            _pad: 0,
        };
        self.queue
            .write_buffer(&s.params_buf, 0, bytemuck::bytes_of(&params));

        let workgroups = candidates.len().div_ceil(self.workgroup_size as usize) as u32;

        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("dict-batch-encoder"),
            });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("dict-batch-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &s.bind_group, &[]);
            pass.dispatch_workgroups(workgroups, 1, 1);
        }
        enc.copy_buffer_to_buffer(&s.match_buf, 0, &s.match_staging, 0, self.match_buf_size);
        self.queue.submit(Some(enc.finish()));
        Ok(())
    }

    /// Map the staging buffer for `slot`, wait for the corresponding submission
    /// to drain, and return its match records.
    pub async fn read_matches(&self, slot: usize) -> Result<Vec<MatchRecord>> {
        if slot >= self.slots.len() {
            return Err(Error::Gpu(format!(
                "slot {slot} out of range (max_in_flight={})",
                self.slots.len()
            )));
        }
        let s = &self.slots[slot];
        let slice = s.match_staging.slice(..);
        let (tx, rx) = tokio::sync::oneshot::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.await
            .map_err(|e| Error::Gpu(format!("map_async sender dropped: {e}")))?
            .map_err(|e| Error::Gpu(format!("map_async failed: {e}")))?;

        let records = {
            let view = slice.get_mapped_range();
            let words: &[u32] = bytemuck::cast_slice(&view);
            let count = words[0];
            let kept = count.min(self.max_matches) as usize;
            let mut out = Vec::with_capacity(kept);
            for i in 0..kept {
                let base = 4 + i * 2;
                out.push(MatchRecord {
                    candidate_idx: words[base],
                    target_idx: words[base + 1],
                });
            }
            if count > self.max_matches {
                tracing::warn!(
                    found = count,
                    capacity = self.max_matches,
                    "dict batch produced more matches than capacity; tail dropped"
                );
            }
            out
        };
        s.match_staging.unmap();
        Ok(records)
    }

    /// Convenience: submit + immediately await results.
    pub async fn dispatch_batch(&self, candidates: &[CandidateSlot]) -> Result<Vec<MatchRecord>> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        self.submit(0, candidates)?;
        self.read_matches(0).await
    }

    pub fn batch_size(&self) -> u32 {
        self.batch_size
    }

    pub fn workgroup_size(&self) -> u32 {
        self.workgroup_size
    }

    pub fn max_in_flight(&self) -> usize {
        self.slots.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::digest;
    use crate::gpu::algos::{md5 as md5_kernel, sha1 as sha1_kernel, sha256 as sha256_kernel};
    use crate::gpu::buffers::CandidateSlot;
    use crate::Algorithm;

    /// Run a 5-input dict test with the given (algorithm, spec) pair. Asserts
    /// that GPU matches CPU for every input.
    async fn assert_dict_matches_cpu(algo: Algorithm, spec: DictKernelSpec) {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("gpuhash_core=info")),
            )
            .with_test_writer()
            .try_init();

        let inputs: Vec<&[u8]> = vec![b"", b"a", b"abc", b"password", b"hello, world"];
        let targets: Vec<Vec<u8>> = inputs.iter().map(|i| digest(algo, i).unwrap()).collect();

        let runner = DictRunner::new(
            spec,
            &targets,
            64,
            DEFAULT_WORKGROUP_SIZE,
            64,
            DEFAULT_MAX_IN_FLIGHT,
        )
        .await
        .expect("runner construction");
        let slots: Vec<CandidateSlot> = inputs
            .iter()
            .map(|i| CandidateSlot::pack(i).expect("fits single-block"))
            .collect();
        let mut matches = runner.dispatch_batch(&slots).await.expect("dispatch ok");
        matches.sort_by_key(|m| (m.candidate_idx, m.target_idx));
        let expected: Vec<MatchRecord> = (0..inputs.len() as u32)
            .map(|i| MatchRecord {
                candidate_idx: i,
                target_idx: i,
            })
            .collect();
        assert_eq!(matches, expected, "{algo} GPU disagrees with CPU");
    }

    #[tokio::test]
    async fn md5_dict_matches_cpu() {
        assert_dict_matches_cpu(Algorithm::Md5, md5_kernel::DICT_SPEC).await;
    }

    #[tokio::test]
    async fn sha1_dict_matches_cpu() {
        assert_dict_matches_cpu(Algorithm::Sha1, sha1_kernel::DICT_SPEC).await;
    }

    #[tokio::test]
    async fn sha256_dict_matches_cpu() {
        assert_dict_matches_cpu(Algorithm::Sha256, sha256_kernel::DICT_SPEC).await;
    }

    #[tokio::test]
    async fn dict_no_match_when_target_absent() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        // Use MD5 spec; the no-match check is algorithm-independent.
        let bogus_target: Vec<u8> = (0u8..16).collect();
        let runner = DictRunner::new(
            md5_kernel::DICT_SPEC,
            &[bogus_target],
            16,
            DEFAULT_WORKGROUP_SIZE,
            8,
            1,
        )
        .await
        .expect("runner ok");
        let inputs: Vec<&[u8]> = vec![b"alpha", b"bravo", b"charlie"];
        let slots: Vec<CandidateSlot> = inputs
            .iter()
            .map(|i| CandidateSlot::pack(i).unwrap())
            .collect();
        let matches = runner.dispatch_batch(&slots).await.expect("dispatch ok");
        assert!(matches.is_empty(), "unexpected matches: {matches:?}");
    }

    #[tokio::test]
    async fn dict_two_in_flight_batches() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let inputs_a: Vec<&[u8]> = vec![b"alpha", b"bravo", b"charlie"];
        let inputs_b: Vec<&[u8]> = vec![b"delta", b"echo", b"foxtrot"];
        let mut all_targets: Vec<Vec<u8>> = Vec::new();
        for input in inputs_a.iter().chain(inputs_b.iter()) {
            all_targets.push(digest(Algorithm::Md5, input).unwrap());
        }
        let runner = DictRunner::new(
            md5_kernel::DICT_SPEC,
            &all_targets,
            32,
            DEFAULT_WORKGROUP_SIZE,
            32,
            2,
        )
        .await
        .expect("runner ok");
        let slots_a: Vec<CandidateSlot> = inputs_a
            .iter()
            .map(|i| CandidateSlot::pack(i).unwrap())
            .collect();
        let slots_b: Vec<CandidateSlot> = inputs_b
            .iter()
            .map(|i| CandidateSlot::pack(i).unwrap())
            .collect();

        runner.submit(0, &slots_a).expect("submit 0");
        runner.submit(1, &slots_b).expect("submit 1");
        let matches_a = runner.read_matches(0).await.expect("read 0");
        let matches_b = runner.read_matches(1).await.expect("read 1");

        assert_eq!(matches_a.len(), 3);
        assert_eq!(matches_b.len(), 3);
        for m in &matches_a {
            assert!(m.target_idx < 3);
        }
        for m in &matches_b {
            assert!(m.target_idx >= 3 && m.target_idx < 6);
        }
    }
}
