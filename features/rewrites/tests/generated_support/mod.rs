use cq_testkit::{Database, Rows, cq, cq_semantics};

pub type Edge = (i32, i32);
pub type Inputs = [Vec<Edge>; 3];
pub type Run = fn(&[Edge], &[Edge], &[Edge]) -> Rows;

macro_rules! triangle {
    ($r:expr, $s:expr, $t:expr; $($pipeline:tt)*) => {
        mini_linq::compile! {
            pipeline { $($pipeline)* }
            struct Triangle;
            relation r(a:i32,b:i32) = ($r);
            relation s(b:i32,c:i32) = ($s);
            relation t(a:i32,c:i32) = ($t);
            answer(a,b,c) :- r(a,b),s(b,c),t(a,c);
        }
    };
}
pub fn binary(r: &[Edge], s: &[Edge], t: &[Edge]) -> Rows {
    collect(triangle! {
        r,s,t;
        plan = cq_planners::order(written, coarse);
    })
}

pub fn generic(r: &[Edge], s: &[Edge], t: &[Edge]) -> Rows {
    collect(triangle! {
        r,s,t;
        plan = cq_planners::order(written, fine);
        physical = [cq_rewrites::canon_prefixes, cq_rewrites::smallest_intersections];
    })
}

fn collect<'a>(output: impl Iterator<Item = (&'a i32, &'a i32, &'a i32)>) -> Rows {
    let output: Vec<_> = output.map(|(a, b, c)| vec![*a, *b, *c]).collect();
    let count = output.len();
    let distinct: Rows = output.into_iter().collect();
    assert_eq!(count, distinct.len(), "generated query emitted duplicates");
    distinct
}

pub fn source() -> cq::Program {
    syn::parse_quote! {
        struct Triangle;
        relation r(a:i32,b:i32);
        relation s(b:i32,c:i32);
        relation t(a:i32,c:i32);
        answer(a,b,c) :- r(a,b),s(b,c),t(a,c);
    }
}

pub fn oracle(inputs: &Inputs) -> Rows {
    let database: Database = ["r", "s", "t"]
        .into_iter()
        .zip(inputs)
        .map(|(name, rows)| {
            (
                name.to_owned(),
                rows.iter().map(|&(a, b)| vec![a, b]).collect(),
            )
        })
        .collect();
    let source = source();
    cq::contract::check(&source).unwrap();
    cq_semantics::evaluate_cq(&source, &database)
}

/// The workload request's symmetric O(n)-input fan with a planted triangle.
/// Each candidate side alternates between a bucket of n rows and a singleton.
pub fn fan(scale: i32) -> Inputs {
    assert!(scale >= 3);
    let mut edges: Vec<_> = (1..=scale).flat_map(|i| [(0, i), (i, 0)]).collect();
    edges.extend([(1, 2), (2, 3), (1, 3)]);
    [edges.clone(), edges.clone(), edges]
}
