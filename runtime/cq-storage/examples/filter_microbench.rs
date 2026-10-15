//! Non-gating microbenchmark: cargo run -p cq-storage --release --example filter_microbench
//! One 1.5M-row host (sparse i64 keys, as TPC-H order keys) filtered by a
//! semijoin key set retaining 1%, 5%, 25% or 100% of rows, then probed with
//! 2M keys drawn from the host. Compares the compact rebuild over survivors
//! (the Filter realization) against an all-row id index whose lookups skip ids
//! the selection clears, measured here only as a local loop.
use cq_storage::{
    AdaptiveKeySet, Build, Column, Filter, HashIndex, I64Codec, IndexRead, Membership,
    PositionalRows, RowId,
};
use std::{hint::black_box, time::Instant};

const ROWS: usize = 1_500_000;
const PROBES: usize = 2_000_000;
type Host = Vec<(i64, i64)>;

#[derive(Clone, Copy)]
struct Key;
impl<'a> Column<&'a Host> for Key {
    type Key = i64;
    fn key(self, host: &'a Host, id: RowId) -> i64 {
        host[id.offset()].0
    }
}

fn rng(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn median(mut runs: Vec<f64>) -> f64 {
    runs.sort_by(f64::total_cmp);
    runs[runs.len() / 2]
}

fn time<T>(runs: usize, mut f: impl FnMut() -> T) -> (f64, T) {
    let mut times = Vec::new();
    let mut last = None;
    for _ in 0..runs {
        let start = Instant::now();
        last = Some(black_box(f()));
        times.push(start.elapsed().as_secs_f64() * 1e3);
    }
    (median(times), last.unwrap())
}

fn main() {
    // Sparse keys like TPC-H o_orderkey: 8 per 32, ascending.
    let host: Host = (0..ROWS as i64)
        .map(|i| ((i / 8) * 32 + i % 8, i))
        .collect();
    let mut state = 0x9e37_79b9_7f4a_7c15;
    let probes: Vec<i64> = (0..PROBES)
        .map(|_| host[(rng(&mut state) % ROWS as u64) as usize].0)
        .collect();
    let (full_build, full): (f64, HashIndex<i64, RowId>) = time(5, || {
        let index = HashIndex::build((&host).positions().map(|id| (host[id.offset()].0, id)));
        index.lookup(&-1).count();
        index
    });
    println!("all-row index build ({ROWS} rows): {full_build:.1} ms");
    println!(
        "{:>8} {:>10} {:>12} {:>12} {:>12} {:>12}",
        "kept", "filter ms", "compact bld", "compact prb", "masked prb", "hits"
    );
    for percent in [1u64, 5, 25, 100] {
        let kept: Vec<i64> = host
            .iter()
            .map(|&(k, _)| k)
            .filter(|&k| (k as u64).wrapping_mul(0x2545_f491_4f6c_dd1d) % 100 < percent)
            .collect();
        let keys: AdaptiveKeySet<i64, I64Codec, 67_108_864> =
            AdaptiveKeySet::build(kept.iter().map(|&k| (k, ())));
        let (filter_ms, selection) = time(5, || (&host).filter(Key, &Membership(&keys)));
        let (compact_build, compact): (f64, HashIndex<i64, &i64>) = time(5, || {
            let index: HashIndex<i64, &i64> = selection.build(|(k, v)| (*k, v));
            index.lookup(&-1).count();
            index
        });
        let (compact_probe, hits) = time(5, || {
            probes
                .iter()
                .map(|k| compact.lookup(k).copied().sum::<i64>())
                .sum::<i64>()
        });
        let (masked_probe, masked_hits) = time(5, || {
            probes
                .iter()
                .map(|k| {
                    full.lookup(k)
                        .filter(|&id| selection.contains(id))
                        .map(|id| host[id.offset()].1)
                        .sum::<i64>()
                })
                .sum::<i64>()
        });
        assert_eq!(hits, masked_hits, "realizations disagree");
        println!(
            "{:>7}% {filter_ms:>10.1} {compact_build:>12.1} {compact_probe:>12.1} {masked_probe:>12.1} {hits:>12}",
            percent
        );
    }
}
