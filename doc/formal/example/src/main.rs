//! The paper's YA example stops at verified VO, RA and IX boundaries.
use cq_ir::{cq, relational_plan, valuation};
use mini_linq_paper_overview::{boundaries, directory};
use std::collections::BTreeSet;

fn oracle(r: &[(i32, i32, i32, i32)], s: &[(i32, i32, i32)], t: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut answer = BTreeSet::new();
    // Enumerate whole base tuples, independently of the compiler's RA and indexes.
    for &(x, k, tag, _) in r {
        for &(sk, stag, z) in s {
            for &(tz, y) in t {
                if k == sk && tag == stag && z == tz && y > 0 {
                    answer.insert((x, y));
                }
            }
        }
    }
    answer.into_iter().collect()
}

fn main() {
    let directory = directory("logical-overview");
    let source: cq::Module = syn::parse_quote! {
        pub struct Overview;
        relation R(a:i32,b:i32,c:i32,d:i32);
        relation S(a:i32,b:i32,c:i32);
        relation T(a:i32,b:i32);
        relation answer(a:i32,b:i32);
        answer(x,y) :- R(x,k,tag,unused), S(k,tag,z), T(z,y), if y > 0;
    };
    let planned = boundaries(&directory, "planned", &source, true);
    let written = boundaries(&directory, "written", &source, false);
    // The GYO join tree: R under S under the root T.
    assert_eq!(
        cq::hypergraph::Analysis::of(&planned.normal).join_tree,
        Some(vec![(0, Some(1)), (1, Some(2)), (2, None)])
    );
    // The guard on T is pushed onto T; the reducer goes up from R through S
    // to T and down again, before the first atom step.
    let expected_vo: valuation::Module = syn::parse_quote! {
        pub struct Overview;
        relation R(a:i32,b:i32,c:i32,d:i32);
        relation S(a:i32,b:i32,c:i32);
        relation T(a:i32,b:i32);
        relation answer(a:i32,b:i32);
        answer(x,y) :- R(x,k,tag,unused), S(k,tag,z), T(z,y), if y > 0;
        valuation {
            reduce T@2 if y > 0;
            reduce S@1 with R@0(_, k, tag, _);
            reduce T@2 with S@1(_, _, z);
            reduce S@1 with T@2(z, _);
            reduce R@0 with S@1(k, tag, _);
            R@0(x, k, tag, unused);
            S@1(k, tag, z);
            T@2(z, y);
        }
    };
    assert_eq!(planned.valuation, expected_vo);
    let expected_ra: relational_plan::Plan = syn::parse_quote! {
        r0 = rename T { a -> z, b -> y };
        r1 = select r0 if (y > 0);
        r2 = rename S { a -> k, b -> tag, c -> z };
        r3 = rename R { a -> x, b -> k, c -> tag, d -> unused };
        r4 = semijoin r2 with r3;
        r5 = semijoin r1 with r4;
        r6 = semijoin r4 with r5;
        r7 = semijoin r3 with r6;
        r8 = natural_join r7 with r6;
        r9 = natural_join r8 with r5;
        r10 = project r9 keep { r#x, r#y };
        output r10 as answer(x, y);
    };
    assert_eq!(planned.relational.plan, expected_ra);
    let expected_written: relational_plan::Plan = syn::parse_quote! {
        r0 = rename R { a -> x, b -> k, c -> tag, d -> unused };
        r1 = rename S { a -> k, b -> tag, c -> z };
        r2 = natural_join r0 with r1;
        r3 = rename T { a -> z, b -> y };
        r4 = natural_join r2 with r3;
        r5 = select r4 if (y > 0);
        r6 = project r5 keep { r#x, r#y };
        output r6 as answer(x, y);
    };
    assert_eq!(written.relational.plan, expected_written);
    let descriptors = |ix: &cq_ir::index_requirements::Module| {
        ix.indexes
            .iter()
            .map(|index| {
                (
                    index.requirement.relation.to_string(),
                    index
                        .requirement
                        .columns
                        .iter()
                        .map(|column| column.index)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        descriptors(&planned.indexes),
        vec![
            ("r3".into(), vec![0, 1]),
            ("r4".into(), vec![2]),
            ("r5".into(), vec![1]),
            ("r6".into(), vec![0, 1])
        ]
    );
    assert_eq!(
        descriptors(&written.indexes),
        vec![("r1".into(), vec![0, 1]), ("r3".into(), vec![1])]
    );
    let r = [
        (1, 10, 1, 100),
        (1, 10, 1, 101),
        (2, 10, 2, 200),
        (3, 20, 1, 300),
        (4, 30, 1, 400),
        (5, 40, 1, 500),
    ];
    let s = [
        (10, 1, 1000),
        (10, 2, 2000),
        (20, 1, 3000),
        (40, 1, 4000),
        (50, 1, 5000),
    ];
    let t = [
        (1000, 7),
        (1000, 8),
        (2000, 9),
        (3000, -1),
        (5000, 11),
        (6000, 12),
    ];
    let expected = vec![(1, 7), (1, 8), (2, 9)];
    assert_eq!(oracle(&r, &s, &t), expected);
    assert_eq!(
        oracle(
            &r.into_iter().rev().chain(r).collect::<Vec<_>>(),
            &s.into_iter().rev().chain(s).collect::<Vec<_>>(),
            &t.into_iter().rev().chain(t).collect::<Vec<_>>()
        ),
        expected
    );
    assert!(oracle(&[], &s, &t).is_empty());
    assert!(oracle(&r, &[], &t).is_empty());
    assert!(oracle(&r, &s, &[]).is_empty());
    let evidence = "PASS: exact YA valuation plan, RA and IX descriptors; independent oracle checks five input presentations. No post-IX compiler or generated-query execution is exercised.\n";
    std::fs::write(directory.join("validation.txt"), evidence).unwrap();
    print!("{evidence}");
}
