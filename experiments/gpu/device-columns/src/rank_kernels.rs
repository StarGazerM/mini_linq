#[cuda_device::cuda_module]
pub(crate) mod rank_load {
    use crate::domain::{DomainView, domain_exact, domain_lower};
    use crate::native::{Input, ELEMENT_ENCODED};
    use crate::{decode, load_element, logical_len};
    use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU32};
    use cuda_device::{device, kernel, launch_bounds, launch_contract, thread, DynamicSharedArray};

    #[device]
    #[inline(always)]
    unsafe fn rank_input_value(input: Input, row: usize) -> i64 {
        if input.element == ELEMENT_ENCODED {
            // DECODE I: load-time extraction constructs shared domain membership and immediately re-encodes keys.
            unsafe { decode(input.column, row as u64) }
        } else {
            unsafe { load_element(input.column.data.cast::<u8>().add(row * input.stride), input.element, 0) }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn domain_rank_bitmap(input: Input, minimum: i64, words: *mut u32) {
        let mut row = thread::index_1d().get();
        let rows = unsafe { logical_len(input.column) } as usize;
        let stride = thread::gridDim_x() as usize * 256;
        while row < rows {
            let offset = (unsafe { rank_input_value(input, row) } as u64).wrapping_sub(minimum as u64);
            unsafe { DeviceAtomicU32::from_ptr(words.add(offset as usize / 32))
                .fetch_or(1 << (offset % 32), AtomicOrdering::Relaxed) };
            row += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn domain_rank_counts(bitmap: *const u32, words: usize, directory: *mut u32) {
        let mut word = thread::index_1d().get();
        let stride = thread::gridDim_x() as usize * 256;
        while word < words {
            unsafe { directory.add(word).write(bitmap.add(word).read().count_ones()) };
            word += stride;
        }
    }

    // Hierarchical exclusive scan over per-bitmap-word popcounts. Each
    // level is stream-local scratch; the root alone is downloaded as cardinality.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (1024, 1024))]
    pub unsafe fn domain_rank_scan(values: *mut u32, len: usize, totals: *mut u32) {
        let tid = thread::threadIdx_x() as usize;
        let shared = DynamicSharedArray::<u8>::get_raw().cast::<u32>();
        let mut block = thread::blockIdx_x() as usize;
        while block < len.div_ceil(256) {
            let i = block * 256 + tid;
            let value = if i < len { unsafe { values.add(i).read() } } else { 0 };
            let mut sum = value;
            unsafe { shared.add(tid).write(sum) };
            thread::sync_threads();
            let mut distance = 1;
            while distance < 256 {
                let before = if tid >= distance { unsafe { shared.add(tid - distance).read() } } else { 0 };
                thread::sync_threads();
                sum += before;
                unsafe { shared.add(tid).write(sum) };
                thread::sync_threads();
                distance *= 2;
            }
            if i < len { unsafe { values.add(i).write(sum - value) }; }
            if tid == 255 { unsafe { totals.add(block).write(sum) }; }
            thread::sync_threads();
            block += thread::gridDim_x() as usize;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn domain_rank_add(values: *mut u32, len: usize, prefixes: *const u32) {
        let mut i = thread::index_1d().get();
        let stride = thread::gridDim_x() as usize * 256;
        while i < len {
            unsafe { *values.add(i) += prefixes.add(i / 256).read() };
            i += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn domain_rank_lookup(domain: DomainView, value: i64, result: *mut u32) {
        if thread::index_1d().get() == 0 {
            let exact = unsafe { domain_exact(domain, value) };
            let lower = unsafe { domain_lower(domain, value) };
            let upper = if domain.directory.is_null() {
                let offset = (value as u64).wrapping_sub(domain.minimum as u64);
                lower + u32::from(value >= domain.minimum && offset < domain.span as u64)
            } else {
                lower + u32::from(exact != u32::MAX)
            };
            unsafe {
                result.write(exact);
                result.add(1).write(lower);
                result.add(2).write(upper);
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn domain_rank_translate(input: Input, domain: DomainView, output: *mut u32, invalid: *mut u32) {
        let mut row = thread::index_1d().get();
        let rows = unsafe { logical_len(input.column) } as usize;
        let stride = thread::gridDim_x() as usize * 256;
        while row < rows {
            let code = unsafe { domain_exact(domain, rank_input_value(input, row)) };
            if code == u32::MAX {
                unsafe { DeviceAtomicU32::from_ptr(invalid).store(1, AtomicOrdering::Relaxed) };
            }
            unsafe { output.add(row).write(code) };
            row += stride;
        }
    }
}
