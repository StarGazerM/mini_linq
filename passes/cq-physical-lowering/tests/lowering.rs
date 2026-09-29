use cq_lang_ix::{self as ix, ra};
use cq_lang_physical::{self as physical, policy::Policy};
use cq_physical_lowering::{check, compile, erase};
#[allow(dead_code)]
#[path = "../../../test-support/relational/cases.rs"]
mod cases;
#[allow(dead_code)]
#[path = "../../../test-support/relational/semantics.rs"]
mod oracle;
#[path = "../../../test-support/relational/scenarios.rs"]
mod scenarios;
#[allow(dead_code)]
#[path = "../../../test-support/relational/values.rs"]
mod values;
#[test]
fn scenarios_preserve_relational_answers_under_probe_and_merge() {
    for case in scenarios::SCENARIOS {
        let source: ix::Program = syn::parse_str(&case.ix()).unwrap();
        for merge in [false, true] {
            let policy_text = if merge {
                case.policy.replacen("}", "join = merge; }", 1)
            } else {
                case.policy.into()
            };
            let policy: Policy = syn::parse_str(&policy_text).unwrap();
            let target = compile(&source, &policy).unwrap();
            check(&source, &target, &policy).unwrap();
            if !merge {
                assert_eq!(
                    target,
                    syn::parse_str::<physical::Program>(case.physical).unwrap(),
                    "{}",
                    case.name
                );
            }
            assert_eq!(erase(&target), source.ra);
            let mut changed = target.clone();
            changed.program.head.relation = syn::parse_quote!(wrong);
            assert!(check(&source, &changed, &policy).is_err());
            if let Some(data) = cases::CASES.iter().find(|c| c.name == case.name) {
                let database = data
                    .inputs
                    .iter()
                    .map(|(n, rows)| ((*n).into(), rows.iter().map(|r| r.to_vec()).collect()))
                    .collect();
                let expected = data
                    .expected
                    .iter()
                    .map(|r| r.to_vec())
                    .collect::<values::Rows>();
                assert_eq!(
                    oracle::evaluate_ra(&erase(&target), &database).output,
                    expected,
                    "{}",
                    case.name
                );
            }
        }
    }
}
#[test]
fn demands_are_a_pass_obligation_and_policy_is_checked() {
    let case = scenarios::SCENARIOS
        .iter()
        .find(|c| c.name == "triangle")
        .unwrap();
    let source: ix::Program = syn::parse_str(&case.ix()).unwrap();
    let policy = syn::parse_str(case.policy).unwrap();
    let target = compile(&source, &policy).unwrap();
    let mut missing = source.clone();
    missing.indexes.clear();
    ix::contract::check(&missing).unwrap();
    physical::contract::check(&target).unwrap();
    assert!(check(&missing, &target, &policy).is_err());
    assert!(compile(&missing, &policy).is_err());
    let changed: Policy = syn::parse_str("policy {    join = merge; }").unwrap();
    assert!(check(&source, &target, &changed).is_err());
}

fn lower(source: &str, policy: &str) -> (ix::Program, physical::Program, Policy) {
    let source = syn::parse_str(source).unwrap();
    let policy = syn::parse_str(policy).unwrap();
    let target = compile(&source, &policy).unwrap();
    check(&source, &target, &policy).unwrap();
    (source, target, policy)
}

#[test]
fn layout_bounds_and_plan_uniqueness_are_independent_obligations() {
    let (source, target, policy) = lower(
        "relational {
        relation R(a:i32,b:i32) = (rows()); r = rename R {a -> x,b -> y};
        s = select r if (x >= 2 && y != 3); output s as answer(x,y);
    } indexes {}",
        "policy {}",
    );
    assert_eq!(target.program.definitions[0].mode, syn::parse_quote!(pipe));
    assert!(matches!(
        target.program.definitions[1].operator,
        physical::Operator::Range(..)
    ));
    assert!(target.program.definitions[1].canon.is_some());
    for replacement in [
        syn::parse_quote!(s = pipe range[y] r if (x >= 2 && y != 3);),
        syn::parse_quote!(s = pipe range[x] r if (x != 2);),
        syn::parse_quote!(s = pipe range[x] r if (x >= 2 && y != 3);),
    ] {
        let mut changed = target.clone();
        changed.program.definitions[1] = replacement;
        assert!(check(&source, &changed, &policy).is_err());
    }
    // A select is admissible even when the optimizer chooses a range.
    let mut select = target.clone();
    select.program.definitions[1] =
        syn::parse_quote!(s = pipe canon select r if (x >= 2 && y != 3););
    check(&source, &select, &policy).unwrap();
    let mut retained = target;
    retained.program.definitions[0].mode = syn::parse_quote!(let);
    check(&source, &retained, &policy).unwrap();
}

#[test]
fn nonleading_unbounded_and_derived_selects_stay_selects() {
    for condition in ["y >= 2", "x != 2", "x < y", "x > 2147483647"] {
        let (_, target, _) = lower(
            &format!(
                "relational {{ relation R(a:i32,b:i32); r = rename R {{a -> x,b -> y}}; s = select r if ({condition}); output s as answer(x,y); }} indexes {{}}"
            ),
            "policy {}",
        );
        assert!(matches!(
            target.program.definitions[1].operator,
            physical::Operator::Select(_)
        ));
    }
}

#[test]
fn join_policy_and_canonical_projection_mutations_are_rejected() {
    let source = "relational { relation L(a:i32); relation R(a:i32,b:i32);
        l = rename L {a -> a}; r = rename R {a -> a,b -> b}; j = natural_join l with r;
        p = project j keep {a}; e = extend p with {n:i32} value (1); output e as answer(a,n);
    } indexes { i = @r[0]; }";
    for policy in ["policy {}", "policy {join = merge; project = canon;}"] {
        let (source, target, policy) = lower(source, policy);
        let mut changed = target.clone();
        let op = &mut changed.program.definitions[2].operator;
        *op = match op.clone() {
            physical::Operator::Probe(_, k) => physical::Operator::Merge(Default::default(), k),
            physical::Operator::Merge(_, k) => physical::Operator::Probe(Default::default(), k),
            _ => unreachable!(),
        };
        assert!(check(&source, &changed, &policy).is_err());
        let mut changed = target.clone();
        changed.program.definitions[0].canon = Some(Default::default());
        assert!(check(&source, &changed, &policy).is_err());
        let mut changed = target;
        changed.program.definitions[3].canon = None;
        changed.program.definitions[4].canon = None;
        assert!(check(&source, &changed, &policy).is_err());
    }
}

#[test]
fn range_checker_rejects_unbounded_authored_range_without_replaying_compile() {
    let source: ix::Program = syn::parse_quote! { relational { relation R(a:i32); r = rename R {a -> a}; s = select r if (a != 1); output s as answer(a); } indexes {} };
    let target: physical::Program = syn::parse_quote! { physical { relation R(a:i32); r = pipe rename R {a -> a}; s = pipe canon range[a] r if (a != 1); return s as answer(a); } };
    assert!(
        check(&source, &target, &syn::parse_quote!(policy {}))
            .unwrap_err()
            .to_string()
            .contains("bound")
    );
}
