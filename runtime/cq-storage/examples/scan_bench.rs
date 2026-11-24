//! Run with scan_bench.py: five lock-held interleaved pairs against Vec.
use cq_storage::scan::*;
use std::{hint::black_box, ops::Bound::Included, time::Instant};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let variant = &args[1];
    let operation = &args[2];
    let n = args
        .get(3)
        .map_or(4_194_304, |s| s.parse::<usize>().unwrap());
    let bits = args.get(4).map_or(16, |s| s.parse::<u32>().unwrap());
    let domain = 1u64 << bits;
    let low = domain / 6;
    let high = domain / 3;
    let values: Vec<u64> = (0..n).map(|i| (i as u64 * 17) % domain).collect();
    let column = ScanColumn::<u64, Bytes>::new(&values, BlockRows::new(16384).unwrap(), false);
    let predicate = Predicate::Window {
        lo: Included(low),
        hi: Included(high),
    };
    let expected: u64 = values
        .iter()
        .enumerate()
        .filter(|(_, v)| (low..=high).contains(*v))
        .map(|(i, _)| i as u64)
        .sum();
    let mut mask = vec![0u8; n];
    let sum: u64 = values.iter().sum();
    let mut out = [0u64; VECTOR_ROWS];
    let mut samples = Vec::new();
    for iteration in 0..26 {
        let start = Instant::now();
        match (variant.as_str(), operation.as_str()) {
            ("vec", "encode") => {
                let copy = black_box(&values).clone();
                assert_eq!(copy.len(), n);
                black_box(copy);
            }
            ("bytes", "encode") => {
                let c = ScanColumn::<u64, Bytes>::new(
                    black_box(&values),
                    BlockRows::new(16384).unwrap(),
                    false,
                );
                assert_eq!(c.len(), n);
                black_box(c);
            }
            ("vec", "decode") => {
                let actual: u64 = black_box(&values).iter().copied().sum();
                assert_eq!(actual, sum);
                black_box(actual);
            }
            ("bytes", "decode") => {
                let mut actual = 0u64;
                for (b, h) in column.headers().iter().enumerate() {
                    for v in 0..(h.rows as usize).div_ceil(VECTOR_ROWS) {
                        let len = column.decode_vector(b, v, &mut out);
                        actual += black_box(&out[..len]).iter().sum::<u64>();
                    }
                }
                assert_eq!(actual, sum);
                black_box(actual);
            }
            ("vec", "filter") => {
                let lo = black_box(low);
                let hi = black_box(high);
                let actual: u64 = black_box(&values)
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| (lo..=hi).contains(*v))
                    .map(|(i, _)| i as u64)
                    .sum();
                assert_eq!(actual, expected);
                black_box(actual);
            }
            ("bytes", "filter") => {
                let mut actual = 0u64;
                column.filter(black_box(&predicate), |i| actual += i as u64);
                assert_eq!(actual, expected);
                black_box(actual);
            }
            ("vec", "mask") => {
                let lo = black_box(low);
                let hi = black_box(high);
                for (selected, &v) in mask.iter_mut().zip(black_box(&values)) {
                    *selected = ((v >= lo) & (v <= hi)) as u8;
                }
                black_box(&mask);
            }
            ("bytes", "mask") => {
                column.filter_mask(black_box(&predicate), black_box(&mut mask));
            }
            _ => panic!("expected vec|bytes encode|decode|filter"),
        }
        if iteration >= 5 {
            samples.push(start.elapsed().as_nanos());
        }
        if operation == "mask" {
            let actual: u64 = mask
                .iter()
                .enumerate()
                .map(|(i, &v)| i as u64 * v as u64)
                .sum();
            assert_eq!(actual, expected);
        }
    }
    println!(
        "{{\"rows\":{n},\"payload_bytes\":{},\"samples_ns\":{samples:?}}}",
        column.payload().len()
    );
}
