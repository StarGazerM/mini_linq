use cq_elaborate::{check, compile};
use cq_lang_vo::cq;
use quote::ToTokens;

fn plan(text: &str) -> (cq_lang_vo::Program, cq_lang_ra::Program) {
    let source: cq::Program = syn::parse_str(text).unwrap();
    let normal = cq::normal::normalize(&source).unwrap();
    let vo = cq_order::compile(&normal, &Default::default()).unwrap();
    let ra = compile(&vo).unwrap();
    check(&vo, &ra).unwrap();
    (vo, ra)
}

#[test]
fn nested_blocks_keep_paths_and_project_local_witnesses() {
    let (vo, ra) = plan(
        "struct Q; relation R(x:i32,y:i32); relation S(x:i32,y:i32); answer(x) :- { R(x,a), { S(x,b) } | { R(x,b) } } | { S(x,c) };",
    );
    let text = vo.to_token_stream().to_string();
    assert!(text.replace(' ', "").contains("0.0.1.0.0"), "{text}");
    let parsed: cq_lang_ra::Program = syn::parse2(ra.to_token_stream()).unwrap();
    cq_lang_ra::contract::check(&parsed).unwrap();
    assert_eq!(ra, parsed);
}

#[test]
fn fallback_records_anti_and_rust_branch_tree() {
    let (_, ra) = plan(
        "struct Q; relation R(x:i32); relation S(x:i32); answer(x) :- R(x), { if accept(x) } else { S(x), if other(x) }, if suffix(x);",
    );
    assert!(
        ra.plan
            .definitions
            .iter()
            .any(|d| matches!(d.operator, cq_lang_ra::Operator::Anti(_)))
    );
}

#[test]
fn rejects_local_escape_and_incompatible_interface_types() {
    for text in [
        "struct Q; relation R(x:i32); answer(y) :- { R(x), let y:i32 = *x } | { R(x) };",
        "struct Q; relation R(x:i32); relation S(y:i32); answer(x) :- {R(x), S(y)} | {R(x)}, S(y);",
        "struct Q; relation R(x:i32); relation S(x:i64); answer(x) :- { R(x) } | { S(x) };",
    ] {
        let source: cq::Program = syn::parse_str(text).unwrap();
        assert!(cq::contract::check(&source).is_err());
    }
}

#[test]
fn rejects_malformed_blocks() {
    for body in [
        "{} | { R(x) }",
        "{ R(x) }",
        "{ R(x) } | { R(x) } else { R(x) }",
    ] {
        assert!(
            syn::parse_str::<cq::Program>(&format!(
                "struct Q; relation R(x:i32); answer(x) :- {body};"
            ))
            .is_err()
        );
    }
}

#[test]
fn every_ordering_preserves_pure_and_rust_block_interfaces() {
    for text in [
        "struct Q; relation R(x:i32,y:i32); relation S(y:i32,z:i32); relation T(y:i32,z:i32); relation U(z:i32,w:i32); answer(x,w) :- R(x,y), { S(y,z) } | { T(y,z) }, U(z,w);",
        "struct Q; relation R(x:i32,y:i32); relation S(y:i32,z:i32); relation T(y:i32,z:i32); relation U(z:i32,w:i32); answer(x,w) :- R(x,y), { S(y,z), if good(z) } else { T(y,z), if other(z) }, U(z,w);",
    ] {
        let source: cq::Program = syn::parse_str(text).unwrap();
        let normal = cq::normal::normalize(&source).unwrap();
        let written = cq_order::compile(&normal, &Default::default()).unwrap();
        for heuristic in [
            cq_order::Heuristic::Written,
            cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
            cq_order::Heuristic::Prefix(cq_order::Ranking::GreedyWidth),
            cq_order::Heuristic::Elimination,
        ] {
            for split in [cq_order::Split::Coarse, cq_order::Split::Fine] {
                let options = cq_order::Options { heuristic, split };
                let ordered = cq_order::compile(&normal, &options)
                    .unwrap_or_else(|e| panic!("{options:?}: {e}"));
                cq_order::check(&normal, &ordered, &options)
                    .unwrap_or_else(|e| panic!("{options:?}: {e}"));
                let ra = compile(&ordered).unwrap_or_else(|e| panic!("{options:?}: {e}"));
                check(&ordered, &ra).unwrap_or_else(|e| panic!("supplied {options:?}: {e}"));
                check(&written, &ra).unwrap_or_else(|e| panic!("written {options:?}: {e}"));
            }
        }
    }
}

