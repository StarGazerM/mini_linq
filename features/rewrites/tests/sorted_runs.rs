use cq_lang_structures::{self as ir, Structure};
use std::collections::BTreeSet;

macro_rules! triangle {
    ($r:expr, $s:expr, $t:expr; $($pipeline:tt)*) => {
        mini_linq::compile! {
            pipeline { $($pipeline)* }
            struct Triangle;
            relation R(a:i32,b:i32) = ($r);
            relation S(b:i32,c:i32) = ($s);
            relation T(a:i32,c:i32) = ($t);
            answer(a,b,c) :- R(a,b), S(b,c), where c >= 0, T(a,c);
        }
    };
}

#[test]
fn public_pipeline_records_sorted_runs_and_executes_default_answers() {
    let baseline: ir::Program = syn::parse_str(triangle!(r, s, t;
        print_input = structures;
        print = cq_rewrite_test_driver::capture_structures;
    ))
    .unwrap();
    let rewritten: ir::Program = syn::parse_str(triangle!(r, s, t;
        structures = [cq_rewrites::sorted_runs];
        print_input = structures;
        print = cq_rewrite_test_driver::capture_structures;
    ))
    .unwrap();
    ir::check_rewrite(&baseline, &rewritten).unwrap();
    // A relational predicate exposes the filtered build to storage selection;
    // an opaque Rust `if` carries no relational planning information.
    assert!(
        rewritten.structures.entries.iter().any(|entry| {
            matches!(&entry.structure, Structure::Built { keyed, .. }
            if matches!(&keyed.ty, syn::Type::Path(p)
                if p.path.segments.last().unwrap().ident == "SortedRun"))
        }),
        "{:#?}",
        rewritten.structures
    );
    for (r, s, t) in [
        (vec![], vec![], vec![]),
        (
            vec![(1, 10), (1, 10), (2, 20), (3, 30)],
            vec![(10, 100), (10, 101), (20, 200), (30, 300), (10, 100)],
            vec![(1, 100), (1, 100), (2, 999)],
        ),
    ] {
        let expected: BTreeSet<_> = r
            .iter()
            .flat_map(|&(a, b)| {
                let t = &t;
                s.iter().filter_map(move |&(sb, c)| {
                    (b == sb && c >= 0 && t.contains(&(a, c))).then_some((a, b, c))
                })
            })
            .collect();
        let baseline: BTreeSet<_> = triangle!(r, s, t;).map(|(a, b, c)| (*a, *b, *c)).collect();
        let rewritten: BTreeSet<_> = triangle!(r, s, t;
            structures = [cq_rewrites::sorted_runs];
        )
        .map(|(a, b, c)| (*a, *b, *c))
        .collect();
        assert_eq!(baseline, expected);
        assert_eq!(rewritten, expected);
    }
}

#[test]
fn one_run_answers_both_payload_and_membership_reads() {
    let left = [(1,), (2,), (3,), (4,)];
    let right = [(3, 30), (1, 10), (1, 11), (1, 10), (4, 40)];
    let actual: BTreeSet<_> = mini_linq::passes::select! {
        pipeline { cq_rewrites::sorted_runs, mini_linq::passes::sequence, mini_linq::passes::sequential, mini_linq::passes::translate_pull };
        physical {
            relation L(a:i32) = (left); relation R(a:i32,b:i32) = (right);
            l = pipe rename L@0 {a -> x};
            r = let rename R@1 {a -> x,b -> y};
            s = pipe semi eager[x] l,r;
            p = pipe probe first[x] s,r;
            out = pipe canon project p keep {x,y};
            return out as answer(x,y);
        }
    }
    .map(|(x, y)| (*x, *y))
    .collect();
    assert_eq!(actual, BTreeSet::from([(1, 10), (1, 11), (3, 30), (4, 40)]));
}

#[test]
fn closed_owned_probe_key_uses_exact_sorted_run() {
    let right = [(3i64, 30i64), (1, 10), (3, 31)];
    let actual: BTreeSet<_> = mini_linq::passes::select! {
        pipeline { cq_rewrites::sorted_runs, mini_linq::passes::sequence, mini_linq::passes::sequential, mini_linq::passes::translate_pull };
        physical {
            relation R(a:i64,b:i64) = (&right);
            u = pipe unit;
            l = pipe bind u with x:i64 = (3i64);
            r = let rename R@0 {a -> x,b -> y};
            p = pipe canon probe first[x] l,r;
            return p as answer(*y);
        }
    }.collect();
    assert_eq!(actual, BTreeSet::from([(30,), (31,)]));
}
