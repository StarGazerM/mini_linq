//! Existing Access quotations over different native Rust representations.
//! The 64-key range cutoff is illustrative, not a cost-based optimizer.
//! This checks local exact access and conservative prefiltering, not YA scheduling.
use mini_linq::access_to_distinct_iter;
use std::collections::{BTreeSet, HashSet};

type Answer = BTreeSet<(i32,)>;

fn hash_access(probe: &[i32], keys: &HashSet<i32>) -> Answer {
    access_to_distinct_iter! {
        answer(x) => ((x,)) :-
        P(x) => for (x,) in (probe.iter().copied().map(|x| (x,))),
        S(x) => if (keys.contains(&x));
    }
    .collect()
}

fn bitmap_access(probe: &[i32], base: i32, bits: u64) -> Answer {
    access_to_distinct_iter! {
        answer(x) => ((x,)) :-
        P(x) => for (x,) in (probe.iter().copied().map(|x| (x,))),
        S(x) => if ({
            let offset = i64::from(x) - i64::from(base);
            (0..64).contains(&offset) && bits & (1u64 << offset as u32) != 0
        });
    }
    .collect()
}

fn compact_bitmap(build: &[i32]) -> Option<(i32, u64)> {
    let Some(&base) = build.iter().min() else {
        return Some((0, 0));
    };
    let top = *build.iter().max().unwrap();
    // Widen before subtraction: even i32::MIN .. i32::MAX is safe.
    if i64::from(top) - i64::from(base) >= 64 {
        return None;
    }
    let bits = build.iter().fold(0, |bits, &key| {
        bits | (1u64 << (i64::from(key) - i64::from(base)) as u32)
    });
    Some((base, bits))
}

fn adaptive_access(build: &[i32], probe: &[i32]) -> (&'static str, Answer) {
    // Observation, allocation and branching are ordinary runtime Rust.
    // Each branch keeps its native representation and uses the same Access shape.
    match compact_bitmap(build) {
        Some((base, bits)) => ("bitmap", bitmap_access(probe, base, bits)),
        None => ("hash", hash_access(probe, &build.iter().copied().collect())),
    }
}

fn coarse_bit(key: i32) -> u64 {
    1u64 << key.rem_euclid(8) as u32
}

fn prefiltered_access(probe: &[i32], keys: &HashSet<i32>, coarse: u64) -> Answer {
    // This deliberately collision-prone 8-bucket filter is not an exact S lookup.
    // MayS denotes its candidate keys; the separate S guard establishes matches.
    access_to_distinct_iter! {
        answer(x) => ((x,)) :-
        P(x) => for (x,) in (probe.iter().copied().map(|x| (x,))),
        MayS(x) => if (coarse & coarse_bit(x) != 0),
        S(x) => if (keys.contains(&x));
    }
    .collect()
}

fn check(build: &[i32], probe: &[i32], verbose: bool) -> (bool, usize) {
    // Independent reference uses direct input membership, with set-valued output.
    let expected: Answer = probe
        .iter()
        .copied()
        .filter(|key| build.contains(key))
        .map(|key| (key,))
        .collect();
    let keys: HashSet<_> = build.iter().copied().collect();
    assert_eq!(hash_access(probe, &keys), expected);
    let (choice, actual) = adaptive_access(build, probe);
    assert_eq!(actual, expected);

    let coarse = build.iter().fold(0, |bits, &key| bits | coarse_bit(key));
    let candidates: Vec<_> = probe
        .iter()
        .copied()
        .filter(|&key| coarse & coarse_bit(key) != 0)
        .collect();
    assert!(build.iter().all(|&key| coarse & coarse_bit(key) != 0));
    let false_positives = candidates.iter().filter(|key| !build.contains(key)).count();
    assert_eq!(prefiltered_access(probe, &keys, coarse), expected);
    if verbose {
        println!(
            "{choice}: build={}, probes={}, answer={}, candidates={}, false positives={}",
            build.len(),
            probe.len(),
            actual.len(),
            candidates.len(),
            false_positives
        );
    }
    (choice == "bitmap", false_positives)
}

fn main() {
    let cases: &[(&[i32], &[i32])] = &[
        (&[], &[]),
        (&[], &[-1, 0, 1]),
        (&[0], &[]),
        (&[-4, -4, 2], &[-4, -3, 2, 2, 60]),
        (&[0, 63], &[-1, 0, 63, 64]),
        (&[0, 64], &[-1, 0, 63, 64, 128]),
        (
            &[i32::MIN, i32::MAX],
            &[i32::MIN, i32::MIN + 1, -1, 0, i32::MAX],
        ),
        (&[-1], &[7, -1, 15]), // Force two false positives before exact checking.
    ];
    let mut bitmap_cases = 0;
    let mut false_positives = 0;
    for &(build, probe) in cases {
        let (bitmap, false_count) = check(build, probe, true);
        bitmap_cases += usize::from(bitmap);
        false_positives += false_count;
    }
    assert_eq!(check(&[-1], &[7, -1, 15], false).1, 2);
    let probes: Vec<_> = (-10..=10).chain([-4, 0, 3]).collect();
    for mask in 0u16..256 {
        let build: Vec<_> = (0..8)
            .filter(|bit| mask & (1 << bit) != 0)
            .map(|bit| bit - 4)
            .collect();
        let (bitmap, false_count) = check(&build, &probes, false);
        bitmap_cases += usize::from(bitmap);
        false_positives += false_count;
    }
    let count = cases.len() + 256;
    assert!(bitmap_cases > 0 && bitmap_cases < count);
    assert!(false_positives > 0);
    println!(
        "PASS: {count} cases, {bitmap_cases} bitmap / {} hash choices, {false_positives} false-positive probes removed",
        count - bitmap_cases
    );
}
