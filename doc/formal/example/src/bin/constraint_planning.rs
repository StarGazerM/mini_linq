//! Compare an extended query with an independently authored optimized RA plan.
//! The current planner is not claimed to discover the authored plan.
use std::collections::BTreeSet;

fn combine(x: i32, y: i32) -> i32 {
    x.wrapping_add(y)
}
fn left_key(k: i32, y: i32) -> i32 {
    k.wrapping_add(y)
}
fn right_key(x: i32, z: i32) -> i32 {
    x.wrapping_add(z)
}
fn permitted(k: i32, z: i32) -> bool {
    k <= z
}

mini_linq::mini_linq! {
    struct Lazy;
    relation R(a:i32,b:i32); relation S(a:i32,b:i32);
    relation T(a:i32,b:i32); relation U(a:i32,b:i32,c:i32);
    relation V(a:i32); relation answer(a:i32,b:i32);
    answer(a,w) :- R(a,x), S(a,y), T(y,z),
        let k:i32 = combine(*x,*y),
        U(left_key(k,*y),right_key(*x,*z),w),
        if *x > 0, if permitted(k,*z), V(w);
}

mini_linq::planned_query! {
    struct Planned;
    relation R(a:i32,b:i32); relation S(a:i32,b:i32);
    relation T(a:i32,b:i32); relation U(a:i32,b:i32,c:i32);
    relation V(a:i32); relation answer(a:i32,b:i32);
    answer(a,w) :- R(a,x), S(a,y), T(y,z),
        let k:i32 = combine(*x,*y),
        U(left_key(k,*y),right_key(*x,*z),w),
        if *x > 0, if permitted(k,*z), V(w);
}

mini_linq::planned_relational_plan! {
    struct Authored;
    relation R(a:i32,b:i32); relation S(a:i32,b:i32);
    relation T(a:i32,b:i32); relation U(a:i32,b:i32,c:i32);
    relation V(a:i32); relation answer(a:i32,b:i32);
    answer(a,w) :- R(a,x), S(a,y), T(y,z),
        let k:i32 = combine(*x,*y),
        U(left_key(k,*y),right_key(*x,*z),w),
        if *x > 0, if permitted(k,*z), V(w);
    relational {
        r_r = rename R {a -> a, b -> x};
        r_a = select r_r if (*x > 0);
        r_s = rename S {a -> a, b -> y};
        r_rs = natural_join r_a with r_s;
        r_b = extend r_rs with {k:i32} value ((combine(*x,*y),));
        r_t = rename T {a -> y, b -> z};
        r_bt = natural_join r_b with r_t;
        r_c = select r_bt if (permitted(k,*z));
        r_keys = extend r_c with {p:i32,q:i32}
            value ((left_key(k,*y),right_key(*x,*z)));
        r_d = project r_keys keep {a,p,q};
        r_u = rename U {a -> p, b -> q, c -> w};
        r_v = rename V {a -> w};
        r_uv = semijoin r_u with r_v;
        r_join = natural_join r_d with r_uv;
        r_out = project r_join keep {a,w};
        output r_out as answer(a,w);
    }
}

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

fn check(input: &Input) {
    let expected = oracle(input);
    assert_eq!(
        Lazy::run(
            input.r.iter().copied(),
            input.s.iter().copied(),
            input.t.iter().copied(),
            input.u.iter().copied(),
            input.v.iter().copied()
        ),
        expected
    );
    assert_eq!(
        Planned::run(
            input.r.iter().copied(),
            input.s.iter().copied(),
            input.t.iter().copied(),
            input.u.iter().copied(),
            input.v.iter().copied()
        ),
        expected
    );
    assert_eq!(
        Authored::run(
            input.r.iter().copied(),
            input.s.iter().copied(),
            input.t.iter().copied(),
            input.u.iter().copied(),
            input.v.iter().copied()
        ),
        expected
    );
}

fn reversed_duplicates<T: Clone>(rows: &[T]) -> Vec<T> {
    rows.iter().rev().chain(rows).cloned().collect()
}

fn main() {
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
    let (_, stats) = Authored::run_with_stats(
        input.r.iter().copied(),
        input.s.iter().copied(),
        input.t.iter().copied(),
        input.u.iter().copied(),
        input.v.iter().copied(),
    );
    for expected in [
        ("r_a", 2, 3),
        ("r_b", 4, 3),
        ("r_bt", 5, 6),
        ("r_c", 5, 3),
        ("r_d", 3, 3),
        ("r_uv", 3, 6),
        ("r_out", 2, 3),
    ] {
        assert!(
            stats.results.contains(&expected),
            "missing {expected:?}: {:?}",
            stats.results
        );
    }
    let mut cases = vec![input.clone(), Input::default()];
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
        cases.push(case);
    }
    for case in &cases {
        check(case);
        check(&Input {
            r: reversed_duplicates(&case.r),
            s: reversed_duplicates(&case.s),
            t: reversed_duplicates(&case.t),
            u: reversed_duplicates(&case.u),
            v: reversed_duplicates(&case.v),
        });
    }
    println!("Three public compilation routes agree on 14 input presentations (42 executions).");
    println!(
        "Authored input answer: [(1,10),(1,12),(3,13)]; selected intermediate arities/counts verified."
    );
}
