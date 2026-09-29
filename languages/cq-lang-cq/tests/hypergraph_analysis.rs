//! Hypergraph facts of normal-form queries: GYO acyclicity with a join tree,
//! maximum-cardinality search with the perfect-elimination check, Gilmore
//! conformality, and the BFMY correspondence between them.
use std::collections::BTreeSet;

use cq_lang_cq as cq;
use cq_lang_cq::hypergraph::Analysis;
use quote::quote;

fn analyze(tokens: proc_macro2::TokenStream) -> Analysis {
    let module: cq::Module = syn::parse2(tokens).unwrap();
    cq::hypergraph::check(&module).unwrap()
}

fn names(vars: &[&str]) -> Vec<String> {
    vars.iter().map(|v| v.to_string()).collect()
}

/// A join tree's parent pointers must be a tree over exactly the atom
/// clauses whose root is the last remaining edge, and every variable's
/// clauses must be connected in it (the running-intersection property).
fn assert_join_tree(analysis: &Analysis) {
    let tree = analysis.join_tree.as_ref().expect("acyclic query");
    let clauses: Vec<usize> = tree.iter().map(|(c, _)| *c).collect();
    assert_eq!(
        clauses,
        analysis.clauses.keys().copied().collect::<Vec<_>>()
    );
    let roots: Vec<usize> = tree
        .iter()
        .filter(|(_, p)| p.is_none())
        .map(|(c, _)| *c)
        .collect();
    assert_eq!(roots.len(), 1);
    let last_absorb_parent = analysis.gyo_steps.iter().rev().find_map(|step| match step {
        cq::hypergraph::GyoStep::Absorb { parent, .. } => Some(*parent),
        _ => None,
    });
    if let Some(parent) = last_absorb_parent {
        assert_eq!(roots[0], parent);
    }
    let parent = |c: usize| tree.iter().find(|(x, _)| *x == c).unwrap().1;
    for (variable, occurrences) in &analysis.incidence {
        // A connected subtree of a rooted tree has exactly one topmost node.
        let tops = occurrences
            .iter()
            .filter(|&&c| parent(c).is_none_or(|p| !occurrences.contains(&p)))
            .count();
        assert_eq!(
            tops, 1,
            "{variable}: clauses {occurrences:?} are not connected in {tree:?}"
        );
    }
}

/// Beeri–Fagin–Maier–Yannakakis: acyclic iff conformal with a chordal primal
/// graph. The three facts are computed by different algorithms.
fn assert_bfmy(analysis: &Analysis) {
    assert_eq!(analysis.acyclic, analysis.chordal && analysis.conformal);
    assert_eq!(analysis.acyclic, analysis.join_tree.is_some());
    assert_eq!(
        analysis.elimination_order.iter().collect::<BTreeSet<_>>(),
        analysis.variables.iter().collect::<BTreeSet<_>>()
    );
}

#[test]
fn triangle_is_chordal_but_not_conformal_hence_cyclic() {
    let analysis = analyze(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        relation T(a: i32, c: i32);
        answer(x, y, z) :- R(x, y), S(y, z), T(z, x);
    });
    assert_eq!(analysis.variables, names(&["x", "y", "z"]));
    assert!(!analysis.acyclic);
    assert!(analysis.join_tree.is_none());
    // K3 is complete, so chordal; the clique {x, y, z} is no atom.
    assert!(analysis.chordal);
    assert!(!analysis.conformal);
    assert_eq!(analysis.width, 3);
    assert_eq!(analysis.primal["x"], ["y", "z"].map(String::from).into());
    assert_eq!(analysis.shared(0, 1), names(&["y"]));
    assert_eq!(analysis.shared(2, 0), names(&["x"]));
    assert_eq!(analysis.shared(0, 0), names(&["x", "y"]));
    // GYO gets stuck at once: no private variable, no containment.
    assert!(analysis.gyo_steps.is_empty());
    assert_bfmy(&analysis);
}

#[test]
fn chain_is_acyclic_with_a_join_tree_and_a_perfect_elimination_order() {
    let analysis = analyze(quote! {
        struct Q;
        relation L(a: i32);
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(a, c) :- L(a), R(a, b), S(b, c);
    });
    assert_eq!(analysis.variables, names(&["a", "b", "c"]));
    assert_eq!(analysis.clauses[&1], names(&["a", "b"]));
    assert_eq!(analysis.incidence["a"], [0, 1].into());
    assert!(analysis.acyclic);
    assert!(analysis.chordal);
    assert!(analysis.conformal);
    assert_eq!(analysis.width, 2);
    assert_eq!(analysis.elimination_order, names(&["c", "b", "a"]));
    assert_eq!(
        analysis.join_tree,
        Some(vec![(0, Some(1)), (1, Some(2)), (2, None)])
    );
    assert_join_tree(&analysis);
    assert_bfmy(&analysis);
}

