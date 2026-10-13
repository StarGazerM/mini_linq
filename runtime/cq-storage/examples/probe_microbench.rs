//! Non-gating exact scalar/batched comparison:
//! cargo run -p cq-storage --release --example probe_microbench
//! Setup and result checking are outside timing; both paths preserve bag order.
use cq_storage::{Build, HashIndex, IndexRead};
use std::{hint::black_box, time::Instant};

fn measure(mut run: impl FnMut() -> u64) -> (f64, u64) {
    let expected = black_box(run());
    let mut samples = [0.0; 7];
    for elapsed in &mut samples {
        let start = Instant::now();
        let result = black_box(run());
        *elapsed = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(result, expected);
    }
    samples.sort_by(f64::total_cmp);
    (samples[3], expected)
}

fn main() {
    for size in [2_048u64, 65_536, 1_048_576] {
        for multiplicity in [1u64, 4] {
            let index = HashIndex::build((0..size).flat_map(move |k| {
                (0..multiplicity).map(move |v| (k * 2, k + v))
            }));
            // Deterministic random order, half missing; no preordered locality.
            let mut state = 17u64;
            let keys: Vec<_> = (0..1_000_000).map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state % (size * 2)
            }).collect();
            black_box(index.lookup(&0).count());
            let scalar = || keys.iter().copied()
                .flat_map(|key| index.lookup(&key))
                .fold(0u64, u64::wrapping_add);
            let batched = || (&index).probe::<128, _, _>(keys.iter().copied(), |key| *key)
                .fold(0u64, |sum, (_, value)| sum.wrapping_add(value));
            let streamed = || (&index).probe::<1, _, _>(keys.iter().copied(), |key| *key)
                .fold(0u64, |sum, (_, value)| sum.wrapping_add(value));
            let (scalar_ms, checksum) = measure(scalar);
            let (batch_ms, batch_checksum) = measure(batched);
            let (stream_ms, stream_checksum) = measure(streamed);
            assert_eq!(checksum, stream_checksum);
            assert_eq!(checksum, batch_checksum);
            println!("keys={size} multiplicity={multiplicity} scalar_ms={scalar_ms:.3} stream_ms={stream_ms:.3} batch_ms={batch_ms:.3} batch_speedup={:.3} stream_speedup={:.3} checksum={checksum}", scalar_ms / batch_ms, scalar_ms / stream_ms);
        }
    }
}
