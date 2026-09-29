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
    for text in [
        "policy { R; S; }",
        "policy { R; S; T; R; }",
        "policy { R keyed [missing]; S; T; }",
    ] {
        assert!(compile(&source, &syn::parse_str(text).unwrap()).is_err());
    }
    let changed: Policy = syn::parse_str("policy { R; S; T; join = merge; }").unwrap();
    assert!(check(&source, &target, &changed).is_err());
}
#[test]
fn ordered_host_range_and_canon_follow_policy() {
    let source: ix::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32);
        r = rename R {a -> x,b -> y};
        s = select r if (x >= 2 && x < 9 && y != 5);
        p = project s keep {x}; output p as answer(x);
    } indexes {} };
    let policy: Policy = syn::parse_quote!(policy { R keyed [a] ordered canon; project = canon; });
    let target = compile(&source, &policy).unwrap();
    assert_body(
        &target,
        "
        r = alias rename R {a -> x, b -> y};
        s = pipe range[x] r if (x >= 2 && x < 9 && y != 5);
        p = pipe canon project s keep {x}; return p as answer(x);
    ",
    );
    check(&source, &target, &policy).unwrap();
    for condition in ["x != 2", "x < y", "x > 2147483647"] {
        let mut changed = source.clone();
        if let ra::Operator::Select(s) = &mut changed.ra.plan.definitions[1].operator {
            s.condition = syn::parse_str(condition).unwrap();
        }
        let result = compile(&changed, &policy).unwrap();
        assert_definition(
            &result,
            "s",
            &format!("s = pipe select r if ({condition});"),
        );
    }
}

const CHAIN: &str = "relational {
    relation L(a: i32);
    relation R(a: i32, b: i32);
    relation S(b: i32, c: i32);
    r0 = rename L {a -> a};
    r1 = rename R {a -> a, b -> b};
    r2 = natural_join r0 with r1;
    r3 = rename S {b -> b, c -> c};
    r4 = natural_join r2 with r3;
    r5 = project r4 keep {a, c};
    output r5 as answer(a, c);
} indexes { i0 = @r1[0]; i1 = @r3[0]; }";

fn lower(source: &ix::Program, policy: &str) -> physical::Program {
    let policy = syn::parse_str(policy).unwrap();
    let target = compile(source, &policy).unwrap();
    check(source, &target, &policy).unwrap();
    target
}

