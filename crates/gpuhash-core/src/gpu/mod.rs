//! GPU plumbing — wgpu adapter/device/pipeline scaffolding.
//!
//! - [`smoke`] is the Phase-2 smallest-possible-kernel sanity check.
//! - [`buffers`] / [`runner`] / `shaders/md5.wgsl` form the Phase-3 MD5 dictionary
//!   attack path. The host packs candidates into fixed-size slots, the WGSL kernel
//!   computes MD5 per slot and compares against the target list, and matches are
//!   read back via a small staging buffer.

pub mod algos;
pub mod bruteforce_runner;
pub mod buffers;
pub mod kernel_spec;
pub mod runner;

use bytemuck;
use wgpu::util::DeviceExt;

use crate::{Error, Result};

/// Run the Phase-2 smoke test: dispatch a no-op WGSL kernel that writes `1u` into
/// a single-element storage buffer, map it back, and return the value.
///
/// Returns `Ok(1)` on success. Anything else (a different value, no adapter, a
/// device error) means the GPU plumbing on this machine is broken and we should
/// not yet move on to Phase 3 (real MD5 in WGSL).
///
/// Also logs `Adapter::get_info()` at INFO level — on Windows + Intel iGPU this
/// should print the DX12 backend.
pub async fn smoke() -> Result<u32> {
    const SHADER: &str = r#"
        @group(0) @binding(0) var<storage, read_write> data : array<u32>;
        @compute @workgroup_size(1) fn main() { data[0] = 1u; }
    "#;

    let instance = wgpu::Instance::default();

    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .ok_or_else(|| Error::Gpu("no compatible GPU adapter found".into()))?;

    let info = adapter.get_info();
    tracing::info!(
        name = %info.name,
        vendor = info.vendor,
        device_type = ?info.device_type,
        backend = ?info.backend,
        driver = %info.driver,
        driver_info = %info.driver_info,
        "wgpu adapter"
    );

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default(), None)
        .await
        .map_err(|e| Error::Gpu(format!("request_device: {e}")))?;

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("smoke"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });

    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("smoke-data"),
        contents: bytemuck::cast_slice(&[0u32]),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("smoke-staging"),
        size: 4,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("smoke-pipeline"),
        layout: None,
        module: &module,
        entry_point: "main",
        compilation_options: Default::default(),
        cache: None,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("smoke-bind-group"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });

    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("smoke-encoder"),
    });
    {
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("smoke-pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    enc.copy_buffer_to_buffer(&buffer, 0, &staging, 0, 4);
    queue.submit(Some(enc.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = tokio::sync::oneshot::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::Maintain::Wait);
    rx.await
        .map_err(|e| Error::Gpu(format!("map_async sender dropped: {e}")))?
        .map_err(|e| Error::Gpu(format!("map_async failed: {e}")))?;

    let value = {
        let view = slice.get_mapped_range();
        bytemuck::cast_slice::<u8, u32>(&view)[0]
    };
    staging.unmap();

    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn smoke_returns_one() {
        // Install a tracing subscriber so adapter info (logged at INFO inside `smoke`)
        // is visible when this test runs with `--nocapture`. `try_init` makes it safe
        // for any other test in the same binary to also install one.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("gpuhash_core=info")),
            )
            .with_test_writer()
            .try_init();

        let v = smoke()
            .await
            .expect("smoke() should succeed on any machine with a working GPU adapter");
        assert_eq!(v, 1, "WGSL kernel should write 1u into data[0]");
    }
}
