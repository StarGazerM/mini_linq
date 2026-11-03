use cq_storage::OrderKey;
use cq_storage::scan::{format::*, *};
use std::{ops::Bound, sync::Arc};
fn declared<T: OrdinalDecode, F: Family>(
    values: &[T],
    rows: u32,
    domain: Domain,
) -> ScanColumn<T, F> {
    let mut builder =
        ScanColumnBuilder::with_domain(BlockRows::new(rows).unwrap(), true, true, domain).unwrap();
    builder.extend(values.iter().copied());
    builder.finish()
}

fn domain() -> Domain {
    Domain {
        id: DomainId {
            namespace: 7,
            version: 3,
        },
        carrier: Width::W64,
        equality: true,
        order: true,
    }
}
fn predicates() -> Vec<Predicate> {
    vec![
        Predicate::Point(i64::MIN.ordinal()),
        Predicate::Point(0i64.ordinal()),
        Predicate::Point(i64::MAX.ordinal()),
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
            lo: Bound::Excluded((-2i64).ordinal()),
            hi: Bound::Included(257i64.ordinal()),
        },
        Predicate::Window {
            lo: Bound::Included(u64::MAX),
            hi: Bound::Included(0),
        },
        Predicate::Membership(vec![]),
        Predicate::Membership(vec![i64::MIN.ordinal(), 0i64.ordinal(), i64::MAX.ordinal()]),
    ]
}
fn oracle(p: &Predicate, ordinal: u64) -> bool {
    match p {
        Predicate::Point(k) => ordinal == *k,
        Predicate::Membership(keys) => keys.contains(&ordinal),
        Predicate::Window { lo, hi } => {
            let low = match lo {
                Bound::Unbounded => true,
                Bound::Included(k) => ordinal >= *k,
                Bound::Excluded(k) => ordinal > *k,
            };
            let high = match hi {
                Bound::Unbounded => true,
                Bound::Included(k) => ordinal <= *k,
                Bound::Excluded(k) => ordinal < *k,
            };
            low && high
        }
    }
}
fn exercise<F: Family + EncodedPoint + EncodedWindow>(
    values: &[i64],
    block_rows: u32,
    block_summary: bool,
    vector_summary: bool,
) {
    let mut builder = ScanColumnBuilder::<i64, F>::with_domain(
        BlockRows::new(block_rows).unwrap(),
        block_summary,
        vector_summary,
        domain(),
    )
    .unwrap();
    builder.extend(values.iter().copied());
    let column = builder.finish();
    let view = column.checked_view(domain(), values.len()).unwrap();
    let imported = CheckedScanView::import(view.metadata(), view.payload()).unwrap();
    let m = imported.metadata();
    if let Some(origin) = m.summary_provenance {
        assert_eq!(origin.origin, SummaryOrigin::ValidatedImport);
    }
    let mut reconstructed = vec![];
    let mut vector = [0; VECTOR_ROWS];
    for (block, h) in column.headers().iter().enumerate() {
        for v in 0..(h.rows as usize).div_ceil(VECTOR_ROWS) {
            let count = column.decode_vector(block, v, &mut vector);
            reconstructed.extend_from_slice(&vector[..count]);
        }
    }
    assert_eq!(reconstructed, values);
    if let Some(summaries) = column.vector_summaries() {
        assert_eq!(summaries.len(), values.len().div_ceil(VECTOR_ROWS));
        for (summary, vector) in summaries.iter().zip(values.chunks(VECTOR_ROWS)) {
            assert_eq!(
                summary.lo,
                vector.iter().map(OrderKey::ordinal).min().unwrap()
            );
            assert_eq!(
                summary.hi,
                vector.iter().map(OrderKey::ordinal).max().unwrap()
            );
        }
    }
    for predicate in predicates() {
        let expected: Vec<u8> = values
            .iter()
            .map(|v| oracle(&predicate, v.ordinal()) as u8)
            .collect();
        let mut actual = vec![9; values.len()];
        imported
            .filter_mask(domain().id, &predicate, &mut actual)
            .unwrap();
        assert_eq!(actual, expected);
        column.filter_mask(&predicate, &mut actual);
        assert_eq!(actual, expected);
        let mut positions = vec![];
        column.filter(&predicate, |p| positions.push(p));
        assert_eq!(
            positions,
            expected
                .iter()
                .enumerate()
                .filter_map(|(i, &v)| (v != 0).then_some(i))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn shared_format_roundtrip_enclosure_translation() {
    let mut corpus = vec![
        vec![],
        vec![i64::MIN],
        vec![i64::MAX],
        vec![0; 3073],
        vec![i64::MIN, -1, 0, 1, i64::MAX],
        (-2000..3001).collect(),
    ];
    for span in [0, 1, 255, 256, 65535, 65536, i64::MAX] {
        corpus.push(vec![0, span]);
        corpus.push(vec![i64::MIN, i64::MIN + span]);
    }
    for values in &corpus {
        for rows in [1024, 3072] {
            for (block, vector) in [(false, false), (true, false), (false, true), (true, true)] {
                exercise::<Plain>(values, rows, block, vector);
                exercise::<Bytes>(values, rows, block, vector);
            }
        }
    }
}

#[test]
fn metadata_rejections_are_independent_of_encoder_choice() {
    let mut builder = ScanColumnBuilder::<i64, Bytes>::with_domain(
        BlockRows::new(2048).unwrap(),
        true,
        true,
        domain(),
    )
    .unwrap();
    builder.extend([0, 1, 255]);
    let source = builder.finish();
    let view = source.checked_view(domain(), 3).unwrap();
    let good = view.metadata();
    let reject = |m| assert!(CheckedScanView::import(m, view.payload()).is_err());
    let mut m = good;
    m.format.version = 2;
    reject(m);
    let mut m = good;
    m.format.vector_rows = 512;
    reject(m);
    let mut m = good;
    m.format.domain.id.version += 1;
    reject(m);
    let mut m = good;
    m.format.preservation.order = false;
    reject(m);
    let mut m = good;
    m.live_count = 4;
    reject(m);
    let mut m = good;
    m.capacity = 0;
    reject(m);
    let mut m = good;
    m.headers = &[];
    reject(m);
    let mut m = good;
    m.vector_summaries = Some(&[]);
    reject(m);
    let mut m = good;
    m.summary_provenance = None;
    reject(m);
    let unrelated = ScanColumn::<i64, Bytes>::new(&[0], BlockRows::new(1024).unwrap(), false);
    let mut m = good;
    m.summary_provenance.as_mut().unwrap().snapshot = unrelated.snapshot();
    reject(m);
    for change in 0..7 {
        let mut headers = good.headers.to_vec();
        match change {
            0 => headers[0].offset = u64::MAX,
            1 => headers[0].offset = 1,
            2 => headers[0].rows = 0,
            3 => headers[0].width = Width::W64,
            4 => headers[0].codec = CodecId::Plain,
            5 => headers[0].reserved[0] = 1,
            _ => headers[0].base = u64::MAX,
        }
        let mut m = good;
        m.headers = &headers;
        assert!(CheckedScanView::import(m, view.payload()).is_err());
    }
    let narrow = [MinMax {
        lo: 0i64.ordinal(),
        hi: 0i64.ordinal(),
    }];
    let mut m = good;
    m.vector_summaries = Some(&narrow);
    assert_eq!(
        CheckedScanView::import(m, view.payload()).unwrap_err(),
        FormatError::SummaryBounds
    );
    assert!(CheckedScanView::import(good, &view.payload()[..2]).is_err());
    // Bytes admits a lawful Plain realization; checking must not recompress or
    // require the selector's smaller FOR candidate for these same values.
    let plain = declared::<i64, Plain>(&[0, 1, 255], 2048, domain());
    let plain_view = plain.checked_view(domain(), 3).unwrap();
    let mut m = plain_view.metadata();
    m.format.family = FamilyId::Bytes;
    CheckedScanView::import(m, plain_view.payload()).unwrap();
}

#[test]
fn retained_snapshot_late_reads_and_live_prefix() {
    let old = Arc::new(declared::<i64, Bytes>(&[-9, 7, 12, 4], 1024, domain()));
    let retained = old.clone();
    let old_id = old.snapshot();
    drop(old);
    let replacement =
        ScanColumn::<i64, Bytes>::new(&[1, 2, 3, 4], BlockRows::new(1024).unwrap(), true);
    let view = retained.checked_view(domain(), 3).unwrap();
    let mut output = [0; 4];
    view.read_positions(old_id, &[2, 0, 2, 1], &mut output)
        .unwrap();
    assert_eq!(
        output,
        [
            12i64.ordinal(),
            (-9i64).ordinal(),
            12i64.ordinal(),
            7i64.ordinal()
        ]
    );
    assert_eq!(
        view.read_positions(replacement.snapshot(), &[0], &mut output[..1]),
        Err(FormatError::Snapshot)
    );
    assert_eq!(
        view.read_positions(old_id, &[3], &mut output[..1]),
        Err(FormatError::Position)
    );
    let mut mask = [0; 3];
    view.filter_mask(domain().id, &Predicate::Point(4i64.ordinal()), &mut mask)
        .unwrap();
    assert_eq!(mask, [0; 3]);
    let mut equality_only = domain();
    equality_only.order = false;
    assert!(retained.checked_view(equality_only, 3).is_err());
    let eq_source = declared::<i64, Bytes>(&[-9, 7, 12, 4], 1024, equality_only);
    let eq = eq_source.checked_view(equality_only, 3).unwrap();
    assert!(
        eq.filter_mask(
            domain().id,
            &Predicate::Window {
                lo: Bound::Unbounded,
                hi: Bound::Unbounded
            },
            &mut mask
        )
        .is_err()
    );
    assert!(
        eq.filter_mask(
            DomainId {
                namespace: 7,
                version: 4
            },
            &Predicate::Point(7i64.ordinal()),
            &mut mask
        )
        .is_err()
    );
    eq.filter_mask(domain().id, &Predicate::Point(7i64.ordinal()), &mut mask)
        .unwrap();
    assert_eq!(mask, [0, 1, 0]);
}

#[test]
fn declared_narrow_carriers_and_boundary_widths() {
    let mut narrow = domain();
    narrow.carrier = Width::W32;
    for (span, width) in [
        (0, Width::W0),
        (255, Width::W8),
        (256, Width::W16),
        (65535, Width::W16),
        (65536, Width::W32),
        (u32::MAX, Width::W32),
    ] {
        let column = declared::<u32, Bytes>(&[0, span], 1024, narrow);
        assert_eq!(column.headers()[0].width, width);
        let view = column.checked_view(narrow, 2).unwrap();
        CheckedScanView::import(view.metadata(), view.payload()).unwrap();
        let mut out = [0; 2];
        view.read_positions(column.snapshot(), &[1, 0], &mut out)
            .unwrap();
        assert_eq!(out, [span as u64, 0]);
        assert!(column.checked_view(domain(), 2).is_err());
    }
    let column = declared::<i32, Plain>(&[i32::MIN, 0, i32::MAX], 1024, narrow);
    let view = column.checked_view(narrow, 3).unwrap();
    CheckedScanView::import(view.metadata(), view.payload()).unwrap();
    let mut out = [0; 3];
    view.read_positions(column.snapshot(), &[2, 0, 1], &mut out)
        .unwrap();
    assert_eq!(out, [u32::MAX as u64, 0, 1 << 31]);
}

#[test]
fn membership_live_prefix_crosses_block_frames_and_can_be_empty() {
    let mut values = vec![-20; 1024];
    values.extend((0..1024).map(|row| 1000 + row % 256));
    values.extend([i64::MAX - 1, i64::MAX]);
    let column = declared::<i64, Bytes>(&values, 1024, domain());
    let predicate = Predicate::Membership(
        [-21i64, -20, 999, 1000, 1255, 1256, i64::MAX, i64::MAX]
            .map(|value| value.ordinal())
            .to_vec(),
    );
    let expected: Vec<u8> = values
        .iter()
        .map(|value| oracle(&predicate, value.ordinal()) as u8)
        .collect();
    for live in [0, 1, 1023, 1024, 1025, 2048, 2049, values.len()] {
        let view = column.checked_view(domain(), live).unwrap();
        let imported = CheckedScanView::import(view.metadata(), view.payload()).unwrap();
        let mut mask = vec![9; live];
        imported
            .filter_mask(domain().id, &predicate, &mut mask)
            .unwrap();
        assert_eq!(mask, expected[..live]);
    }
    let empty = column.checked_view(domain(), 0).unwrap();
    let mut wrong_domain = domain().id;
    wrong_domain.version += 1;
    assert_eq!(
        empty.filter_mask(wrong_domain, &predicate, &mut []),
        Err(FormatError::Domain)
    );
    assert_eq!(
        empty.filter_mask(domain().id, &predicate, &mut [0]),
        Err(FormatError::Count)
    );
}

#[test]
fn checked_bounded_masks_preserve_live_prefix_domain_and_intersection() {
    let values: Vec<i64> = (0..2061).map(|row| row % 19 - 7).collect();
    let column = declared::<i64, Bytes>(&values, 2048, domain());
    for live in [0, 1, 1024, 1027, values.len()] {
        let view = column.checked_view(domain(), live).unwrap();
        for test in [TestMode::Encoded, TestMode::DecodeToCode] {
            for prune in [PruneMode::None, PruneMode::MinMax] {
                let first = Predicate::Window {
                    lo: Bound::Included(0i64.ordinal()),
                    hi: Bound::Unbounded,
                };
                let second = Predicate::Membership(vec![0i64.ordinal(), 5i64.ordinal()]);
                let mut actual = Vec::new();
                let mut mask = [1; VECTOR_ROWS];
                for start in (0..live).step_by(VECTOR_ROWS) {
                    let len = VECTOR_ROWS.min(live - start);
                    mask[..len].fill(1);
                    view.intersect_mask(domain().id, start, &mut mask[..len], &first, prune, test).unwrap();
                    view.intersect_mask(domain().id, start, &mut mask[..len], &second, prune, test).unwrap();
                    actual.extend(mask[..len].iter().enumerate().filter_map(
                        |(local, &yes)| (yes != 0).then_some(start + local),
                    ));
                }
                let expected: Vec<_> = values[..live].iter().enumerate().filter_map(|(row, value)| {
                    (oracle(&first, value.ordinal()) && oracle(&second, value.ordinal())).then_some(row)
                }).collect();
                assert_eq!(actual, expected);
                assert_eq!(
                    view.intersect_mask(domain().id, live, &mut [], &first, prune, test),
                    Ok(Classification::Disqualified),
                );
            }
        }
        let wrong = DomainId { version: domain().id.version + 1, ..domain().id };
        assert_eq!(
            view.intersect_mask(wrong, 0, &mut [], &Predicate::Point(0), PruneMode::None, TestMode::Encoded),
            Err(FormatError::Domain),
        );
        assert_eq!(
            view.intersect_mask(domain().id, live, &mut [1], &Predicate::Point(0), PruneMode::None, TestMode::Encoded),
            Err(FormatError::Position),
        );
    }
}

#[test]
fn requested_summary_capability_is_not_an_implicit_fallback() {
    let mut builder = ScanColumnBuilder::<u64, Bytes>::with_domain(
        BlockRows::new(1024).unwrap(), false, false, domain(),
    ).unwrap();
    builder.extend([1, 2, 3]);
    let column = builder.finish();
    let view = column.checked_view(domain(), 3).unwrap();
    let mut mask = [1; 3];
    assert_eq!(
        view.intersect_mask(domain().id, 0, &mut mask, &Predicate::Point(2), PruneMode::MinMax, TestMode::Encoded),
        Err(FormatError::SummaryCount),
    );
    view.intersect_mask(domain().id, 0, &mut mask, &Predicate::Point(2), PruneMode::None, TestMode::Encoded).unwrap();
    assert_eq!(mask, [0, 1, 0]);
}