fn assert_body(target: &physical::Program, body: &str) {
    use quote::quote;
    let declarations = &target.program.declarations;
    let body: proc_macro2::TokenStream = body.parse().unwrap();
    let expected: physical::Program = syn::parse2(quote! {
        physical { #(#declarations)* #body }
    })
    .unwrap();
    assert_eq!(target, &expected);
}

fn assert_definition(target: &physical::Program, name: &str, expected: &str) {
    let actual = target
        .program
        .definitions
        .iter()
        .find(|d| d.name == name)
        .unwrap();
    assert_eq!(
        actual,
        &syn::parse_str::<physical::Definition>(expected).unwrap()
    );
}

#[test]
fn merge_materializes_changed_keys_and_uses_ordered_hosts_in_place() {
    let chain = syn::parse_str(CHAIN).unwrap();
    let target = lower(&chain, "policy { L; R; S; join = merge; }");
    assert_body(
        &target,
        "
        r0 = let rename L {a -> a};
        r1 = let rename R {a -> a, b -> b};
        r2 = let merge[a] r0, r1;
        r3 = let rename S {b -> b, c -> c};
        r4 = pipe merge[b] r2, r3;
        r5 = pipe canon project r4 keep {a, c};
        return r5 as answer(a, c);",
    );
    let star = syn::parse_str(
        &CHAIN
            .replace("S(b:", "S(a:")
            .replace("S {b -> b", "S {a -> a"),
    )
    .unwrap();
    for (left, mode) in [("L keyed [a] ordered", "alias"), ("L keyed [a]", "let")] {
        let target = lower(
            &star,
            &format!(
                "policy {{ {left}; R keyed [a] ordered; S keyed [a] ordered; join = merge; }}"
            ),
        );
        assert_body(
            &target,
            &format!(
                "
            r0 = {mode} rename L {{a -> a}};
            r1 = alias rename R {{a -> a, b -> b}};
            r2 = pipe merge[a] r0, r1;
            r3 = alias rename S {{a -> a, c -> c}};
            r4 = pipe merge[a] r2, r3;
            r5 = pipe canon project r4 keep {{a, c}};
            return r5 as answer(a, c);
        "
            ),
        );
    }
}

#[test]
fn keyed_hosts_alias_and_canon_hosts_avoid_only_redundant_return_canon() {
    let chain = syn::parse_str(CHAIN).unwrap();
    let target = lower(
        &chain,
        "policy { L canon; R keyed [a] canon; S keyed [b] canon; }",
    );
    assert_body(
        &target,
        "
        r0 = alias rename L {a -> a};
        r1 = alias rename R {a -> a, b -> b};
        r2 = pipe probe[a] r0, r1;
        r3 = alias rename S {b -> b, c -> c};
        r4 = pipe probe[b] r2, r3;
        r5 = pipe canon project r4 keep {a, c};
        return r5 as answer(a, c);",
    );
    let full: ix::Program = syn::parse_quote! { relational {
        relation L(a: i32); relation R(a: i32, b: i32);
        r0 = rename L {a -> a}; r1 = rename R {a -> a, b -> b};
        r2 = natural_join r0 with r1;
        r3 = project r2 keep {a, b}; output r3 as answer(a, b);
    } indexes { i0 = @r1[0]; }};
    for (host, canon) in [("R keyed [a] canon", ""), ("R keyed [a]", "canon")] {
        let target = lower(&full, &format!("policy {{ L canon; {host}; }}"));
        assert_body(
            &target,
            &format!(
                "
            r0 = alias rename L {{a -> a}};
            r1 = alias rename R {{a -> a, b -> b}};
            r2 = pipe probe[a] r0, r1;
            r3 = pipe {canon} project r2 keep {{a, b}};
            return r3 as answer(a, b);
        "
            ),
        );
    }
}

#[test]
fn fine_triangle_canon_policy_pins_intermediate_projection_and_mode() {
    let case = scenarios::SCENARIOS
        .iter()
        .find(|c| c.name == "fine_triangle")
        .unwrap();
    let source = syn::parse_str(&case.ix()).unwrap();
    let keep = lower(&source, "policy { R; S; T; project = keep; }");
    assert_eq!(lower(&source, case.policy), keep);
    assert_eq!(
        keep,
        syn::parse_str::<physical::Program>(case.physical).unwrap()
    );
    let policy: Policy = syn::parse_quote!(policy { R; S; T; project = canon; });
    let target = compile(&source, &policy).unwrap();
    // The entire program is pinned: no other operator or mode changes.
    let expected = case
        .physical
        .replace("r1 = pipe project", "r1 = pipe canon project");
    assert_eq!(
        target,
        syn::parse_str::<physical::Program>(&expected).unwrap()
    );
    reject(
        &source,
        &target,
        &syn::parse_quote!(policy { R; S; T; project = keep; }),
        &["r1", "canon"],
    );
    for change_mode in [false, true] {
        let mut changed = target.clone();
        if change_mode {
            changed.program.definitions[1].mode = syn::parse_quote!(let);
        } else {
            changed.program.definitions[1].canon = None;
        }
        physical::contract::check(&changed).unwrap();
        if change_mode {
            assert_ne!(changed, target);
            check(&source, &changed, &policy).unwrap();
        } else {
            let error = check(&source, &changed, &policy).unwrap_err().to_string();
            assert!(
                error.contains("r1") && error.contains("project=canon"),
                "{error}"
            );
        }
    }
}

#[test]
fn keyed_projection_is_a_canonical_let_even_with_canonical_hosts() {
    let source: ix::Program = syn::parse_quote! { relational {
        relation L(a: i32); relation R(a: i32, b: i32);
        l = rename L {a -> a}; r = rename R {a -> a, b -> b};
        p = project r keep {a}; j = natural_join l with p;
        output j as answer(a);
    } indexes { i0 = @p[0]; }};
    for policy in [
        "policy { L; R; project = canon; }",
        "policy { L canon; R keyed [a] canon; project = canon; }",
    ] {
        assert_body(
            &lower(&source, policy),
            "
            l = alias rename L {a -> a};
            r = alias rename R {a -> a, b -> b};
            p = let canon project r keep {a};
            j = pipe canon probe[a] l, p;
            return j as answer(a);",
        );
    }
}

#[test]
fn projection_canon_uses_the_inferred_key_after_extend() {
    for (host, columns, canon) in [
        ("R canon", "a", ""),
        ("R canon", "a, b", ""),
        ("R canon", "b", "canon"),
        ("R", "a, b", "canon"),
    ] {
        let source = syn::parse_str(&format!(
            "relational {{
            relation R(a: i32);
            r = rename R {{a -> a}};
            e = extend r with {{b: i32}} value ((0,));
            p = project e keep {{{columns}}};
            s = select p if (true); output s as answer({columns});
        }} indexes {{}}"
        ))
        .unwrap();
        let target = lower(&source, &format!("policy {{ {host}; project = canon; }}"));
        assert_definition(
            &target,
            "p",
            &format!("p = pipe {canon} project e keep {{{columns}}};"),
        );
        let kept = lower(&source, &format!("policy {{ {host}; project = keep; }}"));
        assert_definition(
            &kept,
            "p",
            &format!("p = pipe project e keep {{{columns}}};"),
        );
        assert_eq!(
            target.program.definitions.last(),
            kept.program.definitions.last()
        );
    }
}

