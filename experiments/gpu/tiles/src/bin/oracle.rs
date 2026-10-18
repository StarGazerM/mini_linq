//! Deterministic GPU-versus-CPU oracles exercising cross-crate device helpers.

use cuda_core::{CudaContext, CudaStream, DeviceBuffer, LaunchConfig1D};
use cuda_device::{kernel, launch_bounds, launch_contract, thread, DisjointSlice, DynamicSharedArray};
use std::error::Error;

const THREADS: usize = gpu_tiles::BLOCK_THREADS as usize;
const ITEMS: usize = gpu_tiles::TILE_ITEMS as usize;
const POISON: u32 = 0xdead_beef;
type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;

#[cuda_device::cuda_module]
mod kernels {
    use super::*;

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (32, 32))]
    pub fn scan_case(input: &[u32], mut output: DisjointSlice<u32>) {
        let tid = thread::threadIdx_x() as usize;
        let base = thread::blockIdx_x() as usize * THREADS;
        let scratch = DynamicSharedArray::<u8>::get_raw() as *mut u32;
        let (exclusive, total) = gpu_tiles::scan(input[base + tid], scratch);
        // Immediate scratch reuse exercises the exit barrier in the library.
        let (again, again_total) = gpu_tiles::scan(exclusive, scratch);
        unsafe {
            *output.get_unchecked_mut((base + tid) * 4) = exclusive;
            *output.get_unchecked_mut((base + tid) * 4 + 1) = total;
            *output.get_unchecked_mut((base + tid) * 4 + 2) = again;
            *output.get_unchecked_mut((base + tid) * 4 + 3) = again_total;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (4128, 4128))]
    pub fn compact_case(input: &[u32], mut output: DisjointSlice<u32>) {
        let tid = thread::threadIdx_x() as usize;
        let tile = thread::blockIdx_x() as usize;
        let packed = DynamicSharedArray::<u8>::get_raw() as *mut u32;
        let scratch = unsafe { packed.add(ITEMS) };
        let mut i = tid;
        while i < ITEMS {
            unsafe { *packed.add(i) = POISON; }
            i += THREADS;
        }
        thread::sync_threads();
        let base = tile * ITEMS + tid * 4;
        let mask = [input[base] != 0, input[base + 1] != 0, input[base + 2] != 0, input[base + 3] != 0];
        let count = gpu_tiles::compact(mask, packed, scratch);
        let mut i = tid;
        while i < ITEMS {
            unsafe { *output.get_unchecked_mut(tile * (ITEMS + THREADS) + i) = *packed.add(i); }
            i += THREADS;
        }
        unsafe { *output.get_unchecked_mut(tile * (ITEMS + THREADS) + ITEMS + tid) = count; }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (4096, 4096))]
    pub fn sort_case(input: &[u32], sizes: &[u32], mut output: DisjointSlice<u32>) {
        let tid = thread::threadIdx_x() as usize;
        let tile = thread::blockIdx_x() as usize;
        let shared = DynamicSharedArray::<u8>::get_raw() as *mut u32;
        let n = sizes[tile];
        let mut i = tid;
        while i < ITEMS {
            unsafe { *shared.add(i) = if i < n as usize { input[tile * ITEMS + i] } else { POISON }; }
            i += THREADS;
        }
        // Padding may be written by a different thread than the initial load.
        thread::sync_threads();
        gpu_tiles::sort(shared, n);
        let mut i = tid;
        while i < ITEMS {
            unsafe { *output.get_unchecked_mut(tile * ITEMS + i) = *shared.add(i); }
            i += THREADS;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn owner_case(offsets: &[u32], positions: &[u32], n: u32, mut output: DisjointSlice<u32>) {
        let index = thread::index_1d();
        let i = index.get();
        if i < positions.len() {
            let parent = gpu_tiles::owner(offsets.as_ptr(), n, positions[i]);
            unsafe { *output.get_unchecked_mut(i) = parent; }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn merge_case(a: &[u32], b: &[u32], a_len: u32, b_len: u32, mut output: DisjointSlice<u32>) {
        let diagonal = thread::index_1d().get();
        if diagonal <= (a_len + b_len) as usize {
            let (i, j) = gpu_tiles::merge_path(a.as_ptr(), a_len, b.as_ptr(), b_len, diagonal as u32);
            unsafe {
                *output.get_unchecked_mut(diagonal * 2) = i;
                *output.get_unchecked_mut(diagonal * 2 + 1) = j;
            }
        }
    }
}

// Fixed nonzero xorshift seed; independent of platform and external rand crates.
struct Random(u64);
impl Random {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
}

fn check_scan(module: &kernels::LoadedModule, stream: &CudaStream, random: &mut Random) -> Result<usize> {
    let mut cases = vec![vec![0; THREADS], vec![1; THREADS], vec![u32::MAX; THREADS]];
    cases.push((0..THREADS as u32).collect());
    for index in [0, 1, 31, 32, 63, 127, 128, 254, 255] {
        let mut values = vec![0; THREADS];
        values[index] = u32::MAX;
        cases.push(values);
    }
    for valid in [1, 31, 32, 33, 255] {
        cases.push((0..THREADS).map(|i| if i < valid { random.next() % 100 } else { 0 }).collect());
    }
    for _ in 0..64 {
        cases.push((0..THREADS).map(|_| random.next()).collect());
    }
    let input: Vec<u32> = cases.iter().flatten().copied().collect();
    let device = DeviceBuffer::from_host(stream, &input)?;
    let mut output = DeviceBuffer::<u32>::zeroed(stream, input.len() * 4)?;
    let launch = module.prepare_scan_case(LaunchConfig1D::new(cases.len() as u32, THREADS as u32, 32))?;
    module.scan_case(stream, &launch, &device, &mut output)?;
    let actual = output.to_host_vec(stream)?;
    for (case, values) in cases.iter().enumerate() {
        let total = values.iter().fold(0u32, |sum, value| sum.wrapping_add(*value));
        let mut prefix = 0u32;
        let mut second_total = 0u32;
        for value in values {
            second_total = second_total.wrapping_add(prefix);
            prefix = prefix.wrapping_add(*value);
        }
        let mut prefix = 0u32;
        let mut second_prefix = 0u32;
        for (i, value) in values.iter().enumerate() {
            let start = (case * THREADS + i) * 4;
            assert_eq!(&actual[start..start + 4], &[prefix, total, second_prefix, second_total], "scan case={case} lane={i}");
            second_prefix = second_prefix.wrapping_add(prefix);
            prefix = prefix.wrapping_add(*value);
        }
    }
    Ok(cases.len())
}

fn check_compact(module: &kernels::LoadedModule, stream: &CudaStream, random: &mut Random) -> Result<usize> {
    let mut cases = vec![vec![0; ITEMS], vec![1; ITEMS]];
    cases.push((0..ITEMS).map(|i| (i % 2) as u32).collect());
    cases.push((0..ITEMS).map(|i| ((i + 1) % 2) as u32).collect());
    for index in [0, 1, 3, 4, 31, 32, 127, 128, 255, 256, 511, 512, 1022, 1023] {
        let mut values = vec![0; ITEMS];
        values[index] = 7; // All nonzero masks, not only 1, mean selected.
        cases.push(values);
    }
    for valid in [1, 3, 4, 31, 32, 33, 255, 256, 257, 1023] {
        cases.push((0..ITEMS).map(|i| (i < valid) as u32).collect());
    }
    for density in [1, 8, 32, 63] {
        for _ in 0..16 {
            cases.push((0..ITEMS).map(|_| ((random.next() % 64) < density) as u32).collect());
        }
    }
    let input: Vec<u32> = cases.iter().flatten().copied().collect();
    let device = DeviceBuffer::from_host(stream, &input)?;
    let stride = ITEMS + THREADS;
    let mut output = DeviceBuffer::<u32>::zeroed(stream, cases.len() * stride)?;
    let launch = module.prepare_compact_case(LaunchConfig1D::new(cases.len() as u32, THREADS as u32, 4128))?;
    module.compact_case(stream, &launch, &device, &mut output)?;
    let actual = output.to_host_vec(stream)?;
    for (case, values) in cases.iter().enumerate() {
        let expected: Vec<u32> = values.iter().enumerate().filter(|(_, v)| **v != 0).map(|(i, _)| i as u32).collect();
        let result = &actual[case * stride..(case + 1) * stride];
        assert_eq!(&result[..expected.len()], expected.as_slice(), "compact case={case}");
        assert!(result[expected.len()..ITEMS].iter().all(|v| *v == POISON), "compact wrote beyond count, case={case}");
        assert!(result[ITEMS..].iter().all(|v| *v == expected.len() as u32), "compact count case={case}");
    }
    Ok(cases.len())
}

fn check_sort(module: &kernels::LoadedModule, stream: &CudaStream, random: &mut Random) -> Result<usize> {
    let mut cases = Vec::<Vec<u32>>::new();
    for n in [0, 1, 2, 3, 4, 7, 8, 15, 16, 31, 32, 33, 63, 64, 65, 127, 128, 129, 255, 256, 257, 511, 512, 513, 1023, 1024] {
        cases.push((0..n as u32).collect());
        cases.push((0..n as u32).rev().collect());
        cases.push(vec![u32::MAX; n]);
        cases.push(vec![0; n]);
        cases.push((0..n).map(|i| if i % 2 == 0 { u32::MAX } else { 0 }).collect());
        cases.push((0..n).map(|_| random.next()).collect());
        cases.push((0..n).map(|_| random.next() % 7).collect());
    }
    for _ in 0..64 {
        let n = (random.next() as usize) % (ITEMS + 1);
        cases.push((0..n).map(|_| random.next()).collect());
    }
    let sizes: Vec<u32> = cases.iter().map(|v| v.len() as u32).collect();
    let mut input = vec![POISON; cases.len() * ITEMS];
    for (case, values) in cases.iter().enumerate() {
        input[case * ITEMS..case * ITEMS + values.len()].copy_from_slice(values);
    }
    let device = DeviceBuffer::from_host(stream, &input)?;
    let device_sizes = DeviceBuffer::from_host(stream, &sizes)?;
    let mut output = DeviceBuffer::<u32>::zeroed(stream, input.len())?;
    let launch = module.prepare_sort_case(LaunchConfig1D::new(cases.len() as u32, THREADS as u32, 4096))?;
    module.sort_case(stream, &launch, &device, &device_sizes, &mut output)?;
    let actual = output.to_host_vec(stream)?;
    for (case, values) in cases.iter().enumerate() {
        let mut expected = values.clone();
        expected.sort_unstable();
        expected.resize(values.len().max(1).next_power_of_two(), u32::MAX);
        expected.resize(ITEMS, POISON);
        assert_eq!(&actual[case * ITEMS..(case + 1) * ITEMS], expected.as_slice(), "sort case={case} n={}", values.len());
    }
    Ok(cases.len())
}

fn check_owner(module: &kernels::LoadedModule, stream: &CudaStream, random: &mut Random) -> Result<usize> {
    let mut cases = vec![
        vec![0], vec![5], vec![0, 0], vec![0, 1], vec![0, 0, 0, 0],
        vec![0, 0, 3, 3, 3, 7, 7], vec![4, 4, 5, 5, 10, 10],
        vec![0, u32::MAX], vec![0, 1, 1, u32::MAX, u32::MAX],
    ];
    for n in [1, 2, 31, 32, 255, 256, 1024] {
        for _ in 0..8 {
            let mut offsets = vec![0u32];
            for _ in 0..n {
                offsets.push(offsets.last().copied().unwrap() + random.next() % 5);
            }
            cases.push(offsets);
        }
    }
    for (case, offsets) in cases.iter().enumerate() {
        let n = offsets.len() - 1;
        let mut positions = vec![0, 1, u32::MAX - 1, u32::MAX];
        for offset in offsets {
            positions.push(*offset);
            if *offset > 0 { positions.push(offset - 1); }
            if *offset < u32::MAX { positions.push(offset + 1); }
        }
        // Exhaust every position for bounded random ranges; sample the u32 domain too.
        let end = *offsets.last().unwrap();
        if end < 8192 { positions.extend(0..=end); }
        positions.extend((0..64).map(|_| random.next()));
        let device_offsets = DeviceBuffer::from_host(stream, offsets)?;
        let device_positions = DeviceBuffer::from_host(stream, &positions)?;
        let mut output = DeviceBuffer::<u32>::zeroed(stream, positions.len())?;
        let launch = module.prepare_owner_case(LaunchConfig1D::new(positions.len().div_ceil(THREADS) as u32, THREADS as u32, 0))?;
        module.owner_case(stream, &launch, &device_offsets, &device_positions, n as u32, &mut output)?;
        let actual = output.to_host_vec(stream)?;
        for (i, position) in positions.iter().enumerate() {
            // Independent linear interval membership, not another upper_bound.
            let expected = offsets.windows(2).position(|pair| pair[0] <= *position && *position < pair[1]).unwrap_or(n);
            assert_eq!(actual[i], expected as u32, "owner case={case} position={position}");
        }
    }
    Ok(cases.len())
}

fn check_merge(module: &kernels::LoadedModule, stream: &CudaStream, random: &mut Random) -> Result<usize> {
    let mut cases = vec![
        (vec![], vec![]), (vec![], vec![0, 0, u32::MAX]), (vec![0, 0, u32::MAX], vec![]),
        (vec![0], vec![0]), (vec![u32::MAX], vec![u32::MAX]),
        (vec![1, 2, 3], vec![4, 5, 6]), (vec![4, 5, 6], vec![1, 2, 3]),
        (vec![0, 2, 4, 6], vec![1, 3, 5, 7]),
        (vec![0, 0, 2, 2, u32::MAX], vec![0, 1, 2, 2, u32::MAX]),
        (vec![7; 1024], vec![7; 1024]),
    ];
    for (a_len, b_len) in [(1, 1024), (1024, 1), (31, 33), (255, 257), (1024, 1024)] {
        for _ in 0..8 {
            let mut a: Vec<u32> = (0..a_len).map(|_| random.next() % 32).collect();
            let mut b: Vec<u32> = (0..b_len).map(|_| random.next() % 32).collect();
            a.sort_unstable();
            b.sort_unstable();
            cases.push((a, b));
        }
    }
    for _ in 0..64 {
        let a_len = random.next() as usize % (ITEMS + 1);
        let b_len = random.next() as usize % (ITEMS + 1);
        let mut a: Vec<u32> = (0..a_len).map(|_| random.next()).collect();
        let mut b: Vec<u32> = (0..b_len).map(|_| random.next()).collect();
        a.sort_unstable();
        b.sort_unstable();
        cases.push((a, b));
    }
    for (case, (a, b)) in cases.iter().enumerate() {
        // Keep a nonempty allocation for empty logical inputs; the helper must not read it.
        let device_a = DeviceBuffer::from_host(stream, if a.is_empty() { &[POISON][..] } else { a.as_slice() })?;
        let device_b = DeviceBuffer::from_host(stream, if b.is_empty() { &[POISON][..] } else { b.as_slice() })?;
        let diagonals = a.len() + b.len() + 1;
        let mut output = DeviceBuffer::<u32>::zeroed(stream, diagonals * 2)?;
        let launch = module.prepare_merge_case(LaunchConfig1D::new(diagonals.div_ceil(THREADS) as u32, THREADS as u32, 0))?;
        module.merge_case(stream, &launch, &device_a, &device_b, a.len() as u32, b.len() as u32, &mut output)?;
        let actual = output.to_host_vec(stream)?;
        let (mut i, mut j) = (0, 0);
        for diagonal in 0..diagonals {
            assert_eq!(&actual[diagonal * 2..diagonal * 2 + 2], &[i as u32, j as u32], "merge case={case} diagonal={diagonal}");
            if diagonal + 1 < diagonals {
                if i < a.len() && (j == b.len() || a[i] <= b[j]) { i += 1; } else { j += 1; }
            }
        }
    }
    Ok(cases.len())
}

fn main() -> Result<()> {
    let context = CudaContext::new(0)?;
    let stream = context.default_stream();
    let module = unsafe { kernels::load(&context)? };
    let mut random = Random(0xd1b5_4a32_d192_ed03);
    let scans = check_scan(&module, &stream, &mut random)?;
    let compactions = check_compact(&module, &stream, &mut random)?;
    let sorts = check_sort(&module, &stream, &mut random)?;
    let owners = check_owner(&module, &stream, &mut random)?;
    let merges = check_merge(&module, &stream, &mut random)?;
    stream.synchronize()?;
    println!("gpu-tiles oracle passed: scan={scans} compact={compactions} sort={sorts} owner={owners} merge_path={merges}; deterministic seed=0xd1b54a32d192ed03");
    Ok(())
}
