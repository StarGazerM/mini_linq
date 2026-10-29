//! Sequence pipelines preserve independently authored expected answers.
use std::collections::BTreeSet;

macro_rules! check {
    ($expected:expr; $($query:tt)*) => {{
        let left = [(1i32, 2i32), (1, 2), (2, 3), (3, 4)];
        let right = [(2i32, 7i32), (2, 8), (3, 9), (5, 10)];
        let extra = [(7i32, 2i32), (9, 3)];
        let actual: BTreeSet<_> = mini_linq::compile! {
            pipeline {
                sequence = ::mini_linq::passes::sequence;
                grade = ::mini_linq::passes::sequential;
                print = ::mini_linq::passes::translate_pull;
            }
            struct Query;
            relation L(a:i32,b:i32) = (left);
            relation R(b:i32,c:i32) = (right);
            relation S(c:i32,b:i32) = (extra);
            $($query)*
        }.collect();
        let expected = BTreeSet::from($expected);
        assert_eq!(actual, expected);
    }};
}

#[test]
fn join() {
    check!([(&1, &7), (&1, &8), (&2, &9)]; answer(a,c) :- L(a,b), R(b,c););
}

#[test]
fn constant_atom_stream() {
    check!([(&2,)]; answer(b) :- L(1,b););
}

#[test]
fn constant_atom_probed() {
    check!([(&1,)]; answer(a) :- L(a,b), R(b,7););
}

#[test]
fn semi() {
    check!([(&1, &2), (&2, &3)]; answer(a,b) :- L(a,b), R(b,_););
}

#[test]
fn dedup() {
    check!([(&1,), (&2,), (&3,)]; answer(a) :- L(a,_););
}

#[test]
fn union_block() {
    check!([(&1, &7), (&1, &8), (&2, &9)]; answer(a,c) :- L(a,b), { R(b,c) } | { S(c,b) };);
}

#[test]
fn positional_point_and_range_reads_preserve_their_source_spine() {
    let rows = [(1i32, 4i32), (2, 3), (2, 3), (3, 2), (4, 1)];
    let cutoff = 2;
    macro_rules! check {
        ($expected:expr; $($query:tt)*) => {
            let actual: BTreeSet<_> = mini_linq::compile! {
                pipeline {
                    select = ::mini_linq::passes::select(positional(R));
                    sequence = ::mini_linq::passes::sequence;
                    grade = ::mini_linq::passes::sequential;
                    print = ::mini_linq::passes::translate_pull;
                }
                struct Query;
                relation R(a:i32,b:i32) = (rows);
                $($query)*
            }.collect();
            let expected = BTreeSet::from($expected);
            assert_eq!(actual, expected);
        };
    }
    check!([(&2, &3)]; answer(a,b) :- R(a,b), where a == cutoff;);
    check!([(&2, &3), (&3, &2)]; answer(a,b) :- R(a,b), where a >= cutoff, where b >= cutoff;);
}

#[test]
fn retained_positional_range_keeps_its_host_source() {
    let r0 = [(1i32, 2i32), (2, 3), (3, 1)];
    let r1 = [(4i32, 1i32), (5, 2)];
    let actual: BTreeSet<_> = mini_linq::passes::select! {
        arguments { positional(r0,r1) };
        pipeline {
            mini_linq::passes::sequence,
            mini_linq::passes::sequential,
            mini_linq::passes::translate_pull
        };
        physical {
                    relation r0(c0:i32,c1:i32);
                    relation r1(c0:i32,c1:i32);
                    r2 = pipe rename r0@0 {c0 -> v0,c1 -> v1};
                    r3 = let range select r2 if (2 <= v1);
                    r4 = pipe canon project r3 keep {v0};
                    r5 = pipe probe first [v0] r4,r3;
                    r6 = pipe rename r1@1 {c0 -> v2,c1 -> v3};
                    r7 = let canon project r6 keep {v2};
                    r8 = pipe probe first [] r5,r7;
                    r9 = pipe probe first [v2] r8,r6;
                    r10 = pipe select r9 if ((v3 <= v0 and 0 <= v1 and v1 >= v0) or (v0 <= v3));
                    r11 = pipe bind r10 with v4:i32 = (v0.to_owned() + -1);
                    r12 = pipe canon project r11 keep {v1,v2};
                    return r12 as answer(v1,v2);
                }
    }
    .collect();
    let expected = BTreeSet::from([(&2, &4), (&2, &5), (&3, &4), (&3, &5)]);
    assert_eq!(actual, expected);
}
