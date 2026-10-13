use cq_lang_ix as ix;
use cq_lang_physical as physical;
use cq_physical_lowering::{check, compile, erase};
use cq_testkit::{cases, corpus as scenarios, rewrites as authored, semantics as oracle, values};
#[test]
fn scenarios_preserve_relational_answers_under_probe_and_merge() {
    for case in scenarios::SCENARIOS {
        let source = case.parse_ix().unwrap();
        for merge in [false, true] {
            let mut target = compile(&source).unwrap();
            if merge {
                target = authored::merged(&target).unwrap();
            }
            check(&source, &target).unwrap();
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
            assert!(check(&source, &changed).is_err());
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
fn demands_are_a_pass_obligation() {
    let case = scenarios::SCENARIOS
        .iter()
        .find(|c| c.name == "triangle")
        .unwrap();
    let source = case.parse_ix().unwrap();
    let target = compile(&source).unwrap();
    let mut missing = source.clone();
    missing.indexes.clear();
    ix::contract::check(&missing).unwrap();
    physical::contract::check(&target).unwrap();
    assert!(check(&missing, &target).is_err());
    assert!(compile(&missing).is_err());
}

fn lower(source: &str) -> (ix::Program, physical::Program) {
    let source = syn::parse_str(source).unwrap();
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    (source, target)
}

#[test]
fn const_demand_and_point_modifier_are_independent_obligations() {
    let (source, target) = lower(
        "relational { relation R(a:i32,b:i32); r = rename R@0 {a -> x,b -> y};
        s = select r if (x = 2); output s as answer(x,y);
        } indexes {i = @r[0] const;}",
    );
    assert!(matches!(
        target.program.definitions[1].operator,
        physical::Operator::Point(..)
    ));
    let mut changed = target.clone();
    changed.program.definitions[1] = syn::parse_quote!(s = pipe canon select r if (x = 2););
    check(&source, &changed).unwrap();
    let mut missing = source.clone();
    missing.indexes.clear();
    assert!(
        check(&missing, &target)
            .unwrap_err()
            .to_string()
            .contains("demand")
    );
    let mut retained = target;
    retained.program.definitions[0].mode = syn::parse_quote!(let);
    check(&source, &retained).unwrap();
}

#[test]
fn relational_column_equality_is_a_select_and_rust_is_a_guard() {
    let (_, target) = lower(
        "relational {relation R(a:i32,b:i32); r = rename R@0 {a -> x,b -> y};
        s = select r if (x = y); g = guard s if (*x > 2); output g as answer(x,y);
        } indexes {}",
    );
    assert!(matches!(
        target.program.definitions[1].operator,
        physical::Operator::Select(_)
    ));
    assert!(matches!(
        target.program.definitions[2].operator,
        physical::Operator::Guard(_)
    ));
}

#[test]
fn admissible_join_and_canonicalization_rewrites_are_accepted() {
    let source = "relational { relation L(a:i32); relation R(a:i32,b:i32);
        l = rename L@0 {a -> a}; r = rename R@1 {a -> a,b -> b}; j = natural_join l with r;
        p = project j keep {a}; e = bind p with n:i32 = (1); output e as answer(a,n);
    } indexes { i = @r[0] stream; }";
    let (source, target) = lower(source);
    let merged = authored::merged(&target).unwrap();
    check(&source, &merged).unwrap();
    let canon = authored::canonical_projects(&target).unwrap();
    check(&source, &canon).unwrap();
    let mut changed = target.clone();
    changed.program.definitions[0].canon = Some(Default::default());
    check(&source, &changed).unwrap();
    let mut changed = target;
    changed.program.definitions[3].canon = None;
    changed.program.definitions[4].canon = None;
    assert!(check(&source, &changed).is_err());
}

#[test]
fn chain_reordering_fails_erasure_even_if_local_program_is_scoped() {
    let (source, target) = lower(
        "relational {relation R(a:i32); r = rename R@0 {a -> x};
        g = guard r if (*x != 0); b = bind g with y:i32 = (10 / *x);
        output b as answer(x,y); } indexes {}",
    );
    let mut changed = target;
    changed.program.definitions[1] = syn::parse_quote!(b = pipe bind r with y:i32 = (10 / *x););
    changed.program.definitions[2] = syn::parse_quote!(g = pipe canon guard b if (*x != 0););
    changed.program.returned = syn::parse_quote!(g);
    physical::contract::check(&changed).unwrap();
    assert!(check(&source, &changed).is_err());
}

#[test]
fn static_orientation_builds_filtered_branch_and_streams_unfiltered_host() {
    let source: ix::ra::Program = syn::parse_quote! {
        relational {
            relation Orders(o:i32,c:i32); relation Lineitem(o:i32,s:i32);
            o = rename Orders@0 {o -> k,c -> c};
            f = select o if (c = 7);
            l = rename Lineitem@1 {o -> k,s -> s};
            j = natural_join f with l;
            output j as answer(k,c,s);
        }
    };
    let target = cq_physical_lowering::orient(&source).unwrap();
    cq_physical_lowering::check_orientation(&source, &target).unwrap();
    assert_eq!(
        target.plan.definitions[3],
        syn::parse_quote!(j = natural_join l with f;)
    );
    assert!(cq_physical_lowering::check_orientation(&source, &source).is_err());
    let mut wrong = target.clone();
    wrong.plan.definitions[1] = syn::parse_quote!(f = select o if (c = 8););
    assert!(cq_physical_lowering::check_orientation(&source, &wrong).is_err());
    let source_ix: ix::Program = syn::parse_quote!(#target indexes {
        i0 = @o[0] const; i1 = @f[1] stream;
    });
    // Sorted headings are c,k: point reads c, join reads k.
    let physical = compile(&source_ix).unwrap();
    check(&source_ix, &physical).unwrap();
    assert_eq!(erase(&physical), target);
}

#[test]
fn static_orientation_never_commutes_directional_membership() {
    let source: ix::ra::Program = syn::parse_quote! {
        relational {
            relation R(a:i32); relation S(a:i32);
            r = rename R@0 {a -> x}; f = select r if (x = 7);
            s = rename S@1 {a -> x}; j = semijoin f with s;
            output j as answer(x);
        }
    };
    assert_eq!(cq_physical_lowering::orient(&source).unwrap(), source);
}
