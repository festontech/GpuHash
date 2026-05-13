//! `Md5GpuRunner` — owns the wgpu device, the MD5 pipeline, and the persistent
//! buffers used across batched dispatches.
//!
//! Phase-3 scope: one batch per dispatch, awaited. The runner is sized once at
//! construction time (`batch_size`, `max_matches`, target list) and then reused
//! for as many `dispatch_batch` calls as the engine needs.
//!
//! Buffer reuse policy (see `docs/ARCHITECTURE.md` and `CLAUDE.md`):
//! - Pipelines built once per algorithm.
//! - Storage buffers (candidates, targets, matches) allocated once at the
//!   batch-size cap and reused; we `write_buffer` per dispatch instead of
//!   creating new buffers.
//! - Only the small match-staging buffer is `MAP_READ`. Hot-path buffers never
//!   round-trip to the CPU.

use bytemuck;
use wgpu::util::DeviceExt;

use crate::gpu::buffers::{CandidateSlot, MatchRecord, Params};
use crate::{Error, Result};

const WORKGROUP_SIZE: u32 = 64;

/// Owning handle to a GPU MD5 attack runner.
///
/// Construct once via [`Md5GpuRunner::new`]; call [`Md5GpuRunner::dispatch_batch`]
/// repeatedly with up to `batch_size` candidates per call.
pub struct Md5GpuRunner {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,

    // Persistent device-side buffers.
    candidates_buf: wgpu::Buffer,
    match_buf: wgpu::Buffer,
    params_buf: wgpu::Buffer,
    // Small staging buffer for reading the match results back to the host. The
    // candidate and target buffers never travel back (write-only from the host).
    match_staging: wgpu::Buffer,

    batch_size: u32,
    max_matches: u32,
    num_targets: u32,
    match_buf_size: u64,
}

impl Md5GpuRunner {
    /// Construct a runner targeting the host's preferred GPU adapter and
    /// pre-upload the target hashes.
    ///
    /// - `targets` — one digest per entry; each must be exactly 16 bytes (MD5).
    /// - `batch_size` — maximum number of candidates per `dispatch_batch` call.
    /// - `max_matches` — capacity of the match-record buffer per dispatch. Any
    ///   overflow is silently dropped (the in-shader `atomicAdd` still counts it,
    ///   so an overflow is detectable but we don't recover the records).
    pub async fn new(targets: &[Vec<u8>], batch_size: u32, max_matches: u32) -> Result<Self> {
        if batch_size == 0 {
            return Err(Error::Gpu("batch_size must be > 0".into()));
        }
        if max_matches == 0 {
            return Err(Error::Gpu("max_matches must be > 0".into()));
        }
        if targets.is_empty() {
            return Err(Error::Gpu("at least one target hash required".into()));
        }
        for (i, t) in targets.iter().enumerate() {
            if t.len() != 16 {
                return Err(Error::Gpu(format!(
                    "target {i} is {} bytes, expected 16 (MD5)",
                    t.len()
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
            "md5 runner: adapter selected"
        );

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default(), None)
            .await
            .map_err(|e| Error::Gpu(format!("request_device: {e}")))?;

        // ---- pipeline ----
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("md5"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/md5.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("md5-pipeline"),
            layout: None, // inferred from the shader
            module: &module,
            entry_point: "md5_attack",
            compilation_options: Default::default(),
            cache: None,
        });
        let bgl = pipeline.get_bind_group_layout(0);

        // ---- buffers ----
        let candidates_size = (batch_size as u64) * (std::mem::size_of::<CandidateSlot>() as u64);
        let candidates_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("md5-candidates"),
            size: candidates_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Targets are written once and reused across all dispatches.
        let target_words: Vec<u32> = targets
            .iter()
            .flat_map(|t| {
                let mut words = [0u32; 4];
                for (i, chunk) in t.chunks_exact(4).enumerate() {
                    words[i] = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                }
                words
            })
            .collect();
        let targets_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("md5-targets"),
            contents: bytemuck::cast_slice(&target_words),
            usage: wgpu::BufferUsages::STORAGE,
        });

        // MatchBuf layout (matches WGSL):
        //   count: u32                       — offset 0, 4 bytes
        //   _pad : array<u32, 3>             — offset 4, 12 bytes
        //   pairs: array<u32, 2*max_matches> — offset 16, 8*max_matches bytes
        let match_buf_size: u64 = 16 + 8 * (max_matches as u64);
        let match_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("md5-match-buf"),
            size: match_buf_size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let match_staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("md5-match-staging"),
            size: match_buf_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("md5-params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("md5-bind-group"),
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

        Ok(Self {
            device,
            queue,
            pipeline,
            bind_group,
            candidates_buf,
            match_buf,
            params_buf,
            match_staging,
            batch_size,
            max_matches,
            num_targets: targets.len() as u32,
            match_buf_size,
        })
    }

    /// Run one batch through the kernel and return the matches it found.
    ///
    /// `candidates.len()` must be `<= batch_size` (validated). The slots' length
    /// fields are trusted; the caller is responsible for honouring
    /// [`crate::gpu::buffers::MAX_CANDIDATE_LEN`] when packing.
    pub async fn dispatch_batch(&self, candidates: &[CandidateSlot]) -> Result<Vec<MatchRecord>> {
        if candidates.len() as u32 > self.batch_size {
            return Err(Error::Gpu(format!(
                "batch of {} exceeds runner capacity {}",
                candidates.len(),
                self.batch_size
            )));
        }
        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        // Upload candidates.
        self.queue
            .write_buffer(&self.candidates_buf, 0, bytemuck::cast_slice(candidates));

        // Zero the match buffer header (count + pad). Pairs from a previous run
        // beyond the new count are simply ignored.
        let header_zero = [0u32; 4];
        self.queue
            .write_buffer(&self.match_buf, 0, bytemuck::cast_slice(&header_zero));

        // Update params.
        let params = Params {
            num_candidates: candidates.len() as u32,
            num_targets: self.num_targets,
            max_matches: self.max_matches,
            _pad: 0,
        };
        self.queue
            .write_buffer(&self.params_buf, 0, bytemuck::bytes_of(&params));

        let workgroups = candidates.len().div_ceil(WORKGROUP_SIZE as usize) as u32;

        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("md5-batch-encoder"),
            });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("md5-batch-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.dispatch_workgroups(workgroups, 1, 1);
        }
        enc.copy_buffer_to_buffer(
            &self.match_buf,
            0,
            &self.match_staging,
            0,
            self.match_buf_size,
        );
        self.queue.submit(Some(enc.finish()));

