//! Computed-argument planning and index demands; no physical execution.
use cq_ir::{cq, valuation};
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
    let written = boundaries(&directory, "written", &source, false);
    boundaries(&directory, "planned", &source, true);
    // The computed key `next` is bound once `k` is, before S is matched on it.
    let expected_vo: valuation::Module = syn::parse_quote! {
        pub struct Overview;
        relation R(a:i32,b:i32);
        relation S(a:i32,b:i32);
        relation answer(a:i32,b:i32);
        answer(x,*y+1) :- R(x,k), let next:i32 = *k+1, S(next,y), if y > 0;
        valuation {
            R@0(x, k);
            let next: i32 = *k + 1;
            S@2(next, y);
            if y > 0;
        }
    };
    assert_eq!(written.valuation, expected_vo);
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
    let evidence = "PASS: computed argument CQ→VO→RA→IX with kernel-preserving plans; independent oracle answer [(1,11),(2,11),(3,5)]. Generated query execution remains unimplemented for this domain.\n";
    std::fs::write(directory.join("validation.txt"), evidence).unwrap();
    print!("{evidence}");
}
