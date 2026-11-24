//! Word runs against an independent host oracle: stable sort of row indices
//! by full multi-word key, grouping, and the directory definition evaluated
//! with 512-bit integers (`(key - min) >> shift`, which fits `radix_bits`).

use mccl::{
    Gpu, Result,
    words::{OffsetWidth, WordRunParams},
};

/// Little-endian 512-bit value of one key, whatever its lane layout.
fn value(key: &[u64], ms_first: bool) -> [u64; 8] {
    let mut out = [0; 8];
    for (lane, slot) in out.iter_mut().enumerate().take(key.len()) {
        *slot = if ms_first {
            key[key.len() - 1 - lane]
        } else {
            key[lane]
        };
    }
    out
}

fn compare(a: &[u64; 8], b: &[u64; 8]) -> std::cmp::Ordering {
    a.iter().rev().cmp(b.iter().rev())
}

fn subtract(a: [u64; 8], b: [u64; 8]) -> [u64; 8] {
    let (mut out, mut borrow) = ([0; 8], false);
    for i in 0..8 {
        let (x, o1) = a[i].overflowing_sub(b[i]);
        let (y, o2) = x.overflowing_sub(borrow as u64);
        out[i] = y;
        borrow = o1 || o2;
    }
    out
}

fn bit_length(a: [u64; 8]) -> u32 {
    (0..8)
        .rev()
        .find(|&i| a[i] != 0)
        .map_or(0, |i| i as u32 * 64 + 64 - a[i].leading_zeros())
}

fn shift_right(a: [u64; 8], shift: u32) -> u128 {
    let mut wide = 0u128;
    for bit in 0..128u32 {
        let source = shift + bit;
        if source < 512 && (a[(source / 64) as usize] >> (source % 64)) & 1 == 1 {
            wide |= 1 << bit;
        }
    }
    wide
}

fn check(gpu: &Gpu, keys: &[u64], params: WordRunParams) -> Result<()> {
    let lanes = params.lanes;
    let rows = keys.len() / lanes;
    let run = gpu.build_word_run(keys, params)?;
    if rows == 0 {
        assert!(run.is_none());
        return Ok(());
    }
    let run = run.expect("nonempty build publishes a run");
    let values: Vec<_> = keys
        .chunks(lanes)
        .map(|k| value(k, params.ms_first))
        .collect();
    let mut order: Vec<u32> = (0..rows as u32).collect();
    order.sort_by(|&a, &b| compare(&values[a as usize], &values[b as usize]));
    let (mut distinct, mut offsets) = (Vec::new(), Vec::new());
    for (position, &row) in order.iter().enumerate() {
        if position == 0 || values[order[position - 1] as usize] != values[row as usize] {
            distinct.extend_from_slice(&keys[row as usize * lanes..][..lanes]);
            offsets.push(position as u64);
        }
    }
    offsets.push(rows as u64);
    assert_eq!(run.len(), rows);
    assert_eq!(run.order(), order, "stable order {params:?} {keys:x?}");
    assert_eq!(run.keys(), distinct, "distinct keys");
    let got: Vec<u64> = (0..run.offsets().len())
        .map(|i| run.offsets().get(i))
        .collect();
    assert_eq!(got, offsets, "offsets");
    let bits = params.radix_bits;
    let directory = run.directory();
    if bits == 0 {
        assert!(directory.is_empty());
        return Ok(());
    }
    let groups: Vec<_> = distinct
        .chunks(lanes)
        .map(|k| value(k, params.ms_first))
        .collect();
    let (min, max) = (groups[0], groups[groups.len() - 1]);
    let shift = bit_length(subtract(max, min)).saturating_sub(bits);
    assert_eq!(directory.len(), (1 << bits) + 1);
    for bucket in 0..directory.len() as u128 {
        let expected = groups.partition_point(|k| shift_right(subtract(*k, min), shift) < bucket);
        assert_eq!(
            directory.get(bucket as usize),
            expected as u64,
            "bucket {bucket}"
        );
    }
    Ok(())
}

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// Duplicate-heavy keys with full-domain extremes in every lane.
fn keys(rows: usize, lanes: usize, seed: u64) -> Vec<u64> {
    let mut state = seed;
    (0..rows * lanes)
        .map(|i| match xorshift(&mut state) % 8 {
            0 => 0,
            1 => u64::MAX,
            2 | 3 => (i / lanes % 5) as u64,
            _ => xorshift(&mut state) >> (i % 3 * 20),
        })
        .collect()
}