#[test]
fn cartesian_join_is_a_unit_probe_under_either_join_policy() {
    let case = scenarios::SCENARIOS
        .iter()
        .find(|c| c.name == "cartesian")
        .unwrap();
    let source = syn::parse_str(&case.ix()).unwrap();
    for policy in [
        case.policy.to_string(),
        case.policy.replace('}', "join = merge; }"),
    ] {
        let target = lower(&source, &policy);
        assert_eq!(
            target,
            syn::parse_str::<physical::Program>(case.physical).unwrap()
        );
        assert_definition(&target, "r2", "r2 = pipe probe[] r0, r1;");
        assert_eq!(
            physical::contract::check(&target).unwrap().key_reads["r1"],
            [("r2".into(), vec![])]
        );
    }
}

#[test]
fn full_reduction_uses_membership_and_materializes_shared_results() {
    let source: ix::Program = syn::parse_quote! { relational {
        relation L(a: i32, b: i32); relation R(b: i32, c: i32);
        r0 = rename R {b -> b, c -> c}; r1 = rename L {a -> a, b -> b};
        r2 = semijoin r0 with r1; r3 = semijoin r1 with r2;
        r4 = natural_join r3 with r2;
        r5 = project r4 keep {a}; output r5 as answer(a);
    } indexes { i0 = @r1[1]; i1 = @r2[0]; }};
    let target = lower(&source, "policy { L keyed [b]; R; }");
    assert_body(
        &target,
        "
        r0 = alias rename R {b -> b, c -> c};
        r1 = alias rename L {a -> a, b -> b};
        r2 = let semi[b] r0, r1;
        r3 = pipe semi[b] r1, r2;
        r4 = pipe probe[b] r3, r2;
        r5 = pipe canon project r4 keep {a}; return r5 as answer(a);",
    );
    let analysis = physical::contract::check(&target).unwrap();
    assert_eq!(analysis.key_reads["r1"], [("r2".into(), vec!["b".into()])]);
    assert_eq!(
        analysis.key_reads["r2"],
        [
            ("r3".into(), vec!["b".into()]),
            ("r4".into(), vec!["b".into()])
        ]
    );
    let target = lower(&source, "policy { L canon; R canon; }");
    assert_definition(&target, "r1", "r1 = let rename L {a -> a, b -> b};");
    assert_definition(&target, "r5", "r5 = pipe canon project r4 keep {a};");
}

