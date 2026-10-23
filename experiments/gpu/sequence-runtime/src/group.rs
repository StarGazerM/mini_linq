//! GPU machine instance of segmented grouped monoid folds.
use crate::{Result, pointer, release, temporary};
use cq_gpu_runtime::{Buffer, Pool};
use gpu_device_columns::OutputCount;
use rccl_thrust::{DeviceBufferExt, arithmetic_range, select_flagged};
use std::sync::Arc;

/// A checked aggregate's machine representation. Extrema use a separate validity
/// column, so every i64 bit pattern remains a representable value.
#[derive(Clone, Copy)]
pub struct Aggregate {
    pub kind: u32,
    pub min: i64,
    pub max: i64,
}

pub struct GroupedOutput {
    values: Buffer<i64>,
    valid: Buffer<u8>,
    metadata: Arc<Buffer<OutputCount>>,
    statuses: Vec<Arc<Buffer<OutputCount>>>,
    width: usize,
}
impl GroupedOutput {
    pub fn with_decoder<F>(self, decode: F) -> GroupedRows<F> {
        GroupedRows {
            output: self,
            decode,
        }
    }
    pub fn checked_len(&self) -> Result<usize> {
        for status in &self.statuses {
            if status.to_host()?[0].error != 0 {
                return Err("Sequence grouped input failure".into());
            }
        }
        let metadata = self.metadata.to_host()?[0];
        if metadata.error != 0 {
            return Err("Sequence grouped fold arithmetic failure".into());
        }
        Ok(metadata.rows as usize)
    }
}

pub struct GroupedRows<F> {
    output: GroupedOutput,
    decode: F,
}
impl<F> GroupedRows<F> {
    pub fn checked_len(&self) -> Result<usize> {
        self.output.checked_len()
    }
    /// Publish a nonnullable grouped result as resident input columns.
    ///
    /// # Safety
    /// `slots` must describe `Row` in the physical key/aggregate column layout
    /// and select no nullable aggregate. The decoder is not applied: the grouped
    /// machine values must already be the row codec's integer representation.
    /// Each supplied bound must enclose that output column's complete domain;
    /// group keys inherit their input bounds, not the aggregate's value domain.
    pub unsafe fn into_columns<Row: gpu_device_columns::RowCodec<'static>>(
        self,
        slots: &[usize],
        bounds: &[gpu_device_columns::Bounds],
    ) -> Result<cq_gpu_runtime::DeviceColumns<'static, Row>> {
        use gpu_device_columns::{Codec, ColumnView, DeviceColumns, Dictionary,
            EncodedColumn, SourceView, Table};
        let output = self.output;
        if Row::WIDTH != slots.len() || bounds.len() != slots.len() || slots.iter().any(|&slot| slot >= output.width) {
            return Err("grouped publication schema width mismatch".into());
        }
        let pool = output.values.pool().clone();
        // fold allocates exactly one row per group; metadata retains the same
        // count plus arithmetic failures. Neither values nor count are read back.
        let rows = output.values.len() / output.width;
        let mut columns = Vec::with_capacity(slots.len());
        for (&slot, &bounds) in slots.iter().zip(bounds) {
            let mut source = SourceView::from(ColumnView {
                headers: std::ptr::null(),
                data: output.values.ptr().wrapping_add(if rows == 0 { 0 } else { slot }).cast(),
                len: output.values.len() as u64,
                count: std::ptr::null(),
                codec: Codec::Plain as u32,
            });
            source.stride = output.width as u64;
            columns.push(Arc::new(unsafe {
                EncodedColumn::from_gather_source::<_, i64>(
                    pool.clone(), source, std::ptr::null(), rows,
                    output.metadata.clone(), 0, bounds,
                )?
            }));
        }
        let views: Vec<_> = columns.iter().map(|column| unsafe { column.view() }).collect();
        let table = Table {
            columns,
            views: Arc::new(Buffer::from_host(pool.clone(), &views)?),
            metadata: output.metadata,
            capacity: rows,
            bounds: bounds.to_vec(),
            statuses: output.statuses,
        };
        Ok(unsafe { DeviceColumns::from_parts(table, Arc::new(Dictionary::default()), pool) })
    }
    pub fn to_host<Row>(&self) -> Result<Vec<Row>>
    where
        F: Fn(&[i64], &[u8]) -> Result<Row>,
    {
        let rows = self.checked_len()?;
        let values = self.output.values.to_host()?;
        let valid = self.output.valid.to_host()?;
        (0..rows)
            .map(|row| {
                let range = row * self.output.width..(row + 1) * self.output.width;
                (self.decode)(&values[range.clone()], &valid[range])
            })
            .collect()
    }
}

