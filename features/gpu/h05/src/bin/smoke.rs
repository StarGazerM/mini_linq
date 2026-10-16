use cuda_core::{CudaContext, DeviceBuffer, LaunchConfig1D};
use cuda_device::{kernel, launch_bounds, launch_contract, thread, DisjointSlice};
#[cuda_device::cuda_module]
mod kernels {
    use super::*;
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn vecadd(a: &[f32], b: &[f32], mut out: DisjointSlice<f32>) {
        let index = thread::index_1d();
        let i = index.get();
        if let Some(value) = out.get_mut(index) { *value = a[i] + b[i]; }
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = CudaContext::new(0)?;
    let stream = ctx.default_stream();
    let module = unsafe { kernels::load(&ctx)? };
    let a = DeviceBuffer::from_host(&stream, &vec![1.0f32; 1025])?;
    let b = DeviceBuffer::from_host(&stream, &vec![2.0f32; 1025])?;
    let mut out = DeviceBuffer::zeroed(&stream, 1025)?;
    let launch = module.prepare_vecadd(LaunchConfig1D::new(5, 256, 0))?;
    module.vecadd(&stream, &launch, &a, &b, &mut out)?;
    assert_eq!(out.to_host_vec(&stream)?, vec![3.0f32; 1025]);
    println!("smoke: 1025 vector additions exact");
    Ok(())
}