        // Read back. Only the small match-staging buffer is mapped.
        let slice = self.match_staging.slice(..);
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
            // Layout: [count, _pad, _pad, _pad, p0_cand, p0_tgt, p1_cand, p1_tgt, ...]
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
                    "md5 batch produced more matches than buffer capacity; tail dropped"
                );
            }
            out
        };
        self.match_staging.unmap();

        Ok(records)
    }

    pub fn batch_size(&self) -> u32 {
        self.batch_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::digest;
    use crate::gpu::buffers::CandidateSlot;
    use crate::Algorithm;

    #[tokio::test]
    async fn md5_gpu_matches_cpu_on_short_inputs() {
        // Install a tracing subscriber so adapter info is visible with --nocapture.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("gpuhash_core=info")),
            )
            .with_test_writer()
            .try_init();

        // Inputs of varying lengths that fit in single-block MD5 (<= 55 bytes).
        // We don't pin to RFC 1321 vectors specifically; the contract under test
        // is "GPU agrees with the CPU reference on whatever we feed both."
        let inputs: Vec<&[u8]> = vec![
            b"",
            b"a",
            b"abc",
            b"password",
            b"qwerty",
            b"hello, world",
            b"message digest",
            b"abcdefghijklmnopqrstuvwxyz",
        ];
        let targets: Vec<Vec<u8>> = inputs
            .iter()
            .map(|i| digest(Algorithm::Md5, i).unwrap())
            .collect();

        let runner = Md5GpuRunner::new(&targets, 64, 64)
            .await
            .expect("runner construction should succeed");

        let slots: Vec<CandidateSlot> = inputs
            .iter()
            .map(|i| CandidateSlot::pack(i).expect("input fits single-block MD5"))
            .collect();

        let mut matches = runner
            .dispatch_batch(&slots)
            .await
            .expect("dispatch should succeed");

        // Each candidate should match exactly its own target (idx == idx). The
        // shader's atomic counter doesn't guarantee match ordering, so sort.
        matches.sort_by_key(|m| (m.candidate_idx, m.target_idx));
        let expected: Vec<MatchRecord> = (0..inputs.len() as u32)
            .map(|i| MatchRecord {
                candidate_idx: i,
                target_idx: i,
            })
            .collect();
        assert_eq!(matches, expected, "GPU MD5 disagreed with CPU MD5");
    }

    #[tokio::test]
    async fn md5_gpu_no_match_when_target_absent() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();

        // Target a digest that none of the candidates produce.
        let bogus_target: Vec<u8> = (0u8..16).collect(); // 00 01 02 .. 0f
        let runner = Md5GpuRunner::new(&[bogus_target], 16, 8)
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
}