/// Reduce canonical valuation arguments. Each input row contains the key fields
/// followed by one evaluated argument per aggregate (count uses one).
///
/// # Safety
/// The initialized row-major input must be canonical over complete valuations,
/// not merely over aggregate arguments. Optional 0/1 flags and all live rows
/// must be initialized on the same device and ready on `pool`'s stream.
pub unsafe fn fold(
    pool: Arc<Pool>,
    input: &Buffer<i64>,
    live: Option<&cuda_core::DeviceBuffer<u32>>,
    rows: usize,
    keys: usize,
    aggregates: &[Aggregate],
    statuses: Vec<Arc<Buffer<OutputCount>>>,
) -> Result<GroupedOutput> {
    let width = keys
        .checked_add(aggregates.len())
        .ok_or("group width overflow")?;
    if width == 0
        || rows > u32::MAX as usize
        || input.len() < rows.checked_mul(width).ok_or("group size overflow")?
        || !Arc::ptr_eq(input.pool(), &pool)
        || live.is_some_and(|live| live.len() < rows)
    {
        return Err("invalid grouped input extent or pool".into());
    }
    let stream = pool.stream();
    let module = unsafe { group_kernels::load(pool.context())? };
    let thrust = rccl_thrust::cuda::Program::load(pool.context())?;
    let exec = thrust.par_in(stream, pool.resource())?;
    let cub = rccl_cub::cuda::Program::load(pool.context())?;
    let launch = cub.on(stream)?;
    let mut order = temporary::<u32>(stream, rows)?;
    let rows = if let Some(live) = live {
        let mut count = temporary::<u64>(stream, 1)?;
        if rows != 0 {
            select_flagged(
                &exec,
                arithmetic_range(0u32, 1, rows),
                live.view(),
                order.view_mut(),
                &mut count,
            )?;
        }
        let selected = if rows == 0 {
            0
        } else {
            count.to_host_vec(stream)?[0] as usize
        };
        let selected_order = temporary::<u32>(stream, selected)?;
        if selected != 0 {
            unsafe {
                cuda_core::memory::memcpy_dtod_async(
                    selected_order.cu_deviceptr(),
                    order.cu_deviceptr(),
                    selected * std::mem::size_of::<u32>(),
                    stream.cu_stream(),
                )?;
            }
        }
        release(stream, order)?;
        release(stream, count)?;
        order = selected_order;
        selected
    } else {
        rows
    };
    let mut alternate_order = temporary::<u32>(stream, rows)?;
    let mut sort_keys = temporary::<u64>(stream, rows)?;
    let mut alternate_keys = temporary::<u64>(stream, rows)?;
    let mut offsets = temporary::<u32>(stream, rows.max(1))?;
    let flags = temporary::<u32>(stream, rows)?;
    let mut count = temporary::<u64>(stream, 1)?;
    let config = crate::config(rows.max(1));
    if rows != 0 {
        let mut scratch = if keys == 0 {
            None
        } else {
            Some(rccl_cub::device_radix_sort_pairs_buffer_workspace::<
                u64,
                u32,
            >(&launch, rows)?)
        };
        if live.is_none() {
            module.identity(
                stream,
                &module.prepare_identity(config)?,
                pointer(&order),
                rows as u32,
            )?;
        }
        // Stable least-significant-field passes produce lexicographic group order.
        for key in (0..keys).rev() {
            module.gather_key(
                stream,
                &module.prepare_gather_key(config)?,
                input.ptr(),
                pointer(&order),
                pointer(&sort_keys),
                rows as u32,
                width,
                key,
            )?;
            let mut pairs = rccl_cub::DoubleBuffer::paired(
                (&mut sort_keys, &mut order),
                (&mut alternate_keys, &mut alternate_order),
            )
            .ok_or("group radix extents differ")?;
            rccl_cub::device_radix_sort_pairs_buffer(
                &launch,
                &mut pairs,
                0..64,
                scratch.as_mut().unwrap(),
            )?;
            let swapped = !pairs.current_is_first();
            drop(pairs);
            if swapped {
                std::mem::swap(&mut order, &mut alternate_order);
                std::mem::swap(&mut sort_keys, &mut alternate_keys);
            }
        }
        module.boundaries(
            stream,
            &module.prepare_boundaries(config)?,
            input.ptr(),
            pointer(&order),
            pointer(&flags),
            rows as u32,
            width,
            keys,
        )?;
        select_flagged(
            &exec,
            arithmetic_range(0u32, 1, rows),
            flags.view(),
            offsets.view_mut(),
            &mut count,
        )?;
    }
    let groups = if rows == 0 {
        usize::from(keys == 0)
    } else {
        count.to_host_vec(stream)?[0] as usize
    };
    let values = Buffer::<i64>::zeroed(pool.clone(), groups * width)?;
    let valid = Buffer::<u8>::zeroed(pool.clone(), groups * width)?;
    let metadata = Arc::new(Buffer::from_host(
        pool.clone(),
        &[OutputCount {
            rows: groups as u64,
            error: 0,
            padding: 0,
        }],
    )?);
    if groups != 0 {
        let config = cuda_core::LaunchConfig1D::new(groups as u32, 256, 4096);
        for column in 0..width {
            let aggregate = if column < keys {
                Aggregate {
                    kind: 2,
                    min: i64::MIN,
                    max: i64::MAX,
                }
            } else {
                aggregates[column - keys]
            };
            module.reduce(
                stream,
                &module.prepare_reduce(config)?,
                input.ptr(),
                pointer(&order),
                pointer(&offsets),
                rows as u32,
                width,
                column,
                aggregate.kind,
                aggregate.min,
                aggregate.max,
                values.ptr(),
                valid.ptr(),
                metadata.ptr().cast::<u64>().cast::<u32>().wrapping_add(2),
            )?;
        }
    }
    for buffer in [order, alternate_order, offsets, flags] {
        release(stream, buffer)?;
    }
    for buffer in [sort_keys, alternate_keys, count] {
        release(stream, buffer)?;
    }
    Ok(GroupedOutput {
        values,
        valid,
        metadata,
        statuses,
        width,
    })
}

