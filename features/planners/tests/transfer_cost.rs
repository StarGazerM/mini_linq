use cq_planners::{Planner, check, compile, cq};
use cq_testkit::{Database, Rows, semantics};

#[test]
fn transfer_connects_late_bridge_instead_of_expanding_leaf_product() {
    let source: cq::Program = syn::parse_quote! {
        struct LateBridge;
        relation R(x:i32); relation S(y:i32); relation T(x:i32,y:i32);
        answer(x,y) :- R(x), S(y), T(x,y);
    };
    let written = compile(&source, Planner::WrittenCoarse).unwrap();
    let transfer = compile(&source, Planner::Transfer).unwrap();
    check(&source, &transfer).unwrap();
    for n in [0, 1, 8, 24] {
        let database = Database::from([
            ("R".into(), (0..n).map(|v| vec![v]).collect()),
            ("S".into(), (0..n).map(|v| vec![v]).collect()),
            ("T".into(), (0..n).map(|v| vec![v, v]).collect()),
        ]);
        let written = semantics::evaluate_ra(&written, &database);
        let transfer = semantics::evaluate_ra(&transfer, &database);
        let expected: Rows = (0..n).map(|v| vec![v, v]).collect();
        assert_eq!(written.output, expected);
        assert_eq!(transfer.output, expected);
        assert_eq!(
            written.definitions.values().map(|r| r.rows.len()).max(),
            Some((n * n) as usize)
        );
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
