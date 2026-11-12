mod support;

use cq_gpu_sequence_runtime::{
    group::{self, Aggregate, SegmentedGroup},
    Buffer, Geometry, Pool, Result,
};
use support::AUX;

fn aggregate(kind: u32) -> Aggregate {
    Aggregate {
        kind,
        min: i64::MIN,
        max: i64::MAX,
    }
}

#[test]
fn segmented_integer_monoids_and_empty_global_identity() -> Result<()> {
    let pool = Pool::new(0, 0, 0)?;
    // Already canonical valuations; equal arguments retain multiplicity.
    let data = Buffer::from_host(
        pool.clone(),
        &[2i64, 1, -7, -7, -7, 1, 1, 5, 5, 5, 1, 1, 5, 5, 5],
    )?;
    let aggregates = [aggregate(0), aggregate(1), aggregate(2), aggregate(3)];
    let result = unsafe {
        SegmentedGroup::fold(pool.clone(), &data, None, 3, &[true], &aggregates, vec![], &AUX)?
    }
    .with_decoder(|v: &[i64], valid: &[u8]| -> Result<_> { Ok((v.to_vec(), valid.to_vec())) });
    assert_eq!(
        // DECODE P: diagnostic grouped monoids publish host values and validity for exact assertions.
        result.to_host()?,
        vec![
            (vec![1, 2, 10, 5, 5], vec![1; 5]),
            (vec![2, 1, -7, -7, -7], vec![1; 5])
        ]
    );
    let live = cuda_core::DeviceBuffer::from_host(pool.stream(), &[0u32, 1, 0])?;
    let selected = unsafe {
        SegmentedGroup::fold(
            pool.clone(),
            &data,
            Some(&live),
            3,
            &[true],
            &aggregates,
            vec![],
            &AUX,
        )?
    }
    .with_decoder(|v: &[i64], _: &[u8]| -> Result<_> { Ok(v.to_vec()) });
    // DECODE P: diagnostic selected aggregate publishes its final host answer.
    assert_eq!(selected.to_host()?, vec![vec![1, 1, 5, 5, 5]]);
    let empty = Buffer::<i64>::zeroed(pool.clone(), 0)?;
    let result =
        unsafe { group::fold(pool.clone(), &empty, None, 0, &[], &aggregates, vec![], &AUX)? }
            .with_decoder(|v: &[i64], valid: &[u8]| -> Result<_> {
                Ok((v.to_vec(), valid.to_vec()))
            });
    assert_eq!(
        // DECODE P: diagnostic empty global aggregate publishes identity values and validity.
        result.to_host()?,
        vec![(vec![0, 0, 0, 0], vec![1, 1, 0, 0])]
    );
    let result = unsafe {
        SegmentedGroup::fold(pool.clone(), &empty, None, 0, &[true], &aggregates, vec![], &AUX)?
    }
    .with_decoder(|v: &[i64], _: &[u8]| -> Result<_> { Ok(v.to_vec()) });
    // DECODE P: diagnostic empty keyed aggregate exposes its empty host answer.
    assert!(result.to_host()?.is_empty());
    let data = Buffer::from_host(pool.clone(), &[u32::MAX as i64, 1])?;
    let result = unsafe {
        group::fold(
            pool.clone(),
            &data,
            None,
            2,
            &[],
            &[Aggregate {
                kind: 1,
                min: 0,
                max: u32::MAX as i64,
            }],
            vec![],
            &AUX,
        )?
    }
    .with_decoder(|v: &[i64], _: &[u8]| -> Result<_> { Ok(v.to_vec()) });
    // DECODE P: diagnostic u32 aggregate overflow checks logical answer publication rejection.
    assert!(result.to_host().is_err());
    let data = Buffer::from_host(pool.clone(), &[i64::MAX, 1])?;
    let result = unsafe {
        group::fold(
            pool.clone(),
            &data,
            None,
            2,
            &[],
            &[aggregate(1)],
            vec![],
            &AUX,
        )?
    }
    .with_decoder(|v: &[i64], _: &[u8]| -> Result<_> { Ok(v.to_vec()) });
    // DECODE P: diagnostic i64 aggregate overflow checks logical answer publication rejection.
    assert!(result.to_host().is_err());
    // The reduce kernel stages 256 lanes in 4096 shared bytes; other records are rejected.
    let mut narrow = AUX;
    narrow.group_reduce = Geometry {
        block_threads: 256,
        shared_bytes: 2048,
    };
    assert!(narrow.validate().is_err());
    assert!(
        unsafe { group::fold(pool, &data, None, 2, &[], &[aggregate(1)], vec![], &narrow) }.is_err()
    );
    Ok(())
}

#[test]
fn masked_multiblock_multikey_fold_refuses_wide_domains_but_sorts_computed_keys() -> Result<()> {
    const ROWS: usize = 4096;
    const WIDE: i64 = 1 << 40;
    let pool = Pool::new(0, 0, 0)?;
    let mut data = Vec::with_capacity(ROWS * 3);
    let mut live = Vec::with_capacity(ROWS);
    let mut expected = std::collections::BTreeMap::<(i64, i64), i64>::new();
    for row in 0..ROWS {
        let selected = row % 2 == 0;
        let first = if selected { (row / 2 % 6) as i64 - 3 } else { i64::MIN };
        let second = if row / 4 % 2 == 0 { -WIDE } else { WIDE };
        let value = (row % 7) as i64 - 3;
        data.extend_from_slice(&[first, second, value]);
        live.push(u32::from(selected));
        if selected {
            *expected.entry((first, second)).or_default() += value;
        }
    }
    // 2048 live rows span multiple bounds blocks for each of the two keys.
    // Dead rows carry a wide first key: their extrema must not affect either
    // the domain refusal decision or the recorded narrow-key range.
    let data = Buffer::from_host(pool.clone(), &data)?;
    let live = cuda_core::DeviceBuffer::from_host(pool.stream(), &live)?;
    let error = unsafe {
        group::fold(
            pool.clone(), &data, Some(&live), ROWS, &[true, true],
            &[aggregate(1)], vec![], &AUX,
        )
    }.err().expect("live wide domain key must be refused");
    assert!(error.to_string().contains("u32 code domain"));
    let output = unsafe {
        group::fold(
            pool, &data, Some(&live), ROWS, &[true, false],
            &[aggregate(1)], vec![], &AUX,
        )?
    };
    let domains = output.key_domains();
    assert_eq!(domains.len(), 2);
    assert_eq!((domains[0].minimum, domains[0].bound, domains[0].bits), (-3, 6, 3));
    assert_eq!((domains[1].minimum, domains[1].bound, domains[1].bits),
        (-WIDE, (1u128 << 41) + 1, 42));
    let output = output.with_decoder(|values: &[i64], valid: &[u8]| -> Result<_> {
        assert_eq!(valid, &[1, 1, 1]);
        Ok(((values[0], values[1]), values[2]))
    });
    // DECODE P: diagnostic multi-key grouped sum publishes exact masked answers.
    assert_eq!(output.to_host()?, expected.into_iter().collect::<Vec<_>>());
    Ok(())
}