#[cuda_device::cuda_module]
mod group_kernels {
    use cuda_device::{kernel, launch_bounds, launch_contract};
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1))]
    pub fn identity(order: *mut u32, rows: u32) {
        unsafe {
            let mut i = cuda_device::thread::index_1d().get() as u32;
            while i < rows {
                *order.add(i as usize) = i;
                i += cuda_device::thread::gridDim_x() * 256;
            }
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1))]
    pub fn gather_key(
        input: *const i64,
        order: *const u32,
        keys: *mut u64,
        rows: u32,
        width: usize,
        column: usize,
    ) {
        unsafe {
            let mut i = cuda_device::thread::index_1d().get() as u32;
            while i < rows {
                *keys.add(i as usize) =
                    (*input.add(*order.add(i as usize) as usize * width + column) as u64)
                        ^ (1u64 << 63);
                i += cuda_device::thread::gridDim_x() * 256;
            }
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1))]
    pub fn boundaries(
        input: *const i64,
        order: *const u32,
        flags: *mut u32,
        rows: u32,
        width: usize,
        keys: usize,
    ) {
        unsafe {
            let mut i = cuda_device::thread::index_1d().get() as u32;
            while i < rows {
                let mut different = i == 0;
                if i != 0 {
                    let a = *order.add(i as usize) as usize;
                    let b = *order.add(i as usize - 1) as usize;
                    let mut k = 0;
                    while k < keys {
                        different |= *input.add(a * width + k) != *input.add(b * width + k);
                        k += 1;
                    }
                }
                *flags.add(i as usize) = different as u32;
                i += cuda_device::thread::gridDim_x() * 256;
            }
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1),dynamic_shared_range=(4096,4096))]
    pub fn reduce(
        input: *const i64,
        order: *const u32,
        offsets: *const u32,
        rows: u32,
        width: usize,
        column: usize,
        kind: u32,
        min: i64,
        max: i64,
        output: *mut i64,
        valid: *mut u8,
        error: *mut u32,
    ) {
        unsafe {
            let lane = cuda_device::thread::threadIdx_x() as usize;
            let group = cuda_device::thread::blockIdx_x() as usize;
            let begin = if rows == 0 { 0 } else { *offsets.add(group) };
            let end = if group + 1 == cuda_device::thread::gridDim_x() as usize {
                rows
            } else {
                *offsets.add(group + 1)
            };
            let mut value = if kind == 2 {
                i64::MAX
            } else if kind == 3 {
                i64::MIN
            } else {
                0
            };
            let mut position = begin as usize + lane;
            while position < end as usize {
                let next = *input.add(*order.add(position) as usize * width + column);
                if kind == 2 {
                    value = value.min(next);
                } else if kind == 3 {
                    value = value.max(next);
                } else {
                    match value.checked_add(next) {
                        Some(sum) => value = sum,
                        None => crate::device::flag_overflow(error),
                    }
                }
                position += 256;
            }
            let shared = cuda_device::DynamicSharedArray::<i64, 16>::get_raw().cast::<i64>();
            *shared.add(lane) = value;
            cuda_device::thread::sync_threads();
            let mut stride = 128;
            while stride != 0 {
                if lane < stride {
                    let a = *shared.add(lane);
                    let b = *shared.add(lane + stride);
                    *shared.add(lane) = if kind == 2 {
                        a.min(b)
                    } else if kind == 3 {
                        a.max(b)
                    } else {
                        match a.checked_add(b) {
                            Some(sum) => sum,
                            None => {
                                crate::device::flag_overflow(error);
                                0
                            }
                        }
                    };
                }
                cuda_device::thread::sync_threads();
                stride /= 2;
            }
            if lane == 0 {
                let value = *shared;
                let present = kind < 2 || begin != end;
                if present && (value < min || value > max) {
                    crate::device::flag_overflow(error);
                }
                *output.add(group * width + column) = if present { value } else { 0 };
                *valid.add(group * width + column) = present as u8;
            }
        }
    }
}

