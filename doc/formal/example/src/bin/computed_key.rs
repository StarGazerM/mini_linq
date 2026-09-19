//! Computed-argument planning and index demands; no retired physical backend.
use cq_ir::cq;
use mini_linq_paper_overview::{boundaries, directory};
use std::collections::BTreeSet;

fn main() {
    let directory = directory("logical-computed-key");
    let source: cq::Module = syn::parse_quote! {
        pub struct Overview;
        relation R(a:i32,b:i32);
        relation S(a:i32,b:i32);
        relation answer(a:i32,b:i32);
        answer(x,*y+1) :- R(x,k), let next:i32 = *k+1, S(next,y), if y > 0;
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
    boundaries(&directory, "written", &source, false);
    boundaries(&directory, "planned", &source, true);
    let r = [(1, 6), (2, 6), (3, 8)];
    let s = [(7, 10), (7, -2), (9, 4)];
    let mut answer = BTreeSet::new();
    for (x, k) in r {
        for (next, y) in s {
            if k + 1 == next && y > 0 {
                answer.insert((x, y + 1));
            }
        }
    }
    assert_eq!(
        answer.into_iter().collect::<Vec<_>>(),
        [(1, 11), (2, 11), (3, 5)]
    );
    let evidence = "PASS: computed argument CQ→RA→IX and derivation checks; independent oracle answer [(1,11),(2,11),(3,5)]. Generated query execution remains unimplemented for this domain.\n";
    std::fs::write(directory.join("validation.txt"), evidence).unwrap();
    print!("{evidence}");
}
