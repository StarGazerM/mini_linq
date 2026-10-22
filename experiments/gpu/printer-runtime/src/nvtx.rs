//! Compile-time optional, thread-scoped Nsight annotations. Disabled builds
//! contain no NVTX calls, string allocation, environment checks, or linking.
#[cfg(feature = "nvtx")]
unsafe extern "C" {
    fn cq_nvtx_push(name: *const std::ffi::c_char);
    fn cq_nvtx_pop();
    fn cq_nvtx_stream(stream: *mut std::ffi::c_void, name: *const std::ffi::c_char);
}

#[must_use]
pub struct Range(std::marker::PhantomData<*mut ()>);

/// Names are static C strings so annotated hot paths never allocate.
#[inline(always)]
pub fn range(name: &'static std::ffi::CStr) -> Range {
    #[cfg(feature = "nvtx")]
    unsafe {
        cq_nvtx_push(name.as_ptr());
    }
    #[cfg(not(feature = "nvtx"))]
    let _ = name;
    Range(std::marker::PhantomData)
}

impl Drop for Range {
    #[inline(always)]
    fn drop(&mut self) {
        #[cfg(feature = "nvtx")]
        unsafe {
            cq_nvtx_pop();
        }
    }
}

#[cfg(feature = "legacy-datalog")]
#[inline(always)]
pub(crate) fn name_stream(stream: *mut std::ffi::c_void) {
    #[cfg(feature = "nvtx")]
    unsafe {
        cq_nvtx_stream(stream, c"MiniLinq resident".as_ptr());
    }
    #[cfg(not(feature = "nvtx"))]
    let _ = stream;
}