#[test]
fn range_requires_a_bounded_ordered_host_key_and_preserves_non_host_selects() {
    for (key, condition, operator) in [
        ("[a] ordered", "a >= 2 && 9 > a && b != 5", "range[a]"),
        ("[a]", "a >= 2 && 9 > a && b != 5", "select"),
        ("[b] ordered", "a >= 2 && 9 > a && b != 5", "select"),
        ("[a] ordered", "a != 3 && b < 4", "select"),
        ("[a] ordered", "b < 4 && 7 <= a", "range[a]"),
        ("[a] ordered", "a > 2147483647", "select"),
    ] {
        let source = syn::parse_str(&format!(
            "relational {{
            relation R(a: i32, b: i32);
            r0 = rename R {{a -> a, b -> b}};
            r1 = select r0 if ({condition});
            r2 = select r1 if (b >= 0);
            r3 = project r2 keep {{a, b}}; output r3 as answer(a, b);
        }} indexes {{}}"
        ))
        .unwrap();
        let target = lower(&source, &format!("policy {{ R keyed {key}; }}"));
        assert_body(
            &target,
            &format!(
                "
            r0 = alias rename R {{a -> a, b -> b}};
            r1 = pipe {operator} r0 if ({condition});
            r2 = pipe select r1 if (b >= 0);
            r3 = pipe canon project r2 keep {{a, b}}; return r3 as answer(a, b);
        "
            ),
        );
        let analysis = physical::contract::check(&target).unwrap();
        if operator == "range[a]" {
            assert_eq!(analysis.range_reads["r0"], [("r1".into(), "a".into())]);
        } else {
            assert!(analysis.range_reads.is_empty());
        }
    }
}

#[test]
fn reduced_chain_satisfies_the_contract_under_mixed_policies() {
    let source: ix::Program = syn::parse_quote! { relational {
        relation L(a: i32); relation R(a: i32, b: i32); relation S(b: i32, c: i32);
        r0 = rename R {a -> a, b -> b}; r1 = rename L {a -> a};
        r2 = semijoin r0 with r1; r3 = rename S {b -> b, c -> c};
        r4 = semijoin r3 with r2; r5 = semijoin r2 with r4;
        r6 = semijoin r1 with r5; r7 = natural_join r6 with r5;
        r8 = natural_join r7 with r4; r9 = project r8 keep {a, c};
        output r9 as answer(a, c);
    } indexes { i0 = @r1[0]; i1 = @r2[1]; i2 = @r4[0]; i3 = @r5[0]; }};
    for (policy, host_mode, join_mode, join) in [
        ("policy { L; R; S; }", "let", "pipe", "probe"),
        ("policy { L; R; S; join = merge; }", "let", "let", "merge"),
        (
            "policy { L keyed [a] ordered canon; R keyed [a]; S keyed [b] ordered; }",
            "alias",
            "pipe",
            "probe",
        ),
        (
            "policy { L keyed [a] ordered canon; R keyed [b] ordered canon; S keyed [b] ordered canon; join = merge; semijoin = prefer_lazy; }",
            "alias",
            "let",
            "merge",
        ),
    ] {
        assert_body(
            &lower(&source, policy),
            &format!(
                "
            r0 = alias rename R {{a -> a, b -> b}};
            r1 = {host_mode} rename L {{a -> a}};
            r2 = let semi[a] r0, r1;
            r3 = alias rename S {{b -> b, c -> c}};
            r4 = let semi[b] r3, r2;
            r5 = let semi[b] r2, r4;
            r6 = {join_mode} semi[a] r1, r5;
            r7 = {join_mode} {join}[a] r6, r5;
            r8 = pipe {join}[b] r7, r4;
            r9 = pipe canon project r8 keep {{a, c}}; return r9 as answer(a, c);
        "
            ),
        );
    }
}

