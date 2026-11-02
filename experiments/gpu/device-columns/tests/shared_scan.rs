use cq_storage::{
    scan::{format::*, *},
    OrderKey,
};
use gpu_device_columns::{scan::DeviceScanColumn, Pool};
use std::{ops::Bound, sync::Arc};
fn declared<T: OrdinalDecode, F: Family>(values: &[T], domain: Domain) -> ScanColumn<T, F> {
    let mut builder =
        ScanColumnBuilder::with_domain(BlockRows::new(1024).unwrap(), true, true, domain).unwrap();
    builder.extend(values.iter().copied());
    builder.finish()
}
fn domain() -> Domain {
    Domain {
        id: DomainId {
            namespace: 7,
            version: 1,
        },
        carrier: Width::W64,
        equality: true,
        order: true,
    }
}
fn exercise<F: Family + EncodedPoint + EncodedWindow>(
    pool: &Arc<Pool>,
    values: &[i64],
    rows: u32,
    summaries: bool,
) {
    let mut builder = ScanColumnBuilder::<i64, F>::with_domain(
        BlockRows::new(rows).unwrap(),
        summaries,
        summaries,
        domain(),
    )
    .unwrap();
    builder.extend(values.iter().copied());
    let host = Arc::new(builder.finish());
    for live in [values.len(), values.len().saturating_sub(1), 0] {
        let device = DeviceScanColumn::upload(pool.clone(), host.clone(), domain(), live).unwrap();
        assert_eq!(device.download_payload().unwrap(), host.payload());
        let bytes = device.transfer_bytes();
        assert_eq!(bytes.payload, host.payload().len());
        assert_eq!(bytes.headers, host.headers().len() * 24);
        assert_eq!(
            bytes.vector_summaries,
            host.vector_summaries().map_or(0, |s| s.len() * 16)
        );
        let lane = pool.lane().unwrap();
        let view = device.view_on(lane).unwrap();
        let snapshot = view.snapshot();
        drop(device);
        let predicates = [
            Predicate::Point(i64::MIN.ordinal()),
            Predicate::Point(i64::MAX.ordinal()),
            Predicate::Point(0i64.ordinal()),
            Predicate::Point(123456789i64.ordinal()),
            Predicate::Window {
                lo: Bound::Unbounded,
                hi: Bound::Unbounded,
            },
            Predicate::Window {
                lo: Bound::Excluded(u64::MAX),
                hi: Bound::Unbounded,
            },
            Predicate::Window {
                lo: Bound::Unbounded,
                hi: Bound::Excluded(0),
            },
            Predicate::Window {
                lo: Bound::Excluded((-3i64).ordinal()),
                hi: Bound::Included(256i64.ordinal()),
            },
            Predicate::Membership(vec![]),
            Predicate::Membership(vec![
                i64::MIN.ordinal(),
                0i64.ordinal(),
                255i64.ordinal(),
                i64::MAX.ordinal(),
            ]),
        ];
        for predicate in predicates {
            let mut expected = vec![0; values.len()];
            host.filter_mask(&predicate, &mut expected);
            let actual = view
                .filter_mask(domain().id, &predicate)
                .unwrap()
                .to_host()
                .unwrap();
            assert_eq!(actual, expected[..live]);
            let positions: Vec<usize> = actual
                .iter()
                .enumerate()
                .filter_map(|(i, &v)| (v != 0).then_some(i))
                .collect();
            let mut cpu_positions = vec![];
            host.filter(&predicate, |p| {
                if p < live {
                    cpu_positions.push(p)
                }
            });
            assert_eq!(positions, cpu_positions);
            let mut carrier = vec![0; positions.len()];
            host.checked_view(domain(), live)
                .unwrap()
                .read_positions(snapshot, &positions, &mut carrier)
                .unwrap();
            assert_eq!(
                carrier,
                positions
                    .iter()
                    .map(|&p| values[p].ordinal())
                    .collect::<Vec<_>>()
            );
        }
    }
}
#[test]
fn shared_upload_masks_positions_and_tails() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let mut corpus = vec![
        vec![],
        vec![i64::MIN],
        vec![i64::MAX],
        vec![7; 3073],
        vec![i64::MIN, -1, 0, 1, i64::MAX],
        (-1025..2051).collect(),
    ];
    for width in [255, 256, 65535, 65536] {
        corpus.push(vec![i64::MIN, i64::MIN + width, 0, width]);
    }
    for values in &corpus {
        for rows in [1024, 3072] {
            for summaries in [false, true] {
                exercise::<Plain>(&pool, values, rows, summaries);
                exercise::<Bytes>(&pool, values, rows, summaries);
            }
        }
    }
    gpu_device_columns::release_cached_kernels();
}
#[test]
fn retained_device_snapshot_and_failed_publication() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let old = Arc::new(declared::<i64, Bytes>(&[-1, 0, 1], domain()));
    let publication = DeviceScanColumn::upload(pool.clone(), old.clone(), domain(), 3).unwrap();
    let view = publication.view_on(pool.lane().unwrap()).unwrap();
    let retained_snapshot = view.snapshot();
    drop(publication);
    let newer = Arc::new(declared::<i64, Bytes>(&[9, 8, 7], domain()));
    assert_ne!(newer.snapshot(), retained_snapshot);
    assert!(DeviceScanColumn::upload(pool.clone(), newer, domain(), 4).is_err());
    assert_eq!(
        view.filter_mask(domain().id, &Predicate::Point(0i64.ordinal()))
            .unwrap()
            .to_host()
            .unwrap(),
        [0, 1, 0]
    );
    assert!(view
        .filter_mask(
            DomainId {
                namespace: 7,
                version: 2
            },
            &Predicate::Point(0i64.ordinal())
        )
        .is_err());
    let foreign = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let publication = DeviceScanColumn::upload(pool, old, domain(), 3).unwrap();
    assert!(publication.view_on(foreign).is_err());
    drop(view);
    gpu_device_columns::release_cached_kernels();
}

