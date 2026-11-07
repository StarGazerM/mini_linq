use cq_planners::{Planner, check, compile, cq, ra};
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
fn later_full_witness_repairs_only_key_preserving_positive_lineage() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); relation T(a:i32,b:i32,c:i32);
        r = rename R {a -> x,b -> y};
        t = rename T {a -> x,b -> y,c -> z};
        late = select r if (y > 0, y < 3);
        result = natural_join t with late;
        output result as answer(x,y,z);
    }};
    macro_rules! witness {
        ($($tail:tt)*) => { syn::parse_quote! { relational {
            relation R(a:i32,b:i32); relation T(a:i32,b:i32,c:i32);
            r = rename R {a -> x,b -> y};
            t = rename T {a -> x,b -> y,c -> z};
            early = select r if (y > 0);
            late = select early if (y < 3);
            projected = project late keep {x};
            prefix = natural_join t with early;
            requested = natural_join prefix with projected;
            $($tail)*
        }} };
    }
    let target: ra::Program = witness! {
        result = natural_join requested with late;
        output result as answer(x,y,z);
    };
    ra::check_rewrite(&source, &target).unwrap();
    ra::check_rewrite(&target, &source).unwrap();
    let database = Database::from([
        ("R".into(), [vec![1, 1], vec![1, 4]].into()),
        ("T".into(), [vec![1, 1, 7], vec![1, 4, 9]].into()),
    ]);
    let expected = semantics::evaluate_ra(&source, &database).output;
    assert_eq!(expected, [vec![1, 1, 7]].into_iter().collect::<Rows>());
    assert_eq!(semantics::evaluate_ra(&target, &database).output, expected);

    let mutations: [(&str, ra::Program); 3] = [
        ("lost full-row key", witness! {
            result = natural_join requested with projected;
            output result as answer(x,y,z);
        }),
        ("unrelated raw full read", witness! {
            result = natural_join requested with r;
            output result as answer(x,y,z);
        }),
        ("dropped participating binding", witness! {
            cut = project requested keep {x,z};
            result = natural_join cut with late;
            output result as answer(x,y,z);
        }),
    ];
    for (reason, changed) in mutations {
        ra::contract::check(&changed).unwrap();
        assert!(ra::check_rewrite(&source, &changed).is_err(), "{reason}");
        assert!(ra::check_rewrite(&changed, &source).is_err(), "{reason}");
        // Every mutation admits a real spurious row, not just a different DAG.
        assert_ne!(semantics::evaluate_ra(&changed, &database).output, expected, "{reason}");
    }

    let mut guarded_source = source.clone();
    guarded_source.plan.definitions.push(
        syn::parse_quote!(observed = guard result if (opaque(y));),
    );
    guarded_source.plan.output.input = syn::parse_quote!(observed);
    let guarded_target: ra::Program = witness! {
        observed = guard requested if (opaque(y));
        result = natural_join observed with late;
        output result as answer(x,y,z);
    };
    ra::contract::check(&guarded_target).unwrap();
    assert!(ra::check_rewrite(&guarded_source, &guarded_target).is_err());
    assert!(ra::check_rewrite(&guarded_target, &guarded_source).is_err());
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
    let valuation = cq_planners::compile_valuation(&source, Planner::Transfer).unwrap();
    let first: Vec<_> = valuation.valuation.steps.iter().filter_map(|step| match step {
        cq_planners::vo::Step::Atom(atom, _) => Some(atom.relation.to_string()),
        _ => None,
    }).take(2).collect();
    assert_eq!(first, ["R", "P"]);
    let lowered = cq_relational_lowering::compile(&valuation).unwrap();
    cq_relational_lowering::check(&valuation, &lowered).unwrap();
    let target = compile(&source, Planner::Transfer).unwrap();
    check(&source, &target).unwrap();
    for seed in 0..4 {
        let database = cq_testkit::databases::binary_database(&valuation.cq, seed);
        assert_eq!(
            semantics::evaluate_ra(&target, &database).output,
            cq_testkit::cq_semantics::evaluate_cq(&valuation.cq, &database),
            "database {seed}",
        );
    }
}
