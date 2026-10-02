use cq_planners::{Planner, check, compile, compile_valuation, cq, vo};
use cq_testkit::{Database, Rows, semantics};
use quote::quote;

#[test]
fn leaf_first_product_576_becomes_24_when_shared_keys_bind_first() {
    let source: cq::Program = syn::parse_quote! {
        struct LeafProducts;
        relation R(b:i32,a:i32); relation S(c:i32); relation T(b:i32,c:i32,k:i32);
        answer(k) :- R(b,a),S(c),T(b,c,k);
    };
    let normal = cq::normal::normalize(&source).unwrap();
    let leaf_first: vo::Program = syn::parse2(quote! {
        #normal valuation {
            R@0(_,a); R@0(b,a); T@2(b,_,_);
            S@1(c); T@2(b,c,_); T@2(b,c,k);
        }
    })
    .unwrap();
    vo::contract::check(&leaf_first).unwrap();
    let baseline = cq_elaborate::compile(&leaf_first).unwrap();
    let fixed = compile(&source, Planner::EliminationFine).unwrap();
    check(&source, &baseline).unwrap();
    check(&source, &fixed).unwrap();
    let database = Database::from([
        ("R".into(), (0..24).map(|v| vec![v, v]).collect()),
        ("S".into(), (0..24).map(|v| vec![v]).collect()),
        ("T".into(), (0..24).map(|v| vec![v, v, v]).collect()),
    ]);
    let baseline = semantics::evaluate_ra(&baseline, &database);
    let fixed = semantics::evaluate_ra(&fixed, &database);
    assert_eq!(
        baseline.definitions.values().map(|r| r.rows.len()).max(),
        Some(576)
    );
    assert_eq!(
        fixed.definitions.values().map(|r| r.rows.len()).max(),
        Some(24)
    );
    assert_eq!(baseline.output, fixed.output);
    assert_eq!(fixed.output, (0..24).map(|v| vec![v]).collect::<Rows>());
    let valuation = compile_valuation(&source, Planner::EliminationFine).unwrap();
    let vo::Step::Atom(first, _) = &valuation.valuation.steps[0] else {
        panic!("expected shared binder")
    };
    assert_eq!(first.relation, "T");
    assert_eq!(
        first
            .variables()
            .map(|(_, v)| v.to_string())
            .collect::<Vec<_>>(),
        ["b"]
    );
}

#[test]
fn compiled_leaf_regression_visits_24_completed_bindings() {
    let rows: Vec<_> = (0..24).map(|v| (v, v)).collect();
    let r = rows.as_slice();
    let s = rows.as_slice();
    let t = rows.as_slice();
    let visits = std::cell::Cell::new(0);
    // Counting is solely a cost probe, outside the partial-function denotation.
    let observe = |_: &i32, _: &i32| {
        visits.set(visits.get() + 1);
        true
    };
    let got: Rows = mini_linq::compile! {
        pipeline { plan = cq_planners::order(elimination, fine); }
        struct LeafProducts;
        relation r(k:i32,a:i32); relation s(k:i32,b:i32); relation t(k:i32,c:i32);
        answer(k) :- r(k,a),s(k,b),t(k,c),if observe(b,c);
    }
    .map(|(k,)| vec![*k])
    .collect();
    assert_eq!(got, (0..24).map(|v| vec![v]).collect());
    assert_eq!(visits.get(), 24);
}