#[cfg(test)]
mod publication_tests {
    use super::*;

    #[test]
    fn resident_group_publication_preserves_values_count_and_failure() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        let input = Buffer::from_host(pool.clone(), &[1i64, 3, 1, 7, 2, 5])?;
        let aggregate = Aggregate { kind: 1, min: i64::MIN, max: i64::MAX };
        for error in [0, 1] {
            let status = Arc::new(Buffer::from_host(pool.clone(), &[OutputCount {
                rows: 3, error, padding: 0,
            }])?);
            let grouped = unsafe { fold(pool.clone(), &input, 3, 1, &[aggregate], vec![status])? };
            let published = unsafe {
                grouped.with_decoder(()).into_columns::<(i64, i64)>(&[1, 0], &[
                    gpu_device_columns::Bounds::Inclusive { min: i64::MIN, max: i64::MAX },
                    gpu_device_columns::Bounds::Inclusive { min: 1, max: 2 },
                ])?
            };
            assert_eq!(published.capacity(), 2);
            assert_eq!(published.storage().bounds[1], gpu_device_columns::Bounds::Inclusive { min: 1, max: 2 });
            if error == 0 {
                assert_eq!(published.to_host()?, [(10, 1), (5, 2)]);
            } else {
                assert!(published.to_host().is_err());
            }
        }
        Ok(())
    }
}
