// Keep this generic radix instantiation separate from tests loading the
// library's non-generic CUDA modules: cuda-oxide 57eacecc shadows their anchor.
mod support;

use cq_gpu_sequence_runtime::{Codec, ColumnDescriptor, EncodedColumn, Pool, Result};
use gpu_datalog_runtime::RadixRunParams;
use std::sync::Arc;

#[test]
fn radix_source_layout_uses_measured_field_and_sort_bits() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    let keys = [[17u32, 18, 17], [0, 2, 7]].iter().map(|values| {
        let encoded = Arc::new(EncodedColumn::from_host(pool.clone(), values, Codec::For)?);
        // The upload initialized exactly these immutable rows.
        unsafe { ColumnDescriptor::new(encoded, values.len()) }
    }).collect::<Result<Vec<_>>>()?;
    let params = RadixRunParams { ratio: 4, radix_bits: 0, merge_tile_rows: 1024, block_threads: 256 };
    let layout = cq_gpu_sequence_runtime::radix_layout::<(u32, u32), (), u64, u64, ()>(
        &pool, params, &keys, &[], None, support::AUX.selection,
    )?;
    assert_eq!(layout.packing().packed_fields().map(|field| field.bits).collect::<Vec<_>>(), [5, 3]);
    assert_eq!(layout.snapshot().active_bits(), 8);
    // DECODE P: measured radix-source fixture publishes exact rows to its host oracle.
    assert_eq!(layout.to_rows()?, [vec![17, 0], vec![17, 7], vec![18, 2]]);
    Ok(())
}
