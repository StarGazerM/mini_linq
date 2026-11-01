//! EXT-PackedCanon: authored advice retypes canonical rows records to the
//! packed-OrderKey sorted run; answers and their order equal the comparator's.
use cq_lang_structures::{self as ir, Structure};
use cq_rewrite_passes::packed_canon;
use quote::quote;

macro_rules! distinct {
    ($rows:expr; $($pipeline:tt)*) => {
        mini_linq::compile! {
            pipeline { $($pipeline)* }
            struct Distinct;
            relation R(a:i64, b:u32, c:i64) = ($rows);
            answer(c, a) :- R(a, b, c), where b != 3;
        }
    };
}

fn rows_records(program: &ir::Program) -> Vec<String> {
    program
        .structures
        .entries
        .iter()
        .filter_map(|entry| match &entry.structure {
            Structure::Rows { provider, .. } => Some(provider.token.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn packed_canon_retypes_canonical_rows_and_preserves_answers() {
    let captured = distinct!(rows;
        structures = [cq_rewrites::packed_canon];
        print_input = structures;
        print = cq_rewrite_test_driver::capture_structures;
    );
    let program: ir::Program = syn::parse_str(captured).unwrap();
    let facts = ir::contract::check(&program).unwrap();
    assert!(facts.canonical.values().any(|source| source.rows_provider().is_some_and(
        |provider| provider.token == "packed"
            && provider.ty == syn::parse_quote!(::cq_storage::dedup::Packed)
    )));
    assert!(!rows_records(&program).contains(&"sort".to_owned()));
    assert_eq!(packed_canon(&program, quote!()).unwrap(), program);
    assert!(packed_canon(&program, quote!(all)).is_err());

    let rows: Vec<(i64, u32, i64)> = (0..400_i64)
        .map(|i| ((i * 7919) % 13 - 6, (i % 5) as u32, i64::MIN + (i % 9)))
        .chain([
            (i64::MAX, 0, i64::MAX),
            (i64::MIN, 1, i64::MIN),
            (i64::MAX, 0, i64::MAX),
        ])
        .collect();
    let packed: Vec<(i64, i64)> = distinct!(&rows; structures = [cq_rewrites::packed_canon];)
        .map(|(c, a)| (*c, *a))
        .collect();
    let sorted: Vec<(i64, i64)> = distinct!(&rows;).map(|(c, a)| (*c, *a)).collect();
    assert_eq!(packed, sorted);
    let mut expected: Vec<(i64, i64)> = rows
        .iter()
        .filter(|(_, b, _)| *b != 3)
        .map(|&(a, _, c)| (c, a))
        .collect();
    expected.sort();
    expected.dedup();
    let mut unordered = packed.clone();
    unordered.sort();
    assert_eq!(unordered, expected);
}
