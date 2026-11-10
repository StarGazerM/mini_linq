//! GPU-RadixSourceOrder: compare destination codes with an independent scalar
//! oracle, not with the source decoder whose intermediate this path removes.
use std::sync::Arc;

use gpu_device_columns::{Buffer, Codec, ColumnView, EncodedColumn, Pool, SourceView};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[cuda_host::cuda_module]
mod kernels {
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};
    use gpu_device_columns::SourceView;

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn source_order_codes(
        source: SourceView,
        positions: *const u32,
        count: u32,
        bits: u32,
        signed: bool,
        output: *mut u64,
    ) {
        let index = thread::index_1d().get() as u32;
        if index < count {
            unsafe {
                *output.add(index as usize) = gpu_datalog_runtime::radix_run::source_order::order_code(
                    source,
                    *positions.add(index as usize) as u64,
                    bits,
                    signed,
                );
            }
        }
    }
}

fn check(
    pool: &Arc<Pool>,
    source: SourceView,
    values: &[u64],
    positions: &[u32],
    bits: u32,
    signed: bool,
) -> Result {
    let output = Buffer::<u64>::zeroed(pool.clone(), positions.len())?;
    if !positions.is_empty() {
        let positions_device = Buffer::from_host(pool.clone(), positions)?;
        let module = unsafe { kernels::load(pool.context())? };
        module.source_order_codes(
            pool.stream(),
            &module.prepare_source_order_codes(cuda_core::LaunchConfig1D::new(
                positions.len().div_ceil(256) as u32,
                256,
                0,
            ))?,
            source,
            positions_device.ptr(),
            positions.len() as u32,
            bits,
            signed,
            output.ptr(),
        )?;
    }
    let modulus = 1u128 << bits;
    let sign = if signed { modulus / 2 } else { 0 };
    let expected = positions
        .iter()
        .map(|&position| {
            let scalar = values[position as usize] as u128 % modulus;
            ((scalar + sign) % modulus) as u64
        })
        .collect::<Vec<_>>();
    assert_eq!(output.to_host()?, expected);
    Ok(())
}

#[test]
fn vector_frames_preserve_complete_scalar_order_domains() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    for bits in [8, 16, 32, 64] {
        let min = -(1i128 << (bits - 1));
        let span = 1u128 << bits;
        let values = (0..3081u32)
            .map(|row| {
                let local = row % 1024;
                let value = match row / 1024 {
                    0 => min + i128::from(local % 127),
                    1 => i128::from(local % 63),
                    2 => (span as i128 / 2 - 1) - i128::from(local % 127),
                    _ => [min, -1, 0, span as i128 / 2 - 1][local as usize % 4],
                };
                value as i64
            })
            .collect::<Vec<_>>();
        let bits_values = values.iter().map(|&value| value as u64).collect::<Vec<_>>();
        let mut positions = (0..values.len() as u32).rev().collect::<Vec<_>>();
        positions.extend([0, 1023, 1024, 2047, 2048, 3072, 0]);
        for codec in [Codec::Plain, Codec::For, Codec::Delta] {
            let column = EncodedColumn::from_host(pool.clone(), &values, codec)?;
            check(&pool, unsafe { column.view() }.into(), &bits_values, &positions, bits, true)?;
            check(&pool, unsafe { column.view() }.into(), &bits_values, &[], bits, true)?;
        }
    }
    for codec in [Codec::For, Codec::Delta] {
        let values = vec![i64::MIN; 1030];
        let column = EncodedColumn::from_host(pool.clone(), &values, codec)?;
        check(
            &pool,
            unsafe { column.view() }.into(),
            &vec![i64::MIN as u64; values.len()],
            &[0, 1023, 1024, 1029],
            64,
            true,
        )?;
    }
    let values = vec![u32::MIN, u32::MAX, 1, 1 << 31, (1 << 31) - 1];
    let column = EncodedColumn::from_host(pool.clone(), &values, Codec::For)?;
    check(&pool, unsafe { column.view() }.into(), &values.iter().map(|&v| u64::from(v)).collect::<Vec<_>>(), &[4, 0, 1, 3, 2], 32, false)
}

#[test]
fn packed_frames_cross_words_and_preserve_unsigned_extrema() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let residuals = [0, 1, u64::MAX, 1 << 63, (1 << 63) - 1];
    let words = residuals
        .iter()
        .flat_map(|&residual| {
            let packed = (residual as u128) << 61;
            [packed as u64, (packed >> 64) as u64]
        })
        .collect::<Vec<_>>();
    let data = Buffer::from_host(pool.clone(), &words)?;
    for (base, signed) in [(i64::MIN, true), (0, false)] {
        let source = SourceView {
            column: ColumnView {
                headers: std::ptr::null(),
                data: data.ptr(),
                len: residuals.len() as u64,
                count: std::ptr::null(),
                codec: Codec::Plain as u32,
            },
            stride: 2,
            packed: 1,
            shift: 61,
            mask: u64::MAX,
            base,
        };
        let values = residuals.iter().map(|&offset| (base as u64).wrapping_add(offset)).collect::<Vec<_>>();
        check(&pool, source, &values, &[4, 3, 2, 1, 0, 2], 64, signed)?;
        let constant = SourceView { mask: 0, base: -37, ..source };
        check(&pool, constant, &[(-37i64) as u64; 5], &[4, 0, 3], 8, true)?;
    }
    Ok(())
}

#[test]
fn native_strided_sources_keep_selection_and_unsigned_bits() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let values = [u64::MAX, 0, 1 << 63, 7, (1 << 63) - 1];
    let rows = values.iter().flat_map(|&v| [123, v, 456]).collect::<Vec<_>>();
    let data = Buffer::from_host(pool.clone(), &rows)?;
    let mut source = SourceView::from(ColumnView {
        headers: std::ptr::null(),
        data: unsafe { data.ptr().add(1) },
        len: values.len() as u64,
        count: std::ptr::null(),
        codec: Codec::Plain as u32,
    });
    source.stride = 3;
    check(&pool, source, &values, &[3, 1, 4, 0, 2, 0], 64, false)
}
