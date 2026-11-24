use cq_planners::{Planner, check, compile, cq};
use cq_testkit::{Database, Rows, semantics};

#[test]
fn transfer_connects_late_bridge_instead_of_expanding_leaf_product() {
    let source: cq::Program = syn::parse_quote! {
        struct LateBridge;
        relation R(x:i32); relation S(y:i32); relation T(x:i32,y:i32);
        answer(x,y) :- R(x), S(y), T(x,y);
    };
    let error = compile(&source, Planner::WrittenCoarse).unwrap_err().to_string();
    assert!(error.contains("I3"), "{error}");
    let transfer = compile(&source, Planner::Transfer).unwrap();
    check(&source, &transfer).unwrap();
    for n in [0, 1, 8, 24] {
        let database = Database::from([
            ("R".into(), (0..n).map(|v| vec![v]).collect()),
            ("S".into(), (0..n).map(|v| vec![v]).collect()),
            ("T".into(), (0..n).map(|v| vec![v, v]).collect()),
        ]);
        let transfer = semantics::evaluate_ra(&transfer, &database);
        let expected: Rows = (0..n).map(|v| vec![v, v]).collect();
        assert_eq!(transfer.output, expected);
        assert_eq!(
            transfer.definitions.values().map(|r| r.rows.len()).max(),
            Some(n as usize),
            "a fully reduced connected join must not expand a disconnected prefix"
        );
    }
}

#[test]
fn transfer_preserves_partial_guard_domain_while_connecting_later_atoms() {
    let t = [(1_i32, 7_i32), (2, 8), (2, 8)];
    let s = [(7_i32,), (8,), (9,)];
    // The second guard must remain after its written nonzero restriction.
    // Its division prevents treating the entire prefix as a total predicate.
    for input in [vec![], vec![(0_i32,), (1,), (2,), (2,)]] {
        let r = input.as_slice();
        let actual: Rows = mini_linq::compile! {
            pipeline { plan = cq_planners::order(transfer); }
            struct GuardedBridge;
            relation r(x:i32); relation s(y:i32); relation t(x:i32,y:i32);
            answer(x,y) :- r(x), if *x != 0, if 2 / *x > 0, s(y), t(x,y);
        }
        .map(|(x, y)| vec![*x, *y])
        .collect();
        let expected: Rows = input
            .iter()
            .filter(|(x,)| *x != 0 && 2 / *x > 0)
            .flat_map(|(x,)| {
                s.iter()
                    .filter_map(move |(y,)| t.contains(&(*x, *y)).then(|| vec![*x, *y]))
            })
            .collect();
        assert_eq!(actual, expected);
    }
}

#[test]
fn transfer_keeps_early_membership_with_conjoined_reducer_restrictions() {
    let source: cq::Program = syn::parse_quote! {
        struct EarlyMembership;
        relation R(p:i32,s:i32); relation L(o:i32,p:i32,s:i32);
        relation O(o:i32,d:i32); relation P(p:i32); relation S(s:i32,n:i32);
        answer(o,p,n) :- R(p,s), L(o,p,s), O(o,d),
            where d > 0, where d < 3, P(p), S(s,n);
    };
    let (normal, valuation) = cq_planners::compile_valuation(&source, Planner::Transfer).unwrap();
    let first: Vec<_> = valuation.valuation.steps.iter().filter_map(|step| match step {
        cq_planners::vo::Step::Atom(atom, _) => Some(atom.relation.to_string()),
        _ => None,
    }).take(2).collect();
    assert_eq!(first, ["R", "P"]);
    let (lowered, steps) = cq_relational_lowering::compile(&normal, &valuation).unwrap();
    cq_relational_lowering::check(&normal, &valuation, &lowered, &steps).unwrap();
    let target = compile(&source, Planner::Transfer).unwrap();
    check(&source, &target).unwrap();
    for seed in 0..4 {
        let database = cq_testkit::databases::binary_database(&normal, seed);
        assert_eq!(
            semantics::evaluate_ra(&target, &database).output,
            cq_testkit::cq_semantics::evaluate_cq(&normal, &database),
            "database {seed}",
        );
    }
}
