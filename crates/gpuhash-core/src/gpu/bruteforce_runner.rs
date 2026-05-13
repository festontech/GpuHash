//! `Md5BruteforceRunner` — GPU mask-bruteforce equivalent of `Md5GpuRunner`.
//!
//! The mask and the target list are uploaded once at construction; each batch
//! only changes `base_index` (and the number of candidates in the trailing
//! batch). Candidate bytes are not transferred — each thread synthesizes its
//! own from `base_index + gid.x` against the per-position mask spec.
//!
//! Same slot/ring discipline as `Md5GpuRunner`: per-slot params/match/staging
//! buffers + bind group; mask, targets, and pipeline are shared.

use bytemuck;
use wgpu::util::DeviceExt;

use crate::gpu::buffers::{BruteforceParams, MaskPosGpu, MatchRecord};
use crate::mask::{Mask, Position, MAX_MASK_POSITIONS};
use crate::{Error, Result};

const WORKGROUP_SIZE: u32 = 64;

pub struct Md5BruteforceRunner {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    #[allow(dead_code)]
    mask_buf: wgpu::Buffer,
    #[allow(dead_code)]
    targets_buf: wgpu::Buffer,
    slots: Vec<SlotBuffers>,

    batch_size: u32,
    max_matches: u32,
    num_targets: u32,
    num_positions: u32,
    match_buf_size: u64,
}

struct SlotBuffers {
    match_buf: wgpu::Buffer,
    match_staging: wgpu::Buffer,
    params_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// Pack a parsed `Mask` into the GPU `MaskPosGpu` array.
pub fn mask_to_gpu_positions(mask: &Mask) -> Vec<MaskPosGpu> {
    use crate::mask::CharsetKind;
    mask.positions()
        .iter()
        .map(|p| match p {
            Position::Literal(b) => MaskPosGpu::literal(*b),
            Position::Charset(CharsetKind::Lower) => MaskPosGpu::lowercase(),
            Position::Charset(CharsetKind::Upper) => MaskPosGpu::uppercase(),
            Position::Charset(CharsetKind::Digit) => MaskPosGpu::digit(),
        })
        .collect()
}

impl Md5BruteforceRunner {
    pub async fn new(
        mask: &Mask,
        targets: &[Vec<u8>],
        batch_size: u32,
        max_matches: u32,
        max_in_flight: usize,
    ) -> Result<Self> {
        if batch_size == 0 {
            return Err(Error::Gpu("batch_size must be > 0".into()));
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
            if t.len() != 16 {
                return Err(Error::Gpu(format!(
                    "target {i} is {} bytes, expected 16 (MD5)",
                    t.len()
                )));
            }
        }
        if mask.num_positions() > MAX_MASK_POSITIONS {
            return Err(Error::Gpu(format!(
                "mask has {} positions, GPU shader max is {MAX_MASK_POSITIONS}",
                mask.num_positions()
            )));
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
            mask = %mask,
            "md5 bruteforce runner: adapter selected"
        );

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default(), None)
            .await
            .map_err(|e| Error::Gpu(format!("request_device: {e}")))?;

        // ---- pipeline (shared common helpers + brute entry point) ----
        let shader_src = format!(
            "{}\n{}",
            include_str!("shaders/md5_common.wgsl"),
            include_str!("shaders/md5_bruteforce.wgsl"),
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("md5-bruteforce"),
            source: wgpu::ShaderSource::Wgsl(shader_src.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("md5-bruteforce-pipeline"),
            layout: None,
            module: &module,
            entry_point: "md5_bruteforce",
            compilation_options: Default::default(),
            cache: None,
        });
        let bgl = pipeline.get_bind_group_layout(0);