#[test]
fn star_is_acyclic_with_the_centre_shared_by_every_clause() {
    let analysis = analyze(quote! {
        struct Q;
        relation R(c: i32, a: i32);
        relation S(c: i32, b: i32);
        relation T(c: i32, d: i32);
        answer(a, b, d) :- R(c, a), S(c, b), T(c, d);
    });
    assert_eq!(analysis.variables, names(&["c", "a", "b", "d"]));
    assert!(analysis.acyclic);
    assert!(analysis.chordal);
    assert!(analysis.conformal);
    assert_eq!(analysis.width, 2);
    assert_eq!(
        analysis.primal["c"],
        ["a", "b", "d"].map(String::from).into()
    );
    assert_eq!(analysis.primal["a"], ["c"].map(String::from).into());
    for clause in 0..3 {
        for other in 0..3 {
            assert!(analysis.shared(clause, other).contains(&"c".to_string()));
        }
    }
    let tree = analysis.join_tree.as_ref().unwrap();
    assert_eq!(tree.iter().filter(|(_, p)| p.is_none()).count(), 1);
    assert_join_tree(&analysis);
    assert_bfmy(&analysis);
}

#[test]
fn self_join_keeps_clause_indices_distinct() {
    let analysis = analyze(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        answer(x, z) :- R(x, y), R(y, z);
    });
    assert_eq!(analysis.variables, names(&["x", "y", "z"]));
    assert_eq!(analysis.clauses[&0], names(&["x", "y"]));
    assert_eq!(analysis.clauses[&1], names(&["y", "z"]));
    assert_eq!(analysis.incidence["y"], [0, 1].into());
    assert_eq!(analysis.shared(0, 1), names(&["y"]));
    assert!(analysis.acyclic);
    assert_eq!(analysis.join_tree, Some(vec![(0, Some(1)), (1, None)]));
    assert_eq!(analysis.width, 2);
    assert_join_tree(&analysis);
    assert_bfmy(&analysis);
}

#[test]
fn four_cycle_is_conformal_but_not_chordal_hence_cyclic() {
    let analysis = analyze(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(a: i32, b: i32);
        relation T(a: i32, b: i32);
        relation U(a: i32, b: i32);
        answer(a, b, c, d) :- R(a, b), S(b, c), T(c, d), U(d, a);
    });
    assert!(!analysis.acyclic);
    assert!(analysis.gyo_steps.is_empty());
    assert!(!analysis.chordal);
    assert!(analysis.conformal);
    // Eliminating the first variable fills its two neighbours into a
    // triangle; the width of that triangulation is 2, treewidth of C4.
    assert_eq!(analysis.width, 3);
    assert_eq!(analysis.shared(0, 2), Vec::<String>::new());
    assert_bfmy(&analysis);
}

#[test]
fn absorbed_and_bound_variables_keep_the_tree_and_the_facts_honest() {
    // S is contained in R once its private column goes; `d` is let-bound and
    // never an atom variable, so it is no vertex.
    let analysis = analyze(quote! {
        struct Q;
        relation R(a: i32, b: i32, c: i32);
        relation S(b: i32, c: i32, e: i32);
        answer(a, d) :- R(a, b, c), S(b, c, e), if a > 0, let d: i32 = a + c;
    });
    assert_eq!(analysis.variables, names(&["a", "b", "c", "e"]));
    assert!(analysis.acyclic);
    assert_eq!(analysis.width, 3);
    assert_eq!(analysis.shared(0, 1), names(&["b", "c"]));
    // Both private columns go first; the remaining equal edges absorb 0 into 1.
    assert_eq!(
        analysis.gyo_steps,
        [
            cq::hypergraph::GyoStep::Private {
                clause: 0,
                variable: "a".into()
            },
            cq::hypergraph::GyoStep::Private {
                clause: 1,
                variable: "e".into()
            },
            cq::hypergraph::GyoStep::Absorb {
                child: 0,
                parent: 1
            },
            cq::hypergraph::GyoStep::Private {
                clause: 1,
                variable: "b".into()
            },
            cq::hypergraph::GyoStep::Private {
                clause: 1,
                variable: "c".into()
            },
        ]
    );
    assert_eq!(analysis.join_tree, Some(vec![(0, Some(1)), (1, None)]));
    assert_join_tree(&analysis);
    assert_bfmy(&analysis);
}

#[test]
fn analysis_requires_normal_form() {
    let module: cq::Module = syn::parse2(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        answer(x) :- R(x, 5);
    })
    .unwrap();
    assert!(cq::hypergraph::check(&module).is_err());
    let normal = cq::normal::normalize(&module).unwrap();
    let analysis = cq::hypergraph::check(&normal).unwrap();
    // The constant became a let-bound join key, so it is a vertex of R.
    assert_eq!(
        analysis.clauses[&analysis.clauses.keys().next().copied().unwrap()].len(),
        2
    );
    assert!(analysis.acyclic);
    assert_bfmy(&analysis);
}
