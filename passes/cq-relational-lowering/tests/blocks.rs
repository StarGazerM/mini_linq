use cq_lang_vo::cq;
use quote::ToTokens;

/// The lowering alone; the seam table is dropped.
fn compile(cq: &cq::Program, source: &cq_lang_vo::Program) -> syn::Result<cq_lang_ra::Program> {
    cq_relational_lowering::compile(cq, source).map(|(target, _)| target)
}

/// The seam check with the side table of the supplied VO's own lowering.
fn check(cq: &cq::Program, source: &cq_lang_vo::Program, target: &cq_lang_ra::Program) -> syn::Result<()> {
    let (_, steps) = cq_relational_lowering::compile(cq, source)?;
    cq_relational_lowering::check(cq, source, target, &steps)
}

/// Structural refinement of a VO the target was not lowered from (CK-RLStructure only).
fn refines(cq: &cq::Program, source: &cq_lang_vo::Program, target: &cq_lang_ra::Program) -> syn::Result<()> {
    let analysis = cq_lang_vo::contract::check(cq, source)?;
    cq_relational_lowering::check_reference(cq, &source.valuation.steps, &analysis, target)
}


fn plan(text: &str) -> (cq::Program, cq_lang_vo::Program, cq_lang_ra::Program) {
    let source: cq::Program = syn::parse_str(text).unwrap();
    let normal = cq::normal::normalize(&source).unwrap();
    let vo = cq_order::compile(&normal, &Default::default()).unwrap();
    let ra = compile(&normal, &vo).unwrap();
    check(&normal, &vo, &ra).unwrap();
    (normal, vo, ra)
}

#[test]
fn nested_blocks_keep_paths_and_project_local_witnesses() {
    let (_, vo, ra) = plan(
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
    let (_, _, ra) = plan(
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
                let ra = compile(&normal, &ordered).unwrap_or_else(|e| panic!("{options:?}: {e}"));
                check(&normal, &ordered, &ra).unwrap_or_else(|e| panic!("supplied {options:?}: {e}"));
                refines(&normal, &written, &ra).unwrap_or_else(|e| panic!("written {options:?}: {e}"));
            }
        }
    }
}

