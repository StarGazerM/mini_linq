//! This suite invokes logical planning only; physical lowering is not needed.
//! It checks the Yannakakis schedule `cq_reduce` writes into a valuation plan:
//! every reduction precedes the first atom step, the upward sweep reduces a
//! parent with a child only after that child's own children, the downward
//! sweep reduces a child with its parent only after the parent (or from the
//! root), each tree edge is used exactly once per direction, and the
//! elaborated RA carries one semijoin per reduction.
use cq_lang_vo as valuation;
use cq_lang_vo::cq;
use cq_lang_vo::cq::hypergraph::Analysis;
use cq_lang_vo::{Reduction, Step};
use cq_reduce::{Options, Reductions};
use quote::quote;
use std::collections::BTreeSet;

const TREE: Options = Options {
    semijoins: Reductions::Tree,
};

/// One full subatom per atom in written order: the coarse plan the reducer
/// extends.
fn coarse(source: &cq::Program) -> valuation::Program {
    let steps = source.query.body.iter().enumerate().map(|(clause, item)| {
        let cq::BodyItem::Positive { atom } = item else {
            if let cq::BodyItem::Equality(e) = item {
                return quote!(#e;);
            }
            unreachable!("every fixture has relational steps only");
        };
        let relation = &atom.relation;
        let clause = proc_macro2::Literal::usize_unsuffixed(clause);
        let columns = atom.variables();
        quote!(#relation @ #clause (#(#columns),*);)
    });
    let program = &source;
    syn::parse2(quote! { #program valuation { #(#steps)* } }).unwrap()
}

/// The reducer's `reduce … with …` steps as `(target, support)` clauses, all
/// of them before the first atom step.
fn reductions(plan: &valuation::Program) -> Vec<(usize, usize)> {
    let first_atom = plan
        .valuation
        .steps
        .iter()
        .position(|step| matches!(step, Step::Atom(..)))
        .expect("every fixture has an atom");
    assert!(
        plan.valuation.steps[first_atom..]
            .iter()
            .all(|step| !matches!(step, Step::Reduce(..))),
        "a reduction after the first atom step"
    );
    plan.valuation.steps[..first_atom]
        .iter()
        .map(|step| match step {
            Step::Reduce(reduce, _) => match &reduce.reduction {
                Reduction::With(_, support) => (reduce.clause(), support.clause()),
            },
            _ => unreachable!("only reductions precede the first atom"),
        })
        .collect()
}

type JoinTree = Vec<(usize, Option<usize>)>;
type ReductionsSchedule = Vec<(usize, usize)>;

fn schedule(text: &str) -> (JoinTree, ReductionsSchedule) {
    let source: cq::Program = syn::parse_str(text).unwrap();
    let normal = cq::normal::normalize(&source).unwrap();
    let plan = cq_reduce::compile(&coarse(&normal), &TREE)
        .expect("YA scheduling exercise must emit complete legal reductions");
    cq_reduce::check(&coarse(&normal), &plan, &TREE).unwrap();
    let tree = Analysis::of(&normal)
        .join_tree
        .expect("every fixture is acyclic");
    let pairs = reductions(&plan);
    (tree, pairs)
}

#[test]
fn reductions_respect_tree_dependencies_and_exactly_once_obligations() {
    let cases = [
        "struct One; relation R(a:i32); answer(a) :- R(a);",
        "struct Chain; relation E(a:i32,b:i32); answer(a,f) :- E(a,b),E(b,c),E(c,d),E(d,e),E(e,f);",
        "struct Star; relation E(a:i32,b:i32); answer(a,b,c,d) :- E(a,b),E(a,c),E(a,d);",
        "struct Branches; relation E(a:i32,b:i32); answer(a,c,e,g) :- E(a,b),E(b,c),E(a,d),E(d,e),E(a,f),E(f,g);",
        "struct Composite; relation R(a:i32,b:i32,c:i32); relation S(a:i32,b:i32,d:i32); answer(c,d) :- R(a,b,c),S(a,b,d);",
        "struct Disconnected; relation E(a:i32,b:i32); answer(a,d) :- E(a,b),E(c,d);",
        "struct Repeated; relation E(a:i32,b:i32); answer(a,b) :- E(a,b),E(a,b);",
    ];
    for text in cases {
        let (tree, pairs) = schedule(text);
        let nodes = tree.len();
        let parent = |clause: usize| {
            tree.iter()
                .find(|(node, _)| *node == clause)
                .expect("atom clause in tree")
                .1
        };
        let children = |clause: usize| {
            tree.iter()
                .filter_map(move |&(child, p)| (p == Some(clause)).then_some(child))
        };
        assert_eq!(pairs.len(), 2 * (nodes - 1), "{text}");
        let mut up = BTreeSet::new();
        let mut down = BTreeSet::new();
        for &(target, support) in &pairs {
            if down.is_empty() && parent(support) == Some(target) {
                // Upward: the parent is reduced with a child whose own
                // children are already up.
                assert!(children(support).all(|descendant| up.contains(&descendant)));
                assert!(up.insert(support), "duplicate upward edge");
            } else {
                // Downward: the child is reduced with its parent, itself
                // already down unless it is the root.
                assert_eq!(
                    up.len() + 1,
                    nodes,
                    "downward sweep before the upward sweep ended"
                );
                assert_eq!(
                    parent(target),
                    Some(support),
                    "{text}: ({target}, {support})"
                );
                assert!(parent(support).is_none() || down.contains(&support));
                assert!(down.insert(target), "duplicate downward edge");
            }
        }
        assert_eq!(up, down);
        assert_eq!(up.len() + 1, nodes);
    }
}

#[test]
fn the_schedules_are_pinned() {
    // Shared variables are direct joins; atom clause numbers are contiguous.
    // Chain: up from E@0 to E@4, then down again.
    let (tree, pairs) = schedule(
        "struct Chain; relation E(a:i32,b:i32); answer(a,f) :- E(a,b),E(b,c),E(c,d),E(d,e),E(e,f);",
    );
    assert_eq!(
        tree,
        [
            (0, Some(1)),
            (1, Some(2)),
            (2, Some(3)),
            (3, Some(4)),
            (4, None)
        ]
    );
    assert_eq!(
        pairs,
        [
            (1, 0),
            (2, 1),
            (3, 2),
            (4, 3),
            (3, 4),
            (2, 3),
            (1, 2),
            (0, 1)
        ]
    );
    // Branches: breadth-first from the root E@5, the upward sweep visits the
    // deepest edges first and the downward sweep the shallowest.
    let (tree, pairs) = schedule(
        "struct Branches; relation E(a:i32,b:i32); answer(a,c,e,g) :- E(a,b),E(b,c),E(a,d),E(d,e),E(a,f),E(f,g);",
    );
    assert_eq!(
        tree,
        [
            (0, Some(2)),
            (1, Some(0)),
            (2, Some(4)),
            (3, Some(2)),
            (4, Some(5)),
            (5, None)
        ]
    );
    assert_eq!(
        pairs,
        [
            (0, 1),
            (2, 3),
            (2, 0),
            (4, 2),
            (5, 4),
            (4, 5),
            (2, 4),
            (0, 2),
            (3, 2),
            (1, 0)
        ]
    );
    // Composite key, disconnected atoms (an empty separator tests
    // nonemptiness) and a repeated occurrence: one edge, up then down.
    for text in [
        "struct Composite; relation R(a:i32,b:i32,c:i32); relation S(a:i32,b:i32,d:i32); answer(c,d) :- R(a,b,c),S(a,b,d);",
        "struct Disconnected; relation E(a:i32,b:i32); answer(a,d) :- E(a,b),E(c,d);",
        "struct Repeated; relation E(a:i32,b:i32); answer(a,b) :- E(a,b),E(a,b);",
    ] {
        let (tree, pairs) = schedule(text);
        assert_eq!(tree, [(0, Some(1)), (1, None)], "{text}");
        assert_eq!(pairs, [(1, 0), (0, 1)], "{text}");
    }
}

#[test]
fn a_cyclic_query_has_no_schedule() {
    let source: cq::Program = syn::parse_quote! {
        struct Triangle;
        relation E(a:i32,b:i32);
        answer(x) :- E(x,y), E(y,z), E(z,x);
    };
    assert!(cq_reduce::compile(&coarse(&source), &TREE).is_err());
}
