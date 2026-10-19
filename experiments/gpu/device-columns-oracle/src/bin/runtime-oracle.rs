use cq_gpu_runtime::{Bounds, Codec, DeviceColumns, Dictionary, For, Plain, Result, Runtime};
use cuda_core::LaunchConfig1D;
use std::sync::Arc;

#[cuda_device::cuda_module]
mod checks {
    use cq_gpu_runtime::{column, membership_contains, table_len, write_output, MembershipView, OutputView, RunView, TableView};
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn membership(view: MembershipView, first: i64, rows: usize, output: *mut u32) {
        let mut i = thread::index_1d().get();
        let stride = thread::gridDim_x() as usize * 256;
        while i < rows {
            unsafe { output.add(i).write(membership_contains(view, first + i as i64) as u32); }
            i += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn copy_prefix(input: TableView, limit: u64, output: OutputView) {
        let count = unsafe { table_len(input) }.min(limit);
        let mut row = thread::index_1d().get() as u64;
        if row == 0 {
            unsafe { output.count.write(count); }
        }
        while row < count {
            unsafe { write_output(output, 0, row, column(input, 0, row)); }
            row += thread::gridDim_x() as u64 * 256;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn run_matches(view: RunView, first: i64, rows: usize, output: *mut u32) {
        let i = thread::index_1d().get();
        if i < rows {
            let matches = unsafe { cq_gpu_runtime::blocked_run::point(view.rank, first + i as i64) }
                .map_or(0, |(begin, end)| end - begin);
            unsafe { output.add(i).write(matches); }
        }
    }
}

fn escaped_column(runtime: &Runtime) -> Result<()> {
    let mut dictionary = Dictionary::default();
    let input = runtime.upload_owned_rows(&[(-7i64,), (-3,), (4,)], &mut dictionary, &[Codec::Plain])?;
    let mut execution = runtime.execution();
    let table = execution.bind_columns(&input)?;
    let answer: DeviceColumns<'_, (i64,), For> = execution.into_columns(table, &[0])?;
    let count = Arc::downgrade(&answer.storage().metadata);
    let column = answer.storage().columns[0].clone();
    execution.finish()?;
    drop(answer);
    drop(execution);
    drop(input);
    assert!(count.upgrade().is_some(), "escaped encoded column lost its count owner");
    assert_eq!(column.to_host()?, [-7, -3, 4]);
    drop(column);
    assert!(count.upgrade().is_none(), "escaped count owner leaked");
    Ok(())
}

fn derived_bounds(runtime: &Runtime) -> Result<()> {
    let mut dictionary = Dictionary::default();
    let module = unsafe { checks::load(runtime.context())? };
    let copy = module.prepare_copy_prefix(LaunchConfig1D::new(1, 256, 0))?;
    let probe = module.prepare_run_matches(LaunchConfig1D::new(1, 256, 0))?;
    let result = unsafe { cq_gpu_runtime::Buffer::uninitialized(runtime.pool().clone(), 14)? };
    for len in [0, 3] {
        let rows = [(-7i64,), (-3,), (4,)];
        let input = runtime.upload_owned_rows(&rows[..len], &mut dictionary, &[Codec::For])?;
        let mut execution = runtime.execution();
        let source = execution.bind_columns(&input)?;
        let bounds = execution.column_bounds(source, 0)?;
        if len == 0 {
            assert_eq!(bounds, Bounds::Empty);
        } else {
            assert_eq!(bounds, Bounds::Inclusive { min: -7, max: 4 });
        }
        let view = execution.table_view(source)?;
        let output = execution.execute_bounded(&[bounds], 16, |stream, output| {
            unsafe { module.copy_prefix(stream, &copy, view, u64::MAX, output)?; }
            Ok(())
        })?;
        let output = execution.dedup(output)?;
        let answer: DeviceColumns<'_, (i64,), For> = execution.into_columns(output, &[0])?;
        drop(execution);
        drop(input);
        let mut next = runtime.execution();
        let source = next.bind_columns(&answer)?;
        assert_eq!(next.column_bounds(source, 0)?, bounds);
        let run = next.prepare_run(source, 0)?;
        unsafe { module.run_matches(runtime.stream(), &probe, next.run_view(run)?, -8, 14, result.ptr())?; }
        next.finish()?;
        let actual = result.to_host()?;
        for (i, &matches) in actual.iter().enumerate() {
            assert_eq!(matches, (len != 0 && [-7, -3, 4].contains(&(i as i64 - 8))) as u32);
        }
        let mut actual = answer.to_host()?;
        actual.sort_unstable();
        assert_eq!(actual, rows[..len]);
    }
    Ok(())
}

fn membership_domains(runtime: &Runtime) -> Result<()> {
    let mut dictionary = Dictionary::default();
    let module = unsafe { checks::load(runtime.context())? };
    let copy = module.prepare_copy_prefix(LaunchConfig1D::new(1, 256, 0))?;
    let probe = module.prepare_membership(LaunchConfig1D::new(1, 256, 0))?;
    let result = unsafe { cq_gpu_runtime::Buffer::uninitialized(runtime.pool().clone(), 14)? };
    // Tight live domain, deliberately broad sound lineage. This must reduce
    // extrema on-device rather than reject a bitmap that really fits.
    let input = runtime.upload_owned_rows(
        &[(-7i64,), (-3,), (4,), (i64::MIN,), (i64::MAX,)], &mut dictionary, &[Codec::Plain],
    )?;
    for limit in [0, 3, 5] {
        let mut execution = runtime.execution();
        let source = execution.bind_columns(&input)?;
        let bounds = execution.column_bounds(source, 0)?;
        let view = execution.table_view(source)?;
        let filtered = execution.execute_bounded(&[bounds], 5, |stream, output| {
            unsafe { module.copy_prefix(stream, &copy, view, limit, output)?; }
            Ok(())
        })?;
        let membership = execution.prepare_membership(filtered, 0, 8)?;
        unsafe { module.membership(runtime.stream(), &probe, execution.membership_view(membership)?, -8, 14, result.ptr())?; }
        execution.finish()?;
        let actual = result.to_host()?;
        if limit != 5 {
            for (i, &present) in actual.iter().enumerate() {
                assert_eq!(present != 0, limit != 0 && [-7, -3, 4].contains(&(i as i64 - 8)));
            }
        } else {
            assert!(actual.iter().all(|&present| present == 0));
        }
        let answer: DeviceColumns<'_, (i64,), Plain> = execution.into_columns(filtered, &[0])?;
        if limit == 5 {
            assert!(answer.to_host().unwrap_err().to_string().contains("CQ_GPU_MEMBERSHIP_DOMAIN"));
        } else {
            assert_eq!(answer.to_host()?.len(), limit as usize);
        }
    }
    // Capacity errors must remain observable after a compressed projection.
    let mut execution = runtime.execution();
    let source = execution.bind_columns(&input)?;
    let bounds = execution.column_bounds(source, 0)?;
    let view = execution.table_view(source)?;
    let output = execution.execute_bounded(&[bounds], 1, |stream, output| {
        unsafe { module.copy_prefix(stream, &copy, view, 3, output)?; }
        Ok(())
    })?;
    let answer: DeviceColumns<'_, (i64,), For> = execution.into_columns(output, &[0])?;
    assert!(answer.to_host().unwrap_err().to_string().contains("CQ_GPU_OUTPUT_CAPACITY"));
    execution.finish()?;
    Ok(())
}

fn membership_benchmark(runtime: &Runtime, rows: usize) -> Result<()> {
    let mut dictionary = Dictionary::default();
    let first = -(rows as i64);
    let values: Vec<_> = (0..rows).map(|i| (first + 2 * i as i64,)).collect();
    let input = runtime.upload_owned_rows(&values, &mut dictionary, &[Codec::For])?;
    drop(values);
    let module = unsafe { checks::load(runtime.context())? };
    let tested = rows * 2 + 2;
    let result = unsafe { cq_gpu_runtime::Buffer::uninitialized(runtime.pool().clone(), tested)? };
    let launch = module.prepare_membership(LaunchConfig1D::new(tested.div_ceil(256).min(568) as u32, 256, 0))?;
    for iteration in 0..6 {
        let mut execution = runtime.execution();
        let table = execution.bind_columns(&input)?;
        let membership = execution.prepare_membership(table, 0, rows.div_ceil(4) + 8)?;
        let timings = execution.finish()?;
        unsafe { module.membership(runtime.stream(), &launch, execution.membership_view(membership)?, first - 1, tested, result.ptr())?; }
        let actual = result.to_host()?;
        for (i, &present) in actual.iter().enumerate() {
            assert_eq!(present != 0, i > 0 && i < rows * 2 && i % 2 == 1, "membership query {i}");
        }
        let checked: DeviceColumns<'_, (i64,), Plain> = execution.into_columns(table, &[0])?;
        gpu_device_columns::checked_count(checked.storage())?;
        println!("membership rows={rows} iteration={iteration} build_ms={:.6} wall_ms={:.6} exact=true", timings.build_ms, timings.wall_ms);
    }
    Ok(())
}

fn main() -> Result<()> {
    let runtime = Runtime::with_pool(0, 64 << 20, 512 << 20)?;
    if std::env::args().any(|arg| arg == "--membership") {
        membership_benchmark(&runtime, 4_000_000)?;
    } else if std::env::args().any(|arg| arg == "--bounds") {
        derived_bounds(&runtime)?;
    } else {
        escaped_column(&runtime)?;
        derived_bounds(&runtime)?;
        membership_domains(&runtime)?;
        println!("runtime oracle escaped-column, resident indexed outputs, empty/negative bounds, membership budgets, capacity errors exact=true");
    }
    Ok(())
}
