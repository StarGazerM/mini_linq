#![cfg(target_os = "macos")]

use cq_mac::{IndexSpec, MacContext, MacRun, Payload, memory};

const PAIRS: IndexSpec = IndexSpec {
    key_column: 0,
    payload: Payload::Column(1),
};

fn batch(len: usize, seed: u32) -> Vec<(u32, u32)> {
    (0..len)
        .map(|index| {
            let index = index as u32;
            let key = match index % 7 {
                0 => 0,
                1 | 2 => u32::MAX,
                3 => 0x8000_0000,
                4 => 0x000f_ffff,
                5 => 0x0010_0000,
                _ => index.wrapping_mul(104729).wrapping_add(seed),
            };
            // Repeated identical pairs as well as equal-key unequal payloads.
            let value = if index % 7 <= 2 { seed } else { index ^ seed };
            (key, value)
        })
        .collect()
}

fn assert_exact(run: &MacRun, input: &[(u32, u32)]) {
    let mut expected = input.to_vec();
    expected.sort_by_key(|row| row.0);
    assert_eq!(run.len(), input.len());
    assert_eq!(run.is_empty(), input.is_empty());
    assert_eq!(run.rows().collect::<Vec<_>>(), expected);
    assert_eq!(
        run.rows().rev().collect::<Vec<_>>(),
        expected.iter().rev().copied().collect::<Vec<_>>()
    );
    for key in [0, 1, 0x000f_ffff, 0x0010_0000, 0x8000_0000, u32::MAX] {
        let values = input
            .iter()
            .filter(|row| row.0 == key)
            .map(|row| row.1)
            .collect::<Vec<_>>();
        assert_eq!(run.contains(&key), !values.is_empty());
        assert_eq!(run.lookup(&key).collect::<Vec<_>>(), values);
    }
}

#[test]
fn pinned_clone_and_partially_consumed_cursor_survive_rebuilds() {
    let context = MacContext::new().unwrap();
    let original = batch(4097, 13);
    let source = context
        .relation_u32(original.len(), |i| [original[i].0, original[i].1])
        .unwrap();
    let old = context.build(&source, PAIRS, None).unwrap();
    let pinned = old.clone();
    let mut cursor = pinned.lookup(&u32::MAX);
    let values = original
        .iter()
        .filter(|row| row.0 == u32::MAX)
        .map(|row| row.1)
        .collect::<Vec<_>>();
    assert_eq!(cursor.next(), values.first().copied());
    drop(old);

    // MLX may recycle temporaries, but cannot recycle a live immutable run.
    for (round, len) in [0, 17, 32771, 8193, 1, 4097, 0].into_iter().enumerate() {
        let input = batch(len, round as u32 + 100);
        let source = context
            .relation_u32(input.len(), |i| [input[i].0, input[i].1])
            .unwrap();
        let run = context.build(&source, PAIRS, None).unwrap();
        assert_exact(&run, &input);
        assert_exact(&pinned, &original);
        drop(run);
        memory::clear_cache().unwrap();
    }
    assert_eq!(cursor.collect::<Vec<_>>(), values[1..]);
    assert_exact(&pinned, &original);

    // Final-owner drop releases the old output too; subsequent smaller and
    // larger batches still expose only their own initialized rows/directory.
    drop(pinned);
    for len in [8193, 3, 65537, 0] {
        let input = batch(len, 999);
        let source = context
            .relation_u32(input.len(), |i| [input[i].0, input[i].1])
            .unwrap();
        let run = context.build(&source, PAIRS, None).unwrap();
        assert_exact(&run, &input);
    }
}

#[test]
fn allocator_churn_and_growth_preserve_live_outputs() {
    let context = MacContext::new().unwrap();
    let mut retained = Vec::new();
    for (round, len) in [0, 1, 2049, 8193, 37, 32771, 0, 8192]
        .into_iter()
        .enumerate()
    {
        let input = batch(len, round as u32);
        let source = context
            .relation_u32(input.len(), |i| [input[i].0, input[i].1])
            .unwrap();
        let run = context.build(&source, PAIRS, None).unwrap();
        assert_exact(&run, &input);
        if round % 2 == 0 {
            retained.push((run, input));
        } else {
            drop(run);
        }
        let next = batch(len / 2, round as u32 + 500);
        let source = context
            .relation_u32(next.len(), |i| [next[i].0, next[i].1])
            .unwrap();
        let replacement = context.build_on_cpu_radix(&source, PAIRS, None).unwrap();
        assert_exact(&replacement, &next);
        memory::clear_cache().unwrap();
        for (snapshot, original) in &retained {
            assert_exact(snapshot, original);
        }
    }
}

#[test]
fn completed_runs_outlive_context_and_merge_normally() {
    let first = batch(8193, 41);
    let second = batch(2049, 42);
    let (retained, clone, merged) = {
        let context = MacContext::new().unwrap();
        let source = context
            .relation_u32(first.len(), |i| [first[i].0, first[i].1])
            .unwrap();
        let retained = context.build(&source, PAIRS, None).unwrap();
        let clone = retained.clone();
        let source = context
            .relation_u32(second.len(), |i| [second[i].0, second[i].1])
            .unwrap();
        let newer = context.build_on_cpu(&source, PAIRS, None).unwrap();
        let merged = context.merge(&retained, &newer).unwrap();
        drop(newer);
        memory::clear_cache().unwrap();
        assert_exact(&retained, &first);
        (retained, clone, merged)
    };
    assert_exact(&retained, &first);
    drop(retained);
    assert_exact(&clone, &first);
    let combined = first.iter().chain(&second).copied().collect::<Vec<_>>();
    assert_exact(&merged, &combined);
}