fn params(lanes: usize, ms_first: bool, radix_bits: u32, offsets: OffsetWidth) -> WordRunParams {
    WordRunParams {
        lanes,
        ms_first,
        active_bits: lanes as u32 * 64,
        radix_bits,
        offsets,
    }
}

#[test]
fn layouts_match_the_oracle() -> Result<()> {
    let gpu = Gpu::new()?;
    // CUDA u64 (1 lane), u128 (2 lanes, LS first) and [u64; 3] (MS first).
    for (lanes, ms_first) in [(1, false), (2, false), (3, true), (8, true)] {
        for rows in [1, 2, 7, 255, 256, 257, 2048, 20_001] {
            for (bits, width) in [
                (0, OffsetWidth::U32),
                (4, OffsetWidth::U64),
                (12, OffsetWidth::U32),
            ] {
                let seed = 0x9e37_79b9_7f4a_7c15 ^ (rows * 31 + lanes) as u64;
                check(
                    &gpu,
                    &keys(rows, lanes, seed),
                    params(lanes, ms_first, bits, width),
                )?;
            }
        }
    }
    Ok(())
}

#[test]
fn ordered_single_and_equal_inputs() -> Result<()> {
    let gpu = Gpu::new()?;
    for lanes in [1, 2] {
        let ascending: Vec<u64> = (0..5000u64).flat_map(|i| vec![i / 3; lanes]).collect();
        let descending: Vec<u64> = ascending.chunks(lanes).rev().flatten().copied().collect();
        let equal = vec![7u64; 4096 * lanes];
        let single = vec![u64::MAX; lanes];
        for keys in [ascending, descending, equal, single] {
            check(&gpu, &keys, params(lanes, false, 8, OffsetWidth::U64))?;
        }
    }
    check(&gpu, &[], params(2, false, 8, OffsetWidth::U32))
}

#[test]
fn narrow_active_bits_sort_only_the_occupied_bytes() -> Result<()> {
    let gpu = Gpu::new()?;
    let mut state = 17;
    let keys: Vec<u64> = (0..30_000).map(|_| xorshift(&mut state) >> 24).collect(); // 40 bits
    for active_bits in [40, 64] {
        let params = WordRunParams {
            active_bits,
            ..params(1, false, 10, OffsetWidth::U32)
        };
        check(&gpu, &keys, params)?;
    }
    // Zero active bits: a single (zero) key, identity order.
    let zeros = vec![0u64; 1000];
    check(
        &gpu,
        &zeros,
        WordRunParams {
            active_bits: 0,
            ..params(1, false, 3, OffsetWidth::U32)
        },
    )
}

#[test]
fn invalid_parameters_are_rejected() -> Result<()> {
    let gpu = Gpu::new()?;
    let base = params(2, false, 4, OffsetWidth::U32);
    assert!(gpu.build_word_run(&[1, 2, 3], base).is_err(), "partial key");
    assert!(
        gpu.build_word_run(&[1; 18], params(9, false, 0, OffsetWidth::U32))
            .is_err()
    );
    assert!(
        gpu.build_word_run(
            &[1, 2],
            WordRunParams {
                active_bits: 129,
                ..base
            }
        )
        .is_err()
    );
    assert!(
        gpu.build_word_run(
            &[1, 2],
            WordRunParams {
                radix_bits: 31,
                ..base
            }
        )
        .is_err()
    );
    Ok(())
}