#[test]
fn narrow_upload_width_boundaries() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    let mut domain = domain();
    domain.carrier = Width::W32;
    for (span, width) in [
        (0, Width::W0),
        (255, Width::W8),
        (256, Width::W16),
        (65535, Width::W16),
        (65536, Width::W32),
        (u32::MAX, Width::W32),
    ] {
        let host = Arc::new(declared::<u32, Bytes>(&[0, span], domain));
        assert_eq!(host.headers()[0].width, width);
        let device = DeviceScanColumn::upload(pool.clone(), host, domain, 2).unwrap();
        let view = device.view_on(pool.clone()).unwrap();
        let actual = view
            .filter_mask(domain.id, &Predicate::Point(span as u64))
            .unwrap()
            .to_host()
            .unwrap();
        assert_eq!(actual, [u8::from(span == 0), 1]);
    }
    gpu_device_columns::release_cached_kernels();
}

fn queued_membership_masks<F: Family>(pool: &Arc<Pool>, summaries: bool) {
    // More than two complete capped-grid waves, plus a partial vector.
    let capacity = 2 * 568 * VECTOR_ROWS + 17;
    let values: Vec<i64> = (0..capacity)
        .map(|row| (row / VECTOR_ROWS * 1000 + row % 251) as i64)
        .collect();
    let mut builder = ScanColumnBuilder::<i64, F>::with_domain(
        BlockRows::new(3072).unwrap(),
        summaries,
        summaries,
        domain(),
    )
    .unwrap();
    builder.extend(values.iter().copied());
    let host = Arc::new(builder.finish());
    let selected = [
        values[0].ordinal(),
        values[568 * VECTOR_ROWS].ordinal(),
        values[capacity - 1].ordinal(),
    ];
    for live in [
        0,
        1,
        VECTOR_ROWS - 1,
        VECTOR_ROWS,
        VECTOR_ROWS + 1,
        568 * VECTOR_ROWS - 1,
        568 * VECTOR_ROWS,
        568 * VECTOR_ROWS + 1,
        capacity,
    ] {
        // No readback between upload, cross-lane lease and kernel submission.
        let publication =
            DeviceScanColumn::upload(pool.clone(), host.clone(), domain(), live).unwrap();
        let view = publication.view_on(pool.lane().unwrap()).unwrap();
        let mask = {
            let mut predicate = Predicate::Membership(selected.to_vec());
            let mask = view.filter_mask(domain().id, &predicate).unwrap();
            let Predicate::Membership(keys) = &mut predicate else {
                unreachable!()
            };
            keys.fill(u64::MAX);
            mask
        };
        // A second queued upload must not overwrite the first one's staging.
        let absent = view
            .filter_mask(domain().id, &Predicate::Membership(vec![u64::MAX]))
            .unwrap();
        drop(view);
        drop(publication);
        let actual = mask.to_host().unwrap();
        assert_eq!(
            actual,
            values[..live]
                .iter()
                .map(|value| selected.contains(&value.ordinal()) as u8)
                .collect::<Vec<_>>(),
            "live rows: {live}"
        );
        assert_eq!(absent.to_host().unwrap(), vec![0; live]);
    }
}

#[test]
fn async_membership_uploads_and_capped_grid_stride() {
    let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
    queued_membership_masks::<Plain>(&pool, false);
    queued_membership_masks::<Bytes>(&pool, true);
    gpu_device_columns::release_cached_kernels();
}