        // ---- shared mask buffer ----
        let mask_positions = mask_to_gpu_positions(mask);
        let mask_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("md5-bruteforce-mask"),
            contents: bytemuck::cast_slice(&mask_positions),
            usage: wgpu::BufferUsages::STORAGE,
        });

        // ---- shared targets buffer ----
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
            label: Some("md5-bruteforce-targets"),
            contents: bytemuck::cast_slice(&target_words),
            usage: wgpu::BufferUsages::STORAGE,
        });

        // ---- per-slot buffers ----
        let match_buf_size: u64 = 16 + 8 * (max_matches as u64);
        let mut slots = Vec::with_capacity(max_in_flight);
        for slot_idx in 0..max_in_flight {
            let match_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("md5-brute-match-{slot_idx}")),
                size: match_buf_size,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let match_staging = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("md5-brute-match-staging-{slot_idx}")),
                size: match_buf_size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let params_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("md5-brute-params-{slot_idx}")),
                size: std::mem::size_of::<BruteforceParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("md5-brute-bind-{slot_idx}")),
                layout: &bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: mask_buf.as_entire_binding(),
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
            mask_buf,
            targets_buf,
            slots,
            batch_size,
            max_matches,
            num_targets: targets.len() as u32,
            num_positions: mask.num_positions() as u32,
            match_buf_size,
        })
    }

    /// Submit a batch covering `num_candidates` candidate indices starting at
    /// `base_index`. Pair every successful `submit` with a `read_matches` on the
    /// same slot before reusing it.
    pub fn submit(&self, slot: usize, base_index: u32, num_candidates: u32) -> Result<()> {
        if slot >= self.slots.len() {
            return Err(Error::Gpu(format!(
                "slot {slot} out of range (max_in_flight={})",
                self.slots.len()
            )));
        }
        if num_candidates == 0 {
            return Err(Error::Gpu("submit called with num_candidates=0".into()));
        }
        if num_candidates > self.batch_size {
            return Err(Error::Gpu(format!(
                "num_candidates {num_candidates} exceeds batch_size {}",
                self.batch_size
            )));
        }
        let s = &self.slots[slot];

        // Zero match-buffer header.
        let header_zero = [0u32; 4];
        self.queue
            .write_buffer(&s.match_buf, 0, bytemuck::cast_slice(&header_zero));

        let params = BruteforceParams {
            num_positions: self.num_positions,
            num_candidates,
            num_targets: self.num_targets,
            max_matches: self.max_matches,
            base_index,
            _pad0: 0,
            _pad1: 0,
            _pad2: 0,
        };
        self.queue
            .write_buffer(&s.params_buf, 0, bytemuck::bytes_of(&params));

        let workgroups = num_candidates.div_ceil(WORKGROUP_SIZE);

        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("md5-brute-encoder"),
            });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("md5-brute-pass"),
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

    /// Read back the match records produced by the most recent submit on `slot`.
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
                    "md5 brute batch produced more matches than capacity; tail dropped"
                );
            }
            out
        };
        s.match_staging.unmap();
        Ok(records)
    }

    pub fn batch_size(&self) -> u32 {
        self.batch_size
    }

    pub fn max_in_flight(&self) -> usize {
        self.slots.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::digest;
    use crate::gpu::runner::DEFAULT_MAX_IN_FLIGHT;
    use crate::Algorithm;

    #[tokio::test]
    async fn md5_brute_matches_cpu_reference() {
        // Mask "?d?d?d" → 1000 candidates. Pick four that we know exist.
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();

        let mask = Mask::parse("?d?d?d").unwrap();
        let pick = [
            b"000".to_vec(),
            b"123".to_vec(),
            b"999".to_vec(),
            b"042".to_vec(),
        ];
        let targets: Vec<Vec<u8>> = pick
            .iter()
            .map(|p| digest(Algorithm::Md5, p).unwrap())
            .collect();

        let runner = Md5BruteforceRunner::new(&mask, &targets, 1024, 32, DEFAULT_MAX_IN_FLIGHT)
            .await
            .expect("runner ok");

        // One batch covers the entire 1000-candidate keyspace.
        runner.submit(0, 0, mask.total() as u32).expect("submit ok");
        let mut matches = runner.read_matches(0).await.expect("read ok");
        matches.sort_by_key(|m| (m.candidate_idx, m.target_idx));

        // Each of our target indices (the 4 we picked) should appear in
        // matches, with candidate_idx equal to the index of that string in the
        // mask's keyspace.
        let expected: Vec<(u32, u32)> = pick
            .iter()
            .enumerate()
            .map(|(t_idx, bytes)| {
                let cand_idx = std::str::from_utf8(bytes).unwrap().parse::<u32>().unwrap();
                (cand_idx, t_idx as u32)
            })
            .collect();
        let actual: Vec<(u32, u32)> = matches
            .iter()
            .map(|m| (m.candidate_idx, m.target_idx))
            .collect();
        // Sort both for comparison.
        let mut e = expected;
        e.sort();
        let mut a = actual;
        a.sort();
        assert_eq!(a, e, "GPU bruteforce disagrees with CPU expectation");
    }

    #[tokio::test]
    async fn md5_brute_with_literal_position() {
        // Mask "x?d?d" — candidates "x00".."x99" (100 candidates).
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();

        let mask = Mask::parse("x?d?d").unwrap();
        assert_eq!(mask.total(), 100);

        let pick = [b"x00".to_vec(), b"x42".to_vec(), b"x99".to_vec()];
        let targets: Vec<Vec<u8>> = pick
            .iter()
            .map(|p| digest(Algorithm::Md5, p).unwrap())
            .collect();

        let runner = Md5BruteforceRunner::new(&mask, &targets, 128, 16, 1)
            .await
            .expect("runner ok");

        runner.submit(0, 0, mask.total() as u32).expect("submit ok");
        let matches = runner.read_matches(0).await.expect("read ok");
        assert_eq!(matches.len(), pick.len(), "expected one match per target");
    }
}
