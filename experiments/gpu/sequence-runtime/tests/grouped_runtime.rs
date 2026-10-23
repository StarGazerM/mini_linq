use cq_gpu_sequence_runtime::{
    Buffer, Pool, Result,
    group::{self, Aggregate},
};

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
    let result = unsafe { group::fold(pool.clone(), &data, None, 3, 1, &aggregates, vec![])? }
        .with_decoder(|v: &[i64], valid: &[u8]| -> Result<_> { Ok((v.to_vec(), valid.to_vec())) });
    assert_eq!(
        result.to_host()?,
        vec![
            (vec![1, 2, 10, 5, 5], vec![1; 5]),
            (vec![2, 1, -7, -7, -7], vec![1; 5])
        ]
    );
    let live = Buffer::from_host(pool.clone(), &[0u32, 1, 0])?;
    let selected =
        unsafe { group::fold(pool.clone(), &data, Some(&live), 3, 1, &aggregates, vec![])? }
            .with_decoder(|v: &[i64], _: &[u8]| -> Result<_> { Ok(v.to_vec()) });
    assert_eq!(selected.to_host()?, vec![vec![1, 1, 5, 5, 5]]);
    let empty = Buffer::<i64>::zeroed(pool.clone(), 0)?;
    let result = unsafe { group::fold(pool.clone(), &empty, None, 0, 0, &aggregates, vec![])? }
        .with_decoder(|v: &[i64], valid: &[u8]| -> Result<_> { Ok((v.to_vec(), valid.to_vec())) });
    assert_eq!(
        result.to_host()?,
        vec![(vec![0, 0, 0, 0], vec![1, 1, 0, 0])]
    );
    let result = unsafe { group::fold(pool.clone(), &empty, None, 0, 1, &aggregates, vec![])? }
        .with_decoder(|v: &[i64], _: &[u8]| -> Result<_> { Ok(v.to_vec()) });
    assert!(result.to_host()?.is_empty());
    let data = Buffer::from_host(pool.clone(), &[u32::MAX as i64, 1])?;
    let result = unsafe {
        group::fold(
            pool.clone(),
            &data,
            None,
            2,
            0,
            &[Aggregate {
                kind: 1,
                min: 0,
                max: u32::MAX as i64,
            }],
            vec![],
        )?
    }
    .with_decoder(|v: &[i64], _: &[u8]| -> Result<_> { Ok(v.to_vec()) });
    assert!(result.to_host().is_err());
    let data = Buffer::from_host(pool.clone(), &[i64::MAX, 1])?;
    let result = unsafe { group::fold(pool, &data, None, 2, 0, &[aggregate(1)], vec![])? }
        .with_decoder(|v: &[i64], _: &[u8]| -> Result<_> { Ok(v.to_vec()) });
    assert!(result.to_host().is_err());
    Ok(())
}
