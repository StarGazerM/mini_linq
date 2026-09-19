//! The paper's YA example stops at verified RA and IX boundaries.
use cq_ir::{cq, relational_plan};
use cq_query_lowering::Rule as JoinRule;
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
    let (ra, report) = cq_query_lowering::composition::explain_cq_plan(
        &source,
        cq_query_lowering::PlanningOptions {
            strategy: cq_query_lowering::JoinPolicy::Auto,
            max_arity: None,
        },
    )
    .unwrap();
    std::fs::write(directory.join("report.txt"), format!("{report:#?}")).unwrap();
    cq_query_lowering::composition::check_cq_plan(&source, &ra, &report).unwrap();
    assert_eq!(
        report.join_tree.as_ref().unwrap().parent,
        [Some(1), Some(2), None]
    );
    assert_eq!(report.peak_arity, 4);
    assert_eq!(
        report.derivation,
        [
            JoinRule::Introduce { occurrence: 0 },
            JoinRule::Introduce { occurrence: 1 },
            JoinRule::Introduce { occurrence: 2 },
            JoinRule::Select {
                group: 2,
                predicate: 0
            },
            JoinRule::Begin,
            JoinRule::Up { child: 0 },
            JoinRule::Up { child: 1 },
            JoinRule::Down { child: 1 },
            JoinRule::Down { child: 0 },
            JoinRule::Freeze,
            JoinRule::Trim { group: 0 },
            JoinRule::Combine { left: 2, right: 1 },
            JoinRule::Trim { group: 2 },
            JoinRule::Combine { left: 2, right: 0 },
            JoinRule::Trim { group: 2 },
            JoinRule::Output { group: 2 },
        ]
    );
    let expected_ra: relational_plan::Plan = syn::parse_quote! {
        r0 = rename R { a -> x, b -> k, c -> tag, d -> unused };
        r1 = rename S { a -> k, b -> tag, c -> z };
        r2 = rename T { a -> z, b -> y };
        r3 = select r2 if (y > 0i32);
        r4 = semijoin r1 with r0;
        r5 = semijoin r3 with r4;
        r6 = semijoin r4 with r5;
        r7 = semijoin r0 with r6;
        r8 = project r7 keep { k, tag, x };
        r9 = natural_join r5 with r6;
        r10 = project r9 keep { k, tag, y };
        r11 = natural_join r10 with r8;
        r12 = project r11 keep { x, y };
        output r12 as answer(x, y);
    };
    assert_eq!(ra.plan, expected_ra);
    let planned = boundaries(&directory, "planned", &source, true);
    let written = boundaries(&directory, "written", &source, false);
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
        descriptors(&planned),
        vec![
            ("r0".into(), vec![0, 1]),
            ("r4".into(), vec![2]),
            ("r5".into(), vec![1]),
            ("r6".into(), vec![0, 1]),
            ("r6".into(), vec![2]),
            ("r8".into(), vec![0, 1])
        ]
    );
    assert_eq!(
        descriptors(&written),
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
    let evidence = "PASS: exact YA derivation, RA and IX descriptors; independent oracle checks five input presentations. No post-IX compiler or generated-query execution is exercised.\n";
    std::fs::write(directory.join("validation.txt"), evidence).unwrap();
    print!("{evidence}");
}