#[test]
fn missing_demands_name_the_reader_and_arbitrary_demand_names_are_accepted() {
    let mut source: ix::Program = syn::parse_str(CHAIN).unwrap();
    source.indexes.pop();
    let error = compile(&source, &syn::parse_quote!(policy { L; R; S; }))
        .unwrap_err()
        .to_string();
    for name in ["`r4`", "`@r3[b]`"] {
        assert!(error.contains(name), "{error}");
    }
    let mut source: ix::Program = syn::parse_quote! { relational {
        relation L(a: i32); relation R(a: i32);
        l = rename L {a -> a}; r = rename R {a -> a};
        j = semijoin l with r; output j as answer(a);
    } indexes {} };
    let error = compile(&source, &syn::parse_quote!(policy { L; R; }))
        .unwrap_err()
        .to_string();
    assert!(error.contains("`j`"), "{error}");
    source.indexes = syn::parse_quote!(arbitrary_name = @r[0];);
    assert_body(
        &lower(&source, "policy { L; R; }"),
        "
        l = alias rename L {a -> a}; r = let rename R {a -> a};
        j = pipe canon semi[a] l, r; return j as answer(a);",
    );
    let case = scenarios::SCENARIOS
        .iter()
        .find(|c| c.name == "triangle")
        .unwrap();
    let error = compile(
        &syn::parse_str(&case.ix()).unwrap(),
        &syn::parse_quote!(policy { R; S; T; Extra; }),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("undeclared"), "{error}");
}

fn reject(source: &ix::Program, target: &physical::Program, policy: &Policy, names: &[&str]) {
    let error = check(source, target, policy).unwrap_err().to_string();
    for name in names {
        assert!(error.contains(name), "{error}");
    }
}

#[test]
fn independent_lowering_diagnoses_contracts_erasure_demands_and_hosts() {
    let source: ix::Program = syn::parse_str(CHAIN).unwrap();
    let policy: Policy = syn::parse_quote!(policy { L; R; S; });
    let target = compile(&source, &policy).unwrap();
    let mut bad_source = source.clone();
    bad_source.indexes[0].requirement.relation = syn::parse_quote!(absent);
    reject(&bad_source, &target, &policy, &["absent"]);
    let mut invalid = target.clone();
    invalid.program.definitions[2] = syn::parse_quote!(r2 = pipe probe[b] r0,r1;);
    reject(&source, &invalid, &policy, &["r2", "join columns"]);
    let mut changed = target.clone();
    changed.program.definitions[1] = syn::parse_quote!(r1 = let rename R {b -> b, a -> a};);
    physical::contract::check(&changed).unwrap();
    reject(&source, &changed, &policy, &["r1", "erasure"]);
    let mut missing = source.clone();
    missing.indexes.pop();
    reject(&missing, &target, &policy, &["r4", "demand"]);
    for (text, names) in [
        ("policy { L; R; }", &["S", "missing"][..]),
        ("policy { L; R; S; Extra; }", &["Extra", "undeclared"]),
        ("policy { L; R; S; R; }", &["R", "duplicate"]),
        ("policy { L; R keyed [missing]; S; }", &["R", "missing"]),
        ("policy { L; R keyed [a,a]; S; }", &["R", "duplicate"]),
    ] {
        reject(&source, &target, &syn::parse_str(text).unwrap(), names);
    }
}

#[test]
fn independent_lowering_checks_join_semijoin_alias_and_canonical_policy() {
    let source: ix::Program = syn::parse_str(CHAIN).unwrap();
    let policy: Policy = syn::parse_quote!(policy { L; R; S; });
    let target = compile(&source, &policy).unwrap();
    let mut changed = target.clone();
    changed.program.definitions[0].mode = syn::parse_quote!(let);
    changed.program.definitions[2] = syn::parse_quote!(r2 = pipe merge[a] r0,r1;);
    reject(&source, &changed, &policy, &["r2", "requires probe"]);
    let mut materialized = target.clone();
    materialized.program.definitions[0].mode = syn::parse_quote!(let);
    reject(
        &source,
        &materialized,
        &syn::parse_quote!(policy { L; R; S; join = merge; }),
        &["r2", "requires merge"],
    );
    let mut changed = target.clone();
    changed.program.definitions[1].mode = syn::parse_quote!(alias);
    reject(&source, &changed, &policy, &["r1", "alias reads", "R"]);
    let mut changed = target.clone();
    changed.program.definitions[2].mode = syn::parse_quote!(alias);
    reject(&source, &changed, &policy, &["r2", "must rename"]);
    let mut changed = target.clone();
    changed.program.definitions[0].canon = Some(Default::default());
    reject(&source, &changed, &policy, &["r0", "canon"]);
    let mut changed = target.clone();
    changed.program.definitions[5].canon = None;
    reject(&source, &changed, &policy, &["r5", "not canonical"]);

    let semi: ix::Program = syn::parse_quote! { relational {
        relation L(a:i32); relation R(a:i32);
        l = rename L {a -> a}; r = rename R {a -> a};
        s = semijoin l with r; output s as answer(a);
    } indexes { i0 = @r[0]; } };
    let p: Policy = syn::parse_quote!(policy { L; R; });
    let mut changed = compile(&semi, &p).unwrap();
    changed.program.definitions[2] = syn::parse_quote!(s = pipe canon probe[a] l,r;);
    physical::contract::check(&changed).unwrap();
    reject(&semi, &changed, &p, &["s", "erasure"]);
}

