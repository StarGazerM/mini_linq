//! Non-gating microbenchmark: cargo run -p cq-storage --release --example adaptive_microbench
//! One million lookups per case; HashSet any_in is its exact scan oracle.
use cq_storage::{AdaptiveKeySet, ExactIntervalMembership, ExactMembership, I64Codec, NoDense};
use std::{collections::HashSet, hint::black_box, time::Instant};
fn measure(label: &str, mut lookup: impl FnMut(i64) -> bool) {
    let start = Instant::now();
    let mut hits = 0;
    for i in 0..1_000_000 {
        hits += usize::from(black_box(lookup(black_box(i % 8192))));
    }
    println!("{label}: {:?}; hits={hits}", start.elapsed());
}
fn main() {
    let keys: Vec<_> = (0i64..4096).step_by(2).collect();
    let dense = AdaptiveKeySet::<_, _, 4096>::from_keys(keys.iter().copied(), I64Codec);
    let hash = AdaptiveKeySet::<_, _, 4096>::from_keys(keys.iter().copied(), NoDense);
    let standard: HashSet<_> = keys.into_iter().collect();
    measure("dense contains", |k| dense.contains(&k));
    measure("adaptive hash contains", |k| hash.contains(&k));
    measure("std HashSet contains", |k| standard.contains(&k));
    measure("dense any_in", |k| dense.any_in(&k, &(k + 3)));
    measure("adaptive hash any_in", |k| hash.any_in(&k, &(k + 3)));
    measure("std HashSet any_in (exact scan)", |k| {
        standard.iter().any(|&x| k <= x && x <= k + 3)
    });
    println!(
        "retained bytes: dense={}, hash={} (ownership estimate)",
        dense.retained_bytes(),
        hash.retained_bytes()
    );
}
