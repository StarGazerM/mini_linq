use cq_storage::{OrderKey, scan::*};
use std::ops::Bound::{Excluded, Included, Unbounded};

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
