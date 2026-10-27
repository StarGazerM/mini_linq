use metal::objc::{msg_send, sel, sel_impl};
use metal::*;
use std::time::Instant;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub key: u32,
    pub value: u32,
}

pub struct Gpu {
    pub device: Device,
    pub queue: CommandQueue,
    library: Library,
}

impl Gpu {
    pub fn new() -> Self {
        let device = Device::system_default().expect("Metal device required");
        let source = [
            include_str!("common.metal"),
            include_str!("sort.metal"),
            include_str!("maintain.metal"),
        ]
        .join("\n");
        let library = device
            .new_library_with_source(&source, &CompileOptions::new())
            .expect("Metal compilation");
        let queue = device.new_command_queue();
        Self {
            device,
            queue,
            library,
        }
    }
    pub fn buffer(&self, bytes: usize) -> Buffer {
        let bytes = bytes.max(8);
        assert!(bytes as u64 <= self.device.max_buffer_length());
        let b = self
            .device
            .new_buffer(bytes as u64, MTLResourceOptions::StorageModeShared);
        // Pre-touch allocations outside measurements. CPU writes precede submission.
        unsafe {
            std::ptr::write_bytes(b.contents(), 0, bytes);
        }
        b
    }
    pub fn pipeline(&self, name: &str) -> ComputePipelineState {
        let f = self.library.get_function(name, None).expect(name);
        let p = self
            .device
            .new_compute_pipeline_state_with_function(&f)
            .expect(name);
        assert_eq!(p.thread_execution_width(), 32);
        assert!(p.max_total_threads_per_threadgroup() >= 256);
        p
    }
    pub fn dispatch(
        &self,
        cb: &CommandBufferRef,
        p: &ComputePipelineStateRef,
        buffers: &[&BufferRef],
        constants: &[u32],
        groups: usize,
        threads: usize,
    ) {
        if groups == 0 {
            return;
        }
        let enc = cb.new_compute_command_encoder();
        enc.set_compute_pipeline_state(p);
        for (i, b) in buffers.iter().enumerate() {
            enc.set_buffer(i as u64, Some(b), 0);
        }
        if !constants.is_empty() {
            enc.set_bytes(
                buffers.len() as u64,
                std::mem::size_of_val(constants) as u64,
                constants.as_ptr().cast(),
            );
        }
        enc.dispatch_thread_groups(
            MTLSize::new(groups as u64, 1, 1),
            MTLSize::new(threads as u64, 1, 1),
        );
        enc.end_encoding();
    }
    pub fn run(&self, encode: impl FnOnce(&CommandBufferRef)) -> Sample {
        let start = Instant::now();
        let cb = self.queue.new_command_buffer();
        encode(cb);
        cb.commit();
        cb.wait_until_completed();
        assert_eq!(
            cb.status(),
            MTLCommandBufferStatus::Completed,
            "GPU command failed"
        );
        let wall_ms = start.elapsed().as_secs_f64() * 1000.0;
        let (gpu_start, gpu_end): (f64, f64) =
            unsafe { (msg_send![cb, GPUStartTime], msg_send![cb, GPUEndTime]) };
        Sample {
            wall_ms,
            gpu_ms: (gpu_end - gpu_start) * 1000.0,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Sample {
    pub wall_ms: f64,
    pub gpu_ms: f64,
}

// Only call while no submitted command accesses this buffer. The harness waits
// before reading, and never exposes mutable shared CPU/GPU aliases.
pub fn view<T>(b: &BufferRef, n: usize) -> &[T] {
    assert!(std::mem::size_of::<T>() * n <= b.length() as usize);
    unsafe { std::slice::from_raw_parts(b.contents().cast(), n) }
}
pub fn upload<T: Copy>(b: &BufferRef, data: &[T]) {
    assert!(std::mem::size_of_val(data) <= b.length() as usize);
    unsafe {
        std::ptr::copy_nonoverlapping(data.as_ptr(), b.contents().cast(), data.len());
    }
}

pub struct Groups {
    count: ComputePipelineState,
    scan: ComputePipelineState,
    scatter: ComputePipelineState,
    counts: Buffer,
    offsets: Buffer,
    pub keys: Buffer,
    pub starts: Buffer,
    pub total: Buffer,
    capacity: usize,
}
impl Groups {
    pub fn new(gpu: &Gpu, capacity: usize) -> Self {
        Self {
            count: gpu.pipeline("group_count"),
            scan: gpu.pipeline("group_scan"),
            scatter: gpu.pipeline("group_scatter"),
            counts: gpu.buffer(4 * capacity.div_ceil(2048)),
            offsets: gpu.buffer(4 * capacity.div_ceil(2048)),
            keys: gpu.buffer(capacity * 4),
            starts: gpu.buffer((capacity + 1) * 4),
            total: gpu.buffer(4),
            capacity,
        }
    }
    pub fn encode(&self, gpu: &Gpu, cb: &CommandBufferRef, rows: &BufferRef, n: usize) {
        assert!(n <= self.capacity);
        let tiles = n.div_ceil(2048);
        gpu.dispatch(
            cb,
            &self.count,
            &[rows, &self.counts],
            &[n as u32],
            tiles,
            256,
        );
        gpu.dispatch(
            cb,
            &self.scan,
            &[&self.counts, &self.offsets, &self.total, &self.starts],
            &[tiles as u32, n as u32],
            1,
            256,
        );
        gpu.dispatch(
            cb,
            &self.scatter,
            &[rows, &self.offsets, &self.keys, &self.starts],
            &[n as u32],
            tiles,
            256,
        );
    }
    pub fn len(&self) -> usize {
        view::<u32>(&self.total, 1)[0] as usize
    }
}
