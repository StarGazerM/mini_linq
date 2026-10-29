//! Logical planning and an independently authored RA; no physical execution.
use cq_lang_cq as cq;
use cq_lang_ra as ra;
use mini_linq_paper_overview::{boundaries, directory, write};
use std::collections::BTreeSet;

#[derive(Clone, Default)]
struct Input {
    r: Vec<(i32, i32)>,
    s: Vec<(i32, i32)>,
    t: Vec<(i32, i32)>,
    u: Vec<(i32, i32, i32)>,
    v: Vec<(i32,)>,
}

fn oracle(input: &Input) -> Vec<(i32, i32)> {
    let mut answer = BTreeSet::new();
    // Enumerate whole input tuples. No compiler, RA interpreter, or index helper.
    for &(a, x) in &input.r {
        for &(sa, y) in &input.s {
            for &(ty, z) in &input.t {
                for &(p, q, w) in &input.u {
                    for &(vw,) in &input.v {
                        if a == sa
                            && y == ty
                            && p == x.wrapping_add(y).wrapping_add(y)
                            && q == x.wrapping_add(z)
                            && x > 0
                            && x.wrapping_add(y) <= z
                            && w == vw
                        {
                            answer.insert((a, w));
                        }
                    }
                }
            }
        }
    }
    answer.into_iter().collect()
}

fn main() {
    let authored: ra::Program = syn::parse_quote! {

            relational {
    relation R(a:i32,b:i32);
    relation S(a:i32,b:i32);
    relation T(a:i32,b:i32);
    relation U(a:i32,b:i32,c:i32);
    relation V(a:i32);
    relation answer(a:i32,b:i32);
                r_r = rename R@0 {a -> a, b -> x};
                r_a = select r_r if (x > 0);
                r_s = rename S@1 {a -> a, b -> y};
                r_rs = natural_join r_a with r_s;
                r_t = rename T@2 {a -> y, b -> z};
                r_rst = natural_join r_rs with r_t;
                r_b = bind r_rst with k:i32 = (combine(*x,*y));
                r_p = bind r_b with p:i32 = (left_key(k,*y));
                r_q = bind r_p with q:i32 = (right_key(*x,*z));
                r_d = project r_q keep {a,p,q,k,z};
                r_u = rename U@6 {a -> p, b -> q, c -> w};
                r_v = rename V@9 {a -> w};
                r_uv = semijoin r_u with r_v;
                r_join = natural_join r_d with r_uv;
                r_c = guard r_join if (permitted(k,*z));
                r_out = project r_c keep {a,w};
                output r_out as answer(a,w);
            }
        };
    ra::contract::check(&authored).unwrap();
    let ix = cq_index_policy::compile(&authored).unwrap();
    cq_lang_ix::contract::check(&ix).unwrap();
    let source: cq::Program = syn::parse_quote! {
        struct Authored;
        relation R(a:i32,b:i32); relation S(a:i32,b:i32);
        relation T(a:i32,b:i32); relation U(a:i32,b:i32,c:i32);
        relation V(a:i32); relation answer(a:i32,b:i32);
        answer(a,w) :- R(a,x), S(a,y), T(y,z),
            let k:i32 = combine(*x,*y),
            let p:i32 = left_key(k,*y), let q:i32 = right_key(*x,*z),
            U(p,q,w), where x > 0, if permitted(k,*z), V(w);
    };
    let directory = directory("logical-constraint-planning");
    let checked = boundaries(&directory, "written", &source, false);
    cq_elaborate::check(&checked.valuation, &authored).unwrap();
    boundaries(&directory, "planned", &source, true);
    write(&directory, "authored-04-relational.tokens", &authored);
    write(&directory, "authored-05-indexes.tokens", &ix);
    let input = Input {
        r: vec![(1, 1), (1, 2), (2, -1), (3, 3)],
        s: vec![(1, 2), (2, 1), (3, 2)],
        t: vec![(2, 5), (2, 2), (1, 5)],
        u: vec![
            (5, 6, 10),
            (5, 6, 11),
            (6, 7, 12),
            (7, 8, 13),
            (5, 7, 14),
            (99, 99, 15),
            (1, 4, 16),
        ],
        v: vec![(10,), (12,), (13,), (14,), (15,), (16,)],
    };
    assert_eq!(oracle(&input), [(1, 10), (1, 12), (3, 13)]);
    let duplicate = Input {
        r: input.r.iter().rev().chain(&input.r).copied().collect(),
        s: input.s.iter().rev().chain(&input.s).copied().collect(),
        t: input.t.iter().rev().chain(&input.t).copied().collect(),
        u: input.u.iter().rev().chain(&input.u).copied().collect(),
        v: input.v.iter().rev().chain(&input.v).copied().collect(),
    };
    assert_eq!(oracle(&duplicate), oracle(&input));
    assert!(oracle(&Input::default()).is_empty());
    for empty in 0..5 {
        let mut case = input.clone();
        match empty {
            0 => case.r.clear(),
            1 => case.s.clear(),
            2 => case.t.clear(),
            3 => case.u.clear(),
            4 => case.v.clear(),
            _ => unreachable!(),
        }
        assert!(oracle(&case).is_empty());
    }
    println!(
        "PASS: automatic and authored RA/IX contracts; independent whole-tuple oracle and duplicate/empty fixtures. No generated-query semantic equality or intermediate runtime counts are claimed."
    );
}