#[test]
fn source_target_contract_rejects_branch_and_negative_mutations() {
    let (vo, ra) = plan(
        "struct Q; relation R(x:i32); relation S(x:i32); answer(x) :- { R(x) } else { S(x) };",
    );
    let mut changed = ra.clone();
    let union = changed
        .plan
        .definitions
        .iter_mut()
        .find_map(|d| match &mut d.operator {
            cq_lang_ra::Operator::Union(u) => Some(u),
            _ => None,
        })
        .unwrap();
    union.operands.pop();
    assert!(check(&vo, &changed).is_err());
    let mut changed = ra.clone();
    let union = changed
        .plan
        .definitions
        .iter_mut()
        .find_map(|d| match &mut d.operator {
            cq_lang_ra::Operator::Union(u) => Some(u),
            _ => None,
        })
        .unwrap();
    let first = union.operands[0].clone();
    union.operands[0] = union.operands[1].clone();
    union.operands[1] = first;
    assert!(check(&vo, &changed).is_err());
    let mut changed = ra.clone();
    let anti = changed
        .plan
        .definitions
        .iter_mut()
        .find_map(|d| match &mut d.operator {
            cq_lang_ra::Operator::Anti(a) => Some(a),
            _ => None,
        })
        .unwrap();
    anti.columns.push(syn::parse_quote!(x));
    assert!(check(&vo, &changed).is_err());
}

