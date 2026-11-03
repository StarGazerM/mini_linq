use cq_storage::{OrderKey, scan::*};
use std::ops::Bound::{Excluded, Included, Unbounded};

#[test]
fn incremental_encoding_is_batch_independent() {
    let values: Vec<i64> = (0..5117).map(|i| (i % 1300) - 700).collect();
    for summaries in [false, true] {
        for size in [1024, 2048] {
            let block_rows = BlockRows::new(size).unwrap();
            let expected = ScanColumn::<i64, Bytes>::new(&values, block_rows, summaries);
            for batch_size in [1, 333, 1024, 3000, 6000] {
                let mut builder = ScanColumnBuilder::<i64, Bytes>::new(block_rows, summaries);
                builder.extend(std::iter::empty());
                for batch in values.chunks(batch_size) {
                    builder.extend(batch.iter().copied());
                    builder.extend(std::iter::empty());
                }
                let actual = builder.finish();
                assert_eq!(actual.len(), values.len());
                assert_eq!(actual.headers(), expected.headers());
                assert_eq!(actual.payload(), expected.payload());
                assert_eq!(actual.summaries(), expected.summaries());
            }
        }
    }
    let empty = ScanColumnBuilder::<i64, Plain>::new(BlockRows::new(1024).unwrap(), true).finish();
    assert!(empty.is_empty());
    assert!(empty.headers().is_empty());
    assert_eq!(empty.summaries(), Some([].as_slice()));
}

