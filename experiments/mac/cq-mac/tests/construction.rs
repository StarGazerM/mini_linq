#![cfg(target_os = "macos")]

use cq_mac::{IndexSpec, MacContext, Payload};

#[test]
fn skipped_radix_digits_and_all_constructors_preserve_complete_rows() {
    let context = MacContext::new().unwrap();
    // Exercise zero, odd and even numbers of varying bytes, including holes
    // between varying bytes. Payloads never serve as tie breakers.
    for mask in [
        0u32,
        0xff,
        0xff00,
        0xff0000,
        0xff000000,
        0xffff,
        0xff00ff,
        0xffffff,
        u32::MAX,
    ] {
        let mut state = 0x831a53d7u32;
        let rows = (0..4097u32)
            .map(|i| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (
                    (state & mask) | (0xa537c912 & !mask),
                    i.wrapping_mul(104729),
                )
            })
            .collect::<Vec<_>>();
        check(&context, &rows);
    }
    check(&context, &[]);
    check(&context, &[(u32::MAX, 42)]);
    check(&context, &[(u32::MAX, 9), (0, 3), (u32::MAX, 1), (0, 3)]);
}

fn check(context: &MacContext, rows: &[(u32, u32)]) {
    let mut expected = rows.to_vec();
    expected.sort_by_key(|row| row.0);
    let source = context
        .relation_u32(rows.len(), |i| [rows[i].0, rows[i].1])
        .unwrap();
    let spec = IndexSpec {
        key_column: 0,
        payload: Payload::Column(1),
    };
    let radix = context.build_on_cpu_radix(&source, spec, None).unwrap();
    let comparison = context.build_on_cpu(&source, spec, None).unwrap();
    let gpu = context.build(&source, spec, None).unwrap();
    let runs = [radix, comparison, gpu];
    for run in &runs {
        assert_eq!(run.rows().collect::<Vec<_>>(), expected);
        assert_eq!(
            run.rows().rev().collect::<Vec<_>>(),
            expected.iter().rev().copied().collect::<Vec<_>>()
        );
        for key in [0, 0x00ffffff, 0xff000000, u32::MAX] {
            let want = rows
                .iter()
                .filter(|row| row.0 == key)
                .map(|row| row.1)
                .collect::<Vec<_>>();
            assert_eq!(run.lookup(&key).collect::<Vec<_>>(), want);
        }
    }
    // Both constructors publish ordinary immutable runs usable by the same GPU merge.
    let merged = context.merge(&runs[0], &runs[2]).unwrap();
    let mut combined = rows.iter().chain(rows).copied().collect::<Vec<_>>();
    combined.sort_by_key(|row| row.0);
    assert_eq!(merged.rows().collect::<Vec<_>>(), combined);
}

#[test]
fn reused_wide_source_preserves_rows_and_selected_original_row_ids() {
    let context = MacContext::new().unwrap();
    let input = [
        [8, -7, 3],
        [i64::MAX, i64::MIN, 9],
        [8, i64::MAX, 1],
        [0, -1, 3],
    ];
    let source = context.relation_i64(input.len(), |i| input[i]).unwrap();
    let selection = context
        .relation_u32(input.len(), |i| [u32::from(i != 1)])
        .unwrap();
    let row_ids = IndexSpec {
        key_column: 0,
        payload: Payload::RowId,
    };
    let by_other = IndexSpec {
        key_column: 2,
        payload: Payload::RowId,
    };
    for _ in 0..3 {
        let run = context.build(&source, row_ids, Some(&selection)).unwrap();
        assert_eq!(run.rows().collect::<Vec<_>>(), [(0, 3), (8, 0), (8, 2)]);
        let other = context.build(&source, by_other, None).unwrap();
        assert_eq!(
            other.rows().collect::<Vec<_>>(),
            [(1, 2), (3, 0), (3, 3), (9, 1)]
        );
        assert_eq!(source.i64_values().unwrap(), input.as_flattened());
        cq_mac::memory::clear_cache().unwrap();
    }
    assert!(context.build(&source, row_ids, None).is_err());
    let clone = source.clone();
    drop(source);
    drop(context);
    assert_eq!(clone.i64_values().unwrap(), input.as_flattened());
}