#[test]
fn negative_operands_reject_an_added_selection_by_reference() {
    let (vo, ra) =
        plan("struct Q; relation R(x:i32); relation S(x:i32); answer(x) :- {R(x)} else {S(x)};");
    let mut changed = ra.clone();
    let position = changed
        .plan
        .definitions
        .iter()
        .position(|d| matches!(d.operator, cq_lang_ra::Operator::Anti(_)))
        .unwrap();
    let cq_lang_ra::Operator::Anti(anti) = &mut changed.plan.definitions[position].operator else {
        unreachable!()
    };
    let original = anti.right.clone();
    anti.right = syn::parse_quote!(modified_negative);
    let select: cq_lang_ra::Definition =
        syn::parse_quote!(modified_negative = select #original if (x = 1););
    changed.plan.definitions.insert(position, select);
    cq_lang_ra::contract::check(&changed).unwrap();
    assert!(check(&vo, &changed).is_err());
}

#[test]
fn suffix_rust_cannot_move_into_just_one_arm() {
    let (vo, ra) = plan(
        "struct Q; relation R(x:i32); relation S(x:i32); answer(x) :- {R(x)} | {S(x)}, if suffix(x);",
    );
    let mut changed = ra.clone();
    let guard_position = changed
        .plan
        .definitions
        .iter()
        .position(|d| matches!(d.operator, cq_lang_ra::Operator::Guard(_)))
        .unwrap();
    let mut guard = changed.plan.definitions.remove(guard_position);
    let union_position = changed
        .plan
        .definitions
        .iter()
        .position(|d| matches!(d.operator, cq_lang_ra::Operator::Union(_)))
        .unwrap();
    let cq_lang_ra::Operator::Union(union) = &mut changed.plan.definitions[union_position].operator
    else {
        unreachable!()
    };
    let cq_lang_ra::Operator::Guard(g) = &mut guard.operator else {
        unreachable!()
    };
    g.input = union.operands[0].clone();
    union.operands[0] = guard.result.clone();
    let union_result = changed.plan.definitions[union_position].result.clone();
    for definition in &mut changed.plan.definitions {
        if let cq_lang_ra::Operator::Project(p) = &mut definition.operator {
            if p.input == guard.result {
                p.input = union_result.clone();
            }
        }
    }
    changed.plan.definitions.insert(union_position, guard);
    assert!(check(&vo, &changed).is_err());
}

#[test]
fn source_rewrite_cannot_swap_a_prefix_rust_step_with_a_rust_block() {
    let source: cq::Program = syn::parse_quote! { struct Q; relation R(x:i32); answer(x) :- R(x), if prefix(x), {if arm(x)} | {if other(x)}; };
    let mut target = source.clone();
    let mut items: Vec<_> = target.query.body.iter().cloned().collect();
    items.swap(1, 2);
    target.query.body = items.into_iter().collect();
    assert!(cq::check_rewrite(&source, &target).is_err());
}

#[test]
fn identical_rust_payloads_still_have_distinct_sibling_paths() {
    let (vo, ra) =
        plan("struct Q; relation R(x:i32); answer(x) :- R(x), {if arm(x)} | {if arm(x)};");
    let mut changed = ra.clone();
    let position = changed
        .plan
        .definitions
        .iter()
        .position(|d| matches!(d.operator, cq_lang_ra::Operator::Union(_)))
        .unwrap();
    let cq_lang_ra::Operator::Union(union) = &mut changed.plan.definitions[position].operator
    else {
        unreachable!()
    };
    let first = union.operands[0].clone();
    union.operands[1] = syn::parse_quote!(alias_first_arm);
    let alias = syn::parse_quote!(alias_first_arm = project #first keep {x};);
    changed.plan.definitions.insert(position, alias);
    assert!(check(&vo, &changed).is_err());
}

#[test]
fn positive_distribution_can_reference_prefix_rust_but_negative_cuts_cannot() {
    for operator in ["|", "else"] {
        let (vo, ra) = plan(&format!(
            "struct Q; relation R(x:i32); relation S(x:i32,y:i32); relation T(x:i32,y:i32); answer(x,y) :- R(x), if prefix(x), {{S(x,y)}} {operator} {{T(x,y)}};"
        ));
        let mut target = ra.clone();
        let context = target
            .plan
            .definitions
            .iter()
            .find(|d| matches!(d.operator, cq_lang_ra::Operator::Guard(_)))
            .unwrap()
            .result
            .clone();
        let position = target
            .plan
            .definitions
            .iter()
            .position(|d| matches!(d.operator, cq_lang_ra::Operator::Union(_)))
            .unwrap();
        let cq_lang_ra::Operator::Union(union) = &mut target.plan.definitions[position].operator
        else {
            unreachable!()
        };
        let first = union.operands[0].clone();
        union.operands[0] = syn::parse_quote!(positive_cut);
        let cut = syn::parse_quote!(positive_cut = semijoin #first with #context;);
        target.plan.definitions.insert(position, cut);
        check(&vo, &target).unwrap_or_else(|e| panic!("positive {operator}: {e}"));
        if operator == "else" {
            let mut negative = ra.clone();
            let original_position = negative
                .plan
                .definitions
                .iter()
                .position(|d| d.result == first)
                .unwrap();
            let cq_lang_ra::Operator::Project(project) =
                &mut negative.plan.definitions[original_position].operator
            else {
                unreachable!()
            };
            let input = project.input.clone();
            project.input = syn::parse_quote!(negative_cut);
            let cut = syn::parse_quote!(negative_cut = semijoin #input with #context;);
            negative.plan.definitions.insert(original_position, cut);
            cq_lang_ra::contract::check(&negative).unwrap();
            assert!(check(&vo, &negative).is_err());
        }
    }
}

#[test]
fn nested_and_sibling_scopes_keep_their_own_relational_predicates() {
    for choice in ["|", "else"] {
        let text = format!(
            "struct Q; relation R(x:i32); relation S(x:i32); answer(x) :- \
             R(x), if *x >= 1, \
             {{ S(x), if *x <= 2, {{ R(x), if *x != 0 }} | {{ S(x), if *x >= 2 }} }} \
             {choice} {{ S(x), if *x <= 3 }}, if *x <= 4;"
        );
        let (vo, target) = plan(&text);
        let selects: Vec<_> = target
            .plan
            .definitions
            .iter()
            .filter_map(|d| {
                if let cq_lang_ra::Operator::Select(s) = &d.operator {
                    Some(s)
                } else {
                    None
                }
            })
            .collect();
        // Both outer predicates belong to R, and each inner predicate belongs
        // only to the atom in its own sibling/nested body.
        assert_eq!(
            selects
                .iter()
                .map(|s| s.condition.constraints.len())
                .sum::<usize>(),
            6
        );
        for select in selects {
            let input = target
                .plan
                .definitions
                .iter()
                .find(|d| d.result == select.input)
                .unwrap();
            assert!(
                matches!(input.operator, cq_lang_ra::Operator::Rename(_)),
                "{}",
                target.to_token_stream()
            );
        }
        let reference = cq_elaborate::compile_written(&vo).unwrap();
        check(&vo, &reference).unwrap();
        for seed in 0..8 {
            let database = cq_testkit::databases::binary_database(&vo.cq, seed);
            assert_eq!(
                cq_testkit::semantics::evaluate_ra(&target, &database).output,
                cq_testkit::cq_semantics::evaluate_cq(&vo.cq, &database)
            );
        }
    }
}

#[test]
fn suffix_constraints_preserve_finite_branch_domains() {
    for choice in ["|", "else"] {
        for rust_prefix in [
            "",
            "if accept(x),",
            "let k:i32 = std::convert::identity(*x),",
        ] {
            let (vo, target) = plan(&format!(
                "struct Q; relation R(x:i32); relation S(x:i32); \
                 answer(x) :- R(x), if *x >= 0, {rust_prefix} \
                 {{ if *x == 1 }} {choice} {{ S(x) }}, if *x <= 2;"
            ));
            if rust_prefix.is_empty() {
                for seed in 0..8 {
                    let database = cq_testkit::databases::binary_database(&vo.cq, seed);
                    assert_eq!(
                        cq_testkit::semantics::evaluate_ra(&target, &database).output,
                        cq_testkit::cq_semantics::evaluate_cq(&vo.cq, &database)
                    );
                }
            }
        }
    }
}

#[test]
fn branch_generators_capture_written_environment_and_keep_kind() {
    for separator in ["|", "else"] {
        let (vo, ra) = plan(&format!(
            "struct Q; relation R(a:i32); relation answer(b:i32); answer(b) :- R(a), {{ for b in 0..a }} {separator} {{ for b in 1..a }};"
        ));
        assert_eq!(
            syn::parse2::<cq_lang_vo::Program>(vo.to_token_stream()).unwrap(),
            vo
        );
        assert_eq!(
            syn::parse2::<cq_lang_ra::Program>(ra.to_token_stream()).unwrap(),
            ra
        );
        let generators: Vec<_> = ra
            .plan
            .definitions
            .iter()
            .enumerate()
            .filter_map(|(i, d)| match &d.operator {
                cq_lang_ra::Operator::Bind(b) if b.for_token.is_some() => Some((i, b)),
                _ => None,
            })
            .collect();
        assert_eq!(generators.len(), 2);
        for (index, binding) in generators {
            let environment = binding.environment.as_ref().unwrap();
            assert!(environment.path.is_some());
            assert_eq!(
                environment
                    .variables
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
                ["a"]
            );
            let mut changed = ra.clone();
            let cq_lang_ra::Operator::Bind(b) = &mut changed.plan.definitions[index].operator
            else {
                unreachable!()
            };
            b.for_token = None;
            assert!(check(&vo, &changed).is_err());
            let mut changed = ra.clone();
            let cq_lang_ra::Operator::Bind(b) = &mut changed.plan.definitions[index].operator
            else {
                unreachable!()
            };
            b.environment.as_mut().unwrap().variables.clear();
            assert!(check(&vo, &changed).is_err());
        }
    }
}

#[test]
fn branch_literal_constants_are_local_filters_not_disjoint_interface_values() {
    let (vo, target) = plan("struct Q; relation R(a:i32,b:i32); answer(x) :- {R(x,7)} | {R(x,8)};");
    assert!(cq_lang_ra::disjoint_unions(&target).unwrap().is_empty());
    let mut changed = target.clone();
    let rename = changed
        .plan
        .definitions
        .iter_mut()
        .find_map(|d| match &mut d.operator {
            cq_lang_ra::Operator::Rename(r) => Some(r),
            _ => None,
        })
        .unwrap();
    assert_eq!(rename.mapping.len(), 1);
    rename.constants = Some(syn::parse_quote!(constants {b = 9}));
    cq_lang_ra::contract::check(&changed).unwrap();
    assert!(check(&vo, &changed).is_err());
}

#[test]
fn negation_in_hoisted_arms_has_its_free_prefix_domain_before_early_checks() {
    use cq_testkit::{cq_semantics, databases, semantics};
    for operator in ["|", "else"] {
        let source: cq::Program = syn::parse_str(&format!(
            "struct Negative; relation R(a:i32); relation S(a:i32,b:i32); relation K(a:i32,b:i32);
             answer(x,y) :- R(x), {{ S(x,y), !K(x,_) }} {operator} {{ S(x,y), !K(_,y) }};"
        ))
        .unwrap();
        let normal = cq::normal::normalize(&source).unwrap();
        for split in [cq_order::Split::Coarse, cq_order::Split::Fine] {
            let ordered = cq_order::compile(
                &normal,
                &cq_order::Options {
                    heuristic: cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
                    split,
                },
            )
            .unwrap();
            let ra = compile(&ordered).unwrap();
            check(&ordered, &ra).unwrap();
            let mut changed = ra.clone();
            let negative = changed
                .plan
                .definitions
                .iter()
                .find_map(|d| match &d.operator {
                    cq_lang_ra::Operator::Rename(r) if r.relation == "K" => Some(d.result.clone()),
                    _ => None,
                })
                .unwrap();
            let anti = changed
                .plan
                .definitions
                .iter_mut()
                .find_map(|d| match &mut d.operator {
                    cq_lang_ra::Operator::Anti(a) if a.right == negative => Some(a),
                    _ => None,
                })
                .unwrap();
            anti.columns.clear();
            cq_lang_ra::contract::check(&changed).unwrap();
            assert!(check(&ordered, &changed).is_err(), "{operator}/{split:?}");
            for seed in 0..16 {
                let database = databases::binary_database(&source, seed);
                assert_eq!(
                    semantics::evaluate_ra(&ra, &database).output,
                    cq_semantics::evaluate_cq(&source, &database),
                    "{operator}/{split:?}/{seed}"
                );
            }
        }
    }
}