#[test]
fn range_obligations_are_independent_of_the_select_lowerer() {
    let source: ix::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R {a -> x,b -> y};
        s = select r if (x >= 2); output s as answer(x,y);
    } indexes {} };
    let ordered: Policy = syn::parse_quote!(policy { R keyed [a] ordered; });
    let target = compile(&source, &ordered).unwrap();
    // Let is always admissible here, independently of alias selection.
    let mut materialized = target.clone();
    materialized.program.definitions[0].mode = syn::parse_quote!(let);
    check(&source, &materialized, &ordered).unwrap();
    assert_ne!(materialized, target);
    for policy in [
        syn::parse_quote!(policy { R keyed [a]; }),
        syn::parse_quote!(policy { R keyed [b] ordered; }),
    ] {
        reject(&source, &materialized, &policy, &["s", "ordered host key"]);
    }
    for condition in ["x != 2", "x < y", "x > 2147483647", "x < -2147483648"] {
        let mut s = source.clone();
        let ra::Operator::Select(select) = &mut s.ra.plan.definitions[1].operator else {
            unreachable!()
        };
        select.condition = syn::parse_str(condition).unwrap();
        let mut t = materialized.clone();
        let physical::Operator::Range(_, range) = &mut t.program.definitions[1].operator else {
            unreachable!()
        };
        range.condition = syn::parse_str(condition).unwrap();
        reject(&s, &t, &ordered, &["s", "does not bound", "x"]);
    }
    // The policy permits select even when range is available.
    let alternative: physical::Program = syn::parse_quote! { physical {
        relation R(a:i32,b:i32); r = let rename R {a -> x,b -> y};
        s = pipe canon select r if (x >= 2); return s as answer(x,y);
    }};
    check(&source, &alternative, &ordered).unwrap();
    assert_ne!(target, alternative);
}

#[test]
fn hand_written_alias_to_let_alternative_is_accepted() {
    let source: ix::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R {a -> a}; output r as answer(a);
    } indexes {} };
    let policy: Policy = syn::parse_quote!(policy { R canon; });
    let alternative: physical::Program = syn::parse_quote! { physical {
        relation R(a:i32); r = let rename R {a -> a}; return r as answer(a);
    }};
    check(&source, &alternative, &policy).unwrap();
    assert_ne!(compile(&source, &policy).unwrap(), alternative);
}

#[test]
fn cartesian_merge_is_rejected_under_both_policies() {
    let source: ix::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(b:i32);
        r = rename R {a -> a}; s = rename S {b -> b};
        j = natural_join r with s; output j as answer(a,b);
    } indexes { i0 = @s[]; } };
    let target: physical::Program = syn::parse_quote! { physical {
        relation R(a:i32); relation S(b:i32);
        r = let rename R {a -> a}; s = let rename S {b -> b};
        j = pipe canon merge[] r,s; return j as answer(a,b);
    }};
    physical::contract::check(&target).unwrap();
    for policy in [
        syn::parse_quote!(policy { R; S; }),
        syn::parse_quote!(policy { R; S; join = merge; }),
    ] {
        reject(&source, &target, &policy, &["j", "Cartesian", "probe"]);
    }
}