fn laws<T: OrdinalDecode + std::fmt::Debug>(values: &[T]) {
    check::<T, Plain>(values);
    check::<T, Bytes>(values);
}
fn check<T: OrdinalDecode + std::fmt::Debug, F: Family + EncodedPoint + EncodedWindow>(
    values: &[T],
) {
    for summary in [false, true] {
        for size in [1024, 2048] {
            let col = ScanColumn::<T, F>::new(values, BlockRows::new(size).unwrap(), summary);
            assert_eq!(col.len(), values.len());
            assert_eq!(col.is_empty(), values.is_empty());
            assert_eq!(col.payload().as_ptr() as usize % 8, 0);
            let mut decoded = Vec::new();
            if let Some(first) = values.first() {
                let mut out = [*first; VECTOR_ROWS];
                for (b, h) in col.headers().iter().enumerate() {
                    for v in 0..(h.rows as usize).div_ceil(VECTOR_ROWS) {
                        let n = col.decode_vector(b, v, &mut out);
                        decoded.extend_from_slice(&out[..n]);
                    }
                    if let Some(s) = col.summaries() {
                        let block =
                            &values[b * size as usize..(b * size as usize + h.rows as usize)];
                        assert_eq!(s[b].lo, block.iter().map(OrderKey::ordinal).min().unwrap());
                        assert_eq!(s[b].hi, block.iter().map(OrderKey::ordinal).max().unwrap());
                    }
                }
            }
            assert_eq!(decoded, values);
            let constants = [
                0,
                1,
                127,
                255,
                256,
                65535,
                65536,
                u32::MAX as u64,
                u64::MAX - 1,
                u64::MAX,
            ];
            for c in constants
                .into_iter()
                .chain(values.iter().take(12).map(OrderKey::ordinal))
            {
                let predicates = [
                    Predicate::Point(c),
                    Predicate::Window {
                        lo: Included(c),
                        hi: Unbounded,
                    },
                    Predicate::Window {
                        lo: Excluded(c),
                        hi: Unbounded,
                    },
                    Predicate::Window {
                        lo: Unbounded,
                        hi: Included(c),
                    },
                    Predicate::Window {
                        lo: Unbounded,
                        hi: Excluded(c),
                    },
                    Predicate::Window {
                        lo: Excluded(c),
                        hi: Included(c),
                    },
                    Predicate::Membership(vec![0, c, u64::MAX, c]),
                ];
                for p in predicates {
                    let expected: Vec<_> = values
                        .iter()
                        .enumerate()
                        .filter_map(|(i, x)| {
                            let x = x.ordinal();
                            let yes = match &p {
                                Predicate::Point(c) => x == *c,
                                Predicate::Membership(keys) => keys.contains(&x),
                                Predicate::Window { lo, hi } => {
                                    (match lo {
                                        Included(c) => x >= *c,
                                        Excluded(c) => x > *c,
                                        Unbounded => true,
                                    }) && (match hi {
                                        Included(c) => x <= *c,
                                        Excluded(c) => x < *c,
                                        Unbounded => true,
                                    })
                                }
                            };
                            yes.then_some(i)
                        })
                        .collect();
                    let mut actual = Vec::new();
                    col.filter(&p, |i| actual.push(i));
                    assert_eq!(actual, expected, "{p:?}");
                    let mut mask = vec![255; values.len()];
                    col.filter_mask(&p, &mut mask);
                    assert!(mask.iter().all(|v| *v <= 1));
                    let selected: Vec<_> = mask
                        .iter()
                        .enumerate()
                        .filter_map(|(i, &v)| (v != 0).then_some(i))
                        .collect();
                    assert_eq!(selected, expected, "mask {p:?}");
                    for test in [TestMode::Encoded, TestMode::DecodeToCode] {
                        for prune in [PruneMode::None, PruneMode::MinMax] {
                            if prune == PruneMode::MinMax && !summary {
                                continue;
                            }
                            let mut selected = Vec::new();
                            let mut tile = [1; VECTOR_ROWS];
                            for start in (0..values.len()).step_by(VECTOR_ROWS) {
                                let len = VECTOR_ROWS.min(values.len() - start);
                                tile[..len].fill(1);
                                col.intersect_mask(start, &mut tile[..len], &p, prune, test);
                                selected.extend(tile[..len].iter().enumerate().filter_map(
                                    |(local, &live)| (live != 0).then_some(start + local),
                                ));
                            }
                            assert_eq!(selected, expected, "bounded {p:?} {prune:?} {test:?}");
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn decode_enclosure_and_preserved_operations() {
    laws::<i64>(&[]);
    laws(&[i64::MIN, -1, 0, 1, i64::MAX]);
    laws(&[i32::MIN, -1, 0, 1, i32::MAX]);
    laws(&[0u32, 1, u32::MAX]);
    laws(&[0u64, 1, u64::MAX]);
    for span in [0, 1, 255, 256, 65535, 65536, u64::MAX] {
        let mut state = 17u64;
        let values: Vec<_> = (0..2051)
            .map(|_| {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                if span == u64::MAX {
                    state
                } else {
                    state % (span + 1)
                }
            })
            .collect();
        laws(&values);
    }
    laws(&vec![i64::MIN; 2051]);
    laws(&vec![i64::MAX; 2051]);
}
#[test]
fn family_fixed_width_rule_and_layout() {
    assert!(BlockRows::new(0).is_none());
    assert!(BlockRows::new(1023).is_none());
    for (span, width, codec) in [
        (0, Width::W0, CodecId::For),
        (255, Width::W8, CodecId::For),
        (256, Width::W16, CodecId::For),
        (65535, Width::W16, CodecId::For),
        (65536, Width::W64, CodecId::Plain),
    ] {
        let col = ScanColumn::<u64, Bytes>::new(&[0, span], BlockRows::new(1024).unwrap(), true);
        let h = col.headers()[0];
        assert_eq!((h.width, h.codec), (width, codec));
        assert_eq!(col.payload().len(), 2 * width as usize);
    }
    assert_eq!(std::mem::size_of::<BlockHeader>(), 24);
    assert_eq!(CodecId::Plain as u8, 0);
    assert_eq!(CodecId::For as u8, 1);
}
#[test]
fn runtime_scalar_inverse_laws() {
    use cq_load::{Date, Decimal, StringCode};
    laws(&[
        Date::from_days(i32::MIN),
        Date::from_days(0),
        Date::from_days(i32::MAX),
    ]);
    laws(&[
        Decimal::<2>::from_scaled(i64::MIN).unwrap(),
        Decimal::from_scaled(0).unwrap(),
        Decimal::from_scaled(i64::MAX).unwrap(),
    ]);
    laws(&[StringCode(0), StringCode(255), StringCode::MAX]);
}

#[test]
fn membership_translation_borrows_keys_and_checks_block_frame() {
    let predicate = Predicate::Membership(vec![0, 9, 10, 12, 265, 266, u64::MAX - 1, u64::MAX]);
    let Predicate::Membership(original) = &predicate else {
        unreachable!()
    };
    for base in [0, 10, u64::MAX - 2, u64::MAX] {
        for (width, max) in [
            (Width::W0, 0),
            (Width::W8, u8::MAX as u64),
            (Width::W16, u16::MAX as u64),
            (Width::W32, u32::MAX as u64),
            (Width::W64, u64::MAX),
        ] {
            let header = BlockHeader {
                offset: 0,
                base,
                rows: 1,
                width,
                codec: CodecId::For,
                reserved: [0; 2],
            };
            let translated = predicate.translate(&header);
            let TranslatedPredicate::Membership {
                keys,
                base: recorded_base,
                max: recorded_max,
            } = &translated
            else {
                panic!("membership changed predicate kind");
            };
            assert!(std::ptr::eq(*keys, original.as_slice()));
            assert_eq!((*recorded_base, *recorded_max), (base, max));
            for code in [
                0,
                1,
                2,
                9,
                10,
                255,
                256,
                65535,
                65536,
                u32::MAX as u64,
                u64::MAX,
            ] {
                let expected = code <= max
                    && base
                        .checked_add(code)
                        .is_some_and(|k| original.contains(&k));
                assert_eq!(
                    translated.matches(code),
                    expected,
                    "{header:?}, code {code}"
                );
            }
        }
    }
}

fn bounded_laws<F: Family + EncodedPoint + EncodedWindow>() {
    for count in [0, 1, 1024, 1027, 4099] {
        let values: Vec<i64> = (0..count).map(|i| i as i64 / 1024 * 1000 + i as i64 % 17).collect();
        for summaries in [false, true] {
            let table = ScanTable::new((
                ScanColumn::<i64, F>::new(&values, BlockRows::new(2048).unwrap(), summaries),
                ScanColumn::<i64, F>::new(&values, BlockRows::new(2048).unwrap(), summaries),
            ));
            let membership = Predicate::Membership(vec![0i64.ordinal(), 7i64.ordinal(), 1007i64.ordinal(), 4002i64.ordinal()]);
            let window = Predicate::Window { lo: Included(7i64.ordinal()), hi: Unbounded };
            for test in [TestMode::Encoded, TestMode::DecodeToCode] {
                for prune in [PruneMode::None, PruneMode::MinMax] {
                    if prune == PruneMode::MinMax && !summaries {
                        continue;
                    }
                    let expected: Vec<_> = values.iter().enumerate().filter_map(|(i, value)| {
                        (membership.matches(value.ordinal()) && window.matches(value.ordinal())).then_some(i)
                    }).collect();
                    let actual: Vec<_> = table.scan_positions(0..count, |tile| {
                        tile.intersect::<0>(&membership, prune, test);
                        tile.intersect::<1>(&window, prune, test);
                    }).map(ScanPosition::index).collect();
                    assert_eq!(actual, expected);

                    let mut mask = ScanMask::default();
                    let mut empty = table.tile(count, &mut mask);
                    assert!(empty.is_empty());
                    empty.intersect::<0>(&membership, prune, test);
                    assert_eq!(empty.positions().count(), 0);
                }
            }
        }
    }
}

#[test]
fn bounded_plain_and_bytes_intersect_without_losing_occurrences() {
    assert_eq!(std::mem::size_of::<ScanMask>(), VECTOR_ROWS);
    bounded_laws::<Plain>();
    bounded_laws::<Bytes>();
}

#[test]
fn lazy_ranges_and_carrier_filters_preserve_order_and_multiplicity() {
    use std::cell::Cell;
    let values: Vec<u64> = (0..2051).map(|i| i % 3).collect();
    let table = ScanTable::new((
        ScanColumn::<u64, Bytes>::new(&values, BlockRows::new(1024).unwrap(), false),
    ));
    let calls = Cell::new(0);
    let mut scan = table.scan_positions(1024..2051, |tile| {
        calls.set(calls.get() + 1);
        tile.intersect::<0>(&Predicate::Point(1), PruneMode::None, TestMode::Encoded);
    });
    assert_eq!(calls.get(), 0);
    let first = scan.next().unwrap();
    assert_eq!(first.index(), 1024);
    assert_eq!(calls.get(), 1);
    drop(scan);
    assert_eq!(calls.get(), 1, "early stop must not prepare the next vector");

    let last = table.scan_positions(2048..2051, |_| {}).next().unwrap();
    let input = [(last, "late"), (first, "first"), (first, "duplicate"), (last, "again")];
    let filtered: Vec<_> = table.filter_rows(input, |item| item.0, |tile| {
        tile.retain(|position| table.read_ordinal::<0>(position) == 1);
    }).collect();
    assert_eq!(filtered, vec![(first, "first"), (first, "duplicate")]);
    assert_eq!(table.scan_positions(0..1024, |_| {}).count(), 1024);
    assert_eq!(table.scan_positions(1024..2048, |_| {}).count(), 1024);
    assert_eq!(table.scan_positions(2048..2051, |_| {}).count(), 3);
}

#[test]
#[should_panic(expected = "scan position belongs to another table")]
fn table_positions_cannot_cross_snapshots() {
    let a = ScanTable::new((ScanColumn::<u64, Plain>::new(&[3], BlockRows::new(1024).unwrap(), false),));
    let b = ScanTable::new((ScanColumn::<u64, Plain>::new(&[3], BlockRows::new(1024).unwrap(), false),));
    let position = a.scan_positions(0..1, |_| {}).next().unwrap();
    b.read_ordinal::<0>(position);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct NeverMaterialize(u64);
impl OrderKey for NeverMaterialize {
    type Ordinal = u64;
    fn ordinal(&self) -> u64 { self.0 }
}
impl OrdinalDecode for NeverMaterialize {
    const WIDTH: Width = Width::W64;
    fn from_ordinal(_: u64) -> Self {
        panic!("an undemanded field was materialized")
    }
}

#[test]
fn only_authorized_demanded_fields_are_materialized() {
    let table = ScanTable::new((
        ScanColumn::<NeverMaterialize, Bytes>::new(
            &[NeverMaterialize(1), NeverMaterialize(2)], BlockRows::new(1024).unwrap(), false,
        ),
        ScanColumn::<i64, Plain>::new(&[-8, 12], BlockRows::new(1024).unwrap(), false),
    ));
    let position = table.scan_positions(0..2, |tile| {
        tile.intersect::<0>(&Predicate::Point(2), PruneMode::None, TestMode::DecodeToCode);
    }).next().unwrap();
    assert_eq!(table.read_ordinal::<0>(position), 2);
    assert_eq!(
        table.read_value::<1>(position, LogicalRead::OpaqueRust { consumer: "arithmetic" }),
        12,
    );
    assert_eq!(
        table.read_value::<1>(position, LogicalRead::FinalAnswer { consumer: "answer" }),
        12,
    );
}

#[test]
fn explicit_slot_domains_bind_without_reencoding() {
    use cq_storage::scan::format::{Domain, DomainId, FormatError};
    let column = ScanColumn::<i64, Bytes>::new(&[-1, 7], BlockRows::new(1024).unwrap(), false);
    let payload = column.payload().as_ptr();
    let snapshot = column.snapshot();
    let domain = Domain {
        id: DomainId { namespace: 11, version: 3 },
        carrier: Width::W64,
        equality: true,
        order: true,
    };
    let table = ScanTable::new((column,)).bind_domains(&[domain]).unwrap();
    let _: &ScanColumn<i64, Bytes> = table.column::<0>();
    assert_eq!(table.column::<0>().declared_domain(), Some(domain));
    assert_eq!(table.column::<0>().payload().as_ptr(), payload);
    assert_eq!(table.column::<0>().snapshot(), snapshot);
    assert!(table.column::<0>().checked_view(domain, 2).is_ok());
    let bad = Domain { carrier: Width::W32, ..domain };
    assert!(matches!(table.bind_domains(&[bad]), Err(FormatError::Domain)));
}
