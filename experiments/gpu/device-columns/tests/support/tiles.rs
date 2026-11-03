//! Shared host fixtures for the tile test binaries. Each binary of this
//! package that defines its own `cuda_module` shadows the library's embedded
//! kernels (bundles are selected by package name), so device probes and
//! library-kernel tests live in separate binaries.

use cq_storage::{scan::{format::{Domain, DomainId}, *}, OrderKey};
use gpu_device_columns::tile::{self, Window};
use std::{ops::Bound, sync::Arc};

pub fn domain<T: OrdinalDecode>() -> Domain {
    Domain {
        id: DomainId { namespace: 71, version: 1 },
        carrier: T::WIDTH, equality: true, order: true,
    }
}
pub fn column<T: OrdinalDecode, F: Family>(values: &[T], rows: u32, blocks: bool, vectors: bool) -> Arc<ScanColumn<T, F>> {
    // with_domain uses with_summaries, and also supplies the mandatory S1 domain.
    let mut builder = ScanColumnBuilder::with_domain(BlockRows::new(rows).unwrap(), blocks, vectors, domain::<T>()).unwrap();
    builder.extend(values.iter().copied());
    Arc::new(builder.finish())
}
pub fn predicates() -> Vec<Predicate> {
    vec![
        Predicate::Point(0), Predicate::Point(u64::MAX),
        Predicate::Point(i64::MIN.ordinal()), Predicate::Point(i64::MAX.ordinal()),
        Predicate::Point(0i64.ordinal()), Predicate::Point(123_456_789),
        Predicate::Window { lo: Bound::Unbounded, hi: Bound::Unbounded },
        Predicate::Window { lo: Bound::Excluded(u64::MAX), hi: Bound::Unbounded },
        Predicate::Window { lo: Bound::Unbounded, hi: Bound::Excluded(0) },
        Predicate::Window { lo: Bound::Included(255), hi: Bound::Included(256) },
        Predicate::Window { lo: Bound::Excluded(255), hi: Bound::Excluded(65_536) },
        Predicate::Window { lo: Bound::Excluded((-3i64).ordinal()), hi: Bound::Included(256i64.ordinal()) },
        Predicate::Window { lo: Bound::Included(u64::MAX), hi: Bound::Included(0) },
    ]
}
pub fn classification<T: OrdinalDecode, F: Family>(host: &ScanColumn<T, F>, v: usize, p: &Predicate) -> u32 {
    if Window::from_predicate(p).unwrap().kind == 0 { return tile::DISQUALIFIED; }
    let bounds = host.vector_summaries().map(|s| s[v])
        .or_else(|| host.summaries().map(|s| s[v * VECTOR_ROWS / host.block_rows().get()]));
    match bounds.map(|bounds| p.classify(bounds)) {
        Some(Classification::Disqualified) => tile::DISQUALIFIED,
        Some(Classification::Qualified) => tile::QUALIFIED,
        _ => tile::AMBIVALENT,
    }
}

pub const TAILS: [usize; 15] = [0, 1, 255, 256, 1023, 1024, 1025, 1279, 1280, 2047, 4097, 4351, 4352, 5119, 8192];
pub fn signed_values() -> Vec<i64> {
    (0..8192).map(|row| match row % 11 {
        0 => i64::MIN, 1 => i64::MAX, 2 => -1, 3 => 0, 4 => 1,
        _ => row as i64 - 4096,
    }).collect()
}
/// Constant, u8, u16 and two native-width (span beyond u16) blocks.
pub fn mixed_values() -> Vec<u64> {
    let mut values = vec![17u64; 2048];
    for (base, span) in [(1000u64, 255u64), (1 << 32, 65_535), (0, u32::MAX as u64), (0, u64::MAX)] {
        values.extend((0..2048).map(|row| if row % 2 == 0 { base } else { base + span }));
    }
    values
}
/// More than two capped-grid waves: exercise grid stride and a live prefix.
pub fn summary_values() -> (usize, Vec<u64>) {
    let capacity = 2 * 568 * VECTOR_ROWS + 1023;
    (capacity, (0..capacity).map(|row| (row / VECTOR_ROWS * 10_000 + row % 251) as u64).collect())
}