#[test]
fn source_target_contract_rejects_branch_and_negative_mutations() {
    let (cq, vo, ra) = plan(
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
    assert!(check(&cq, &vo, &changed).is_err());
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
    assert!(check(&cq, &vo, &changed).is_err());
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
    assert!(check(&cq, &vo, &changed).is_err());
}

#[test]
fn negative_operands_reject_an_added_selection_by_reference() {
    let (cq, vo, ra) = plan("struct Q; relation R(x:i32); relation S(x:i32); answer(x) :- {R(x)} else {S(x)};");
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
    assert!(check(&cq, &vo, &changed).is_err());
}

#[test]
fn suffix_rust_cannot_move_into_just_one_arm() {
    let (cq, vo, ra) = plan(
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
    assert!(check(&cq, &vo, &changed).is_err());
}

#[test]
fn cq_rewrite_is_identity_but_vo_can_swap_independent_rust_with_a_rust_block() {
    let source: cq::Program = syn::parse_quote! { struct Q; relation R(x:i32); answer(x) :- R(x), if prefix(x), {if arm(x)} | {if other(x)}; };
    let mut target = source.clone();
    let mut items: Vec<_> = target.query.body.iter().cloned().collect();
    items.swap(1, 2);
    target.query.body = items.into_iter().collect();
    assert!(cq::check_rewrite(&source, &target).is_err());

    let written = cq_order::compile(&source, &Default::default()).unwrap();
    let mut reordered = written.clone();
    reordered.valuation.steps.swap(1, 2);
    cq_lang_vo::contract::check(&source, &reordered).unwrap();
    let target = compile(&source, &reordered).unwrap();
    check(&source, &reordered, &target).unwrap();
    refines(&source, &written, &target).unwrap();
}

#[test]
fn moved_rust_blocks_record_only_their_free_variable_scope() {
    for body in [
        "{ if accept(x) } | { if other(x) }",
        "{ let z:i32 = *x } | { let z:i32 = *x + 1 }",
        "{ for z in values(x) } | { for z in other(x) }",
    ] {
        let source: cq::Program = syn::parse_str(&format!(
            "struct Q; relation R(x:i32); relation S(y:i32); \
             answer(x,y) :- R(x), S(y), {body};"
        ))
        .unwrap();
        let mut moved = cq_order::compile(&source, &cq_order::Options {
            heuristic: cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
            split: cq_order::Split::Coarse,
        }).unwrap();
        let block = moved.valuation.steps.iter()
            .position(|step| matches!(step, cq_lang_vo::Step::Block(..))).unwrap();
        if block != 1 {
            let block = moved.valuation.steps.remove(block);
            moved.valuation.steps.insert(1, block);
        }
        cq_lang_vo::contract::check(&source, &moved).unwrap();
        let target = compile(&source, &moved).unwrap();
        check(&source, &moved, &target).unwrap();
        let environments: Vec<_> = target
            .plan
            .definitions
            .iter()
            .filter_map(|definition| match &definition.operator {
                cq_lang_ra::Operator::Guard(guard) => guard.environment.as_ref(),
                cq_lang_ra::Operator::Bind(binding) => binding.environment.as_ref(),
                _ => None,
            })
            .collect();
        assert_eq!(environments.len(), 2);
        for environment in environments {
            assert_eq!(
                environment
                    .variables
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
                ["x"],
            );
        }
    }
}

#[test]
fn identical_rust_payloads_still_have_distinct_sibling_paths() {
    let (cq, vo, ra) = plan("struct Q; relation R(x:i32); answer(x) :- R(x), {if arm(x)} | {if arm(x)};");
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
    assert!(check(&cq, &vo, &changed).is_err());
}

#[test]
fn positive_distribution_can_reference_prefix_rust_but_negative_cuts_cannot() {
    for operator in ["|", "else"] {
        let (cq, vo, ra) = plan(&format!(
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
        check(&cq, &vo, &target).unwrap_or_else(|e| panic!("positive {operator}: {e}"));
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
            assert!(check(&cq, &vo, &negative).is_err());
        }
    }
}

#[test]
fn nested_and_sibling_scopes_keep_their_own_relational_predicates() {
    for choice in ["|", "else"] {
        let text = format!(
            "struct Q; relation R(x:i32); relation S(x:i32); answer(x) :- \
             R(x), where x >= 1, \
             {{ S(x), where x <= 2, {{ R(x), where x != 0 }} | {{ S(x), where x >= 2 }} }} \
             {choice} {{ S(x), where x <= 3 }}, where x <= 4;"
        );
        let (cq, _, target) = plan(&text);
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
        let written = cq_order::compile_written(&cq).unwrap();
        let reference = compile(&cq, &written).unwrap();
        check(&cq, &written, &reference).unwrap();
        for seed in 0..8 {
            let database = cq_testkit::databases::binary_database(&cq, seed);
            assert_eq!(
                cq_testkit::semantics::evaluate_ra(&target, &database).output,
                cq_testkit::cq_semantics::evaluate_cq(&cq, &database)
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
            let (cq, _, target) = plan(&format!(
                "struct Q; relation R(x:i32); relation S(x:i32); \
                 answer(x) :- R(x), where x >= 0, {rust_prefix} \
                 {{ where x == 1 }} {choice} {{ S(x) }}, where x <= 2;"
            ));
            if rust_prefix.is_empty() {
                for seed in 0..8 {
                    let database = cq_testkit::databases::binary_database(&cq, seed);
                    assert_eq!(
                        cq_testkit::semantics::evaluate_ra(&target, &database).output,
                        cq_testkit::cq_semantics::evaluate_cq(&cq, &database)
                    );
                }
            }
        }
    }
}

#[test]
fn branch_generators_capture_written_environment_and_keep_kind() {
    for separator in ["|", "else"] {
        let (cq, vo, ra) = plan(&format!(
            "struct Q; relation R(a:i32); relation answer(b:i32); answer(b) :- R(a), {{ for b in 0..a }} {separator} {{ for b in 1..a }};"
        ));
        assert_eq!(
            parse_vo(&cq, vo.to_token_stream()),
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
            assert!(check(&cq, &vo, &changed).is_err());
            let mut changed = ra.clone();
            let cq_lang_ra::Operator::Bind(b) = &mut changed.plan.definitions[index].operator
            else {
                unreachable!()
            };
            b.environment.as_mut().unwrap().variables.clear();
            assert!(check(&cq, &vo, &changed).is_err());
        }
    }
}

#[test]
fn branch_literal_constants_are_local_filters_not_disjoint_interface_values() {
    let (cq, vo, target) = plan("struct Q; relation R(a:i32,b:i32); answer(x) :- {R(x,7)} | {R(x,8)};");
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
    assert!(check(&cq, &vo, &changed).is_err());
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
            let ra = compile(&normal, &ordered).unwrap();
            check(&normal, &ordered, &ra).unwrap();
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
            assert!(check(&normal, &ordered, &changed).is_err(), "{operator}/{split:?}");
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

#[test]
fn block_placement_round_trips_and_controls_both_elaboration_entry_points() {
    use cq_lang_vo::{Placement, Step};
    fn placements(steps: &[Step], expected: Placement) {
        for step in steps {
            if let Step::Block(block, _) = step {
                assert_eq!(block.placement, expected);
                for branch in &block.branches {
                    placements(&branch.steps, expected);
                }
            }
        }
    }
    let (cq, hoisted, target) = plan(
        "struct Q; relation R(x:i32); relation S(x:i32,y:i32); relation T(x:i32,y:i32);
         answer(x,y) :- R(x), { S(x,y), { R(x) } | { R(x) } } else { T(x,y) };",
    );
    placements(&hoisted.valuation.steps, Placement::Hoisted);
    let written = cq_order::compile_written(&cq).unwrap();
    placements(&written.valuation.steps, Placement::Correlated);
    for vo in [&hoisted, &written] {
        let reparsed: cq_lang_vo::Program = parse_vo(&cq, vo.to_token_stream());
        assert_eq!(*vo, reparsed);
        cq_lang_vo::contract::check(&cq, &reparsed).unwrap();
    }
    let reference = compile(&cq, &written).unwrap();
    check(&cq, &written, &reference).unwrap();
    let correlated = compile(&cq, &written).unwrap();
    check(&cq, &written, &correlated).unwrap();
    assert!(check(&cq, &hoisted, &correlated).is_err());
    assert!(check(&cq, &written, &target).is_err());
    for (ra, correlated) in [(&target, false), (&reference, true), (&correlated, true)] {
        for definition in &ra.plan.definitions {
            if let cq_lang_ra::Operator::Union(union) = &definition.operator {
                assert_eq!(union.origin.as_ref().unwrap().over.is_some(), correlated);
            }
        }
    }
    for seed in 0..16 {
        let database = cq_testkit::databases::binary_database(&cq, seed);
        let expected = cq_testkit::cq_semantics::evaluate_cq(&cq, &database);
        for ra in [&target, &reference, &correlated] {
            assert_eq!(
                cq_testkit::semantics::evaluate_ra(ra, &database).output,
                expected
            );
        }
    }
}

#[test]
fn block_placement_is_required_and_checked_before_elaboration() {
    use cq_lang_vo::{Placement, Step};
    assert!(syn::parse_str::<Step>("block @0(x) {R@0.0.0(x);} | {R@0.1.0(x);};").is_err());
    let (rust_cq, mut rust, _) = plan(
        "struct Q; relation R(x:i32); answer(x) :- R(x),
         { if accept(x) } | { if other(x) };",
    );
    let Step::Block(block, _) = &mut rust.valuation.steps[1] else {
        panic!("expected block");
    };
    assert_eq!(block.placement, Placement::Correlated);
    block.placement = Placement::Hoisted;
    assert!(cq_lang_vo::contract::check(&rust_cq, &rust).is_err());

    let (pure_cq, _, _) = plan(
        "struct Q; relation R(x:i32); relation S(x:i32,y:i32);
         answer(x,y) :- R(x), {S(x,y)} | {S(x,y)};",
    );
    let mut missing_prefix = cq_order::compile_written(&pure_cq).unwrap();
    missing_prefix.valuation.steps.swap(0, 1);
    assert!(cq_lang_vo::contract::check(&pure_cq, &missing_prefix).is_err());

    let mut split = cq_order::compile(
        &pure_cq,
        &cq_order::Options {
            heuristic: cq_order::Heuristic::Written,
            split: cq_order::Split::Fine,
        },
    )
    .unwrap();
    let Step::Block(block, semi) = split.valuation.steps.last().unwrap() else {
        panic!("expected block");
    };
    let mut repeated = block.clone();
    let semi = *semi;
    repeated.placement = Placement::Correlated;
    split.valuation.steps.push(Step::Block(repeated, semi));
    assert!(cq_lang_vo::contract::check(&pure_cq, &split).is_err());
}

#[test]
fn rust_expressions_and_blocks_cannot_delay_ready_filters_past_bindings() {
    let source: cq::Program = syn::parse_str(
        "struct Q; relation R(x:i32); answer(y) :-
         R(x), if accept(x), let y:i32 = x + 1;",
    )
    .unwrap();
    let normal = cq::normal::normalize(&source).unwrap();
    let planned = cq_order::compile(&normal, &Default::default()).unwrap();
    let mut moved = planned.clone();
    moved.valuation.steps.swap(1, 2);
    assert!(cq_lang_vo::contract::check(&normal, &moved).unwrap_err().to_string()
        .contains("VO-Order I2"));
    let target = compile(&normal, &planned).unwrap();
    check(&normal, &planned, &target).unwrap();

    let source: cq::Program = syn::parse_str(
        "struct Q; relation R(x:i32); answer(y) :-
         R(x), let y:i32 = x + 1, { if accept(x) } | { if other(x) };",
    )
    .unwrap();
    let normal = cq::normal::normalize(&source).unwrap();
    let planned = cq_order::compile(&normal, &Default::default()).unwrap();
    let mut late = planned.clone();
    late.valuation.steps.swap(1, 2);
    assert!(cq_lang_vo::contract::check(&normal, &late).unwrap_err().to_string()
        .contains("VO-Order I2"));
    let target = compile(&normal, &planned).unwrap();
    check(&normal, &planned, &target).unwrap();
}

fn parse_vo(cq: &cq::Program, tokens: proc_macro2::TokenStream) -> cq_lang_vo::Program {
    syn::parse::Parser::parse2(
        |input: syn::parse::ParseStream<'_>| cq_lang_vo::Program::parse(input, cq),
        tokens,
    ).unwrap()
}
