//! IX → Physical: the pass's output equals independently authored Physical
//! programs, and every output satisfies the Physical contract.
use cq_ir::policy::Policy;
use cq_ir::{cq, index_requirements as ix, physical};
use cq_physical_lowering::{check, compile};
use quote::{ToTokens, quote};

fn parse_policy(tokens: proc_macro2::TokenStream) -> Policy {
    syn::parse2(tokens).unwrap()
}

/// No reductions: relation-at-a-time joins in written order.
const UNREDUCED: cq_reduce::Options = cq_reduce::Options {
    guards: cq_reduce::Pushdown::None,
    semijoins: cq_reduce::Reductions::None,
};

/// Single-atom guards pushed onto their atoms and the Yannakakis full reducer.
const REDUCED: cq_reduce::Options = cq_reduce::Options {
    guards: cq_reduce::Pushdown::Atoms,
    semijoins: cq_reduce::Reductions::Tree,
};

/// CQ text → NF → VO in written order → reduced under `options` → RA → IX.
fn ix_of(tokens: proc_macro2::TokenStream, options: &cq_reduce::Options) -> ix::Module {
    let source: cq::Module = syn::parse2(tokens).unwrap();
    let normal = cq::normal::normalize(&source).unwrap();
    let plan = cq_order::compile(&normal, &cq_order::Options::default()).unwrap();
    let plan = cq_reduce::compile(&plan, options).unwrap();
    let target = cq_elaborate::compile(&plan).unwrap();
    cq_index_policy::compile(&target).unwrap()
}

/// `r0 = L; r1 = R; r2 = r0 ⋈ r1; r3 = S; r4 = r2 ⋈ r3; r5 = π r4`.
fn chain_ix() -> ix::Module {
    ix_of(
        quote! {
            struct Chain;
            relation L(a: i32);
            relation R(a: i32, b: i32);
            relation S(b: i32, c: i32);
            answer(a, c) :- L(a), R(a, b), S(b, c);
        },
        &UNREDUCED,
    )
}

/// Fine subatoms project the triangle's atoms to each variable prefix.
fn triangle_ix() -> ix::Module {
    let source: cq::Module = syn::parse_quote! {
        struct Triangle;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        relation T(a: i32, c: i32);
        answer(a, b, c) :- R(a, b), S(b, c), T(a, c);
    };
    let normal = cq::normal::normalize(&source).unwrap();
    let plan = cq_order::compile(
        &normal,
        &cq_order::Options {
            heuristic: cq_order::Heuristic::Written,
            split: cq_order::Split::Fine,
        },
    )
    .unwrap();
    let target = cq_elaborate::compile(&plan).unwrap();
    cq_index_policy::compile(&target).unwrap()
}

#[test]
fn fine_triangle_keeps_intermediate_projections_by_default() {
    // Pin the original Fine triangle RA independently of elaboration's
    // elimination of redundant projections on semijoin right operands.
    let mut plan = triangle_ix().relational_plan;
    plan.plan = syn::parse_quote! {
        r0 = rename R {a -> a, b -> b};
        r1 = project r0 keep {a};
        r2 = rename T {a -> a, c -> c};
        r3 = project r2 keep {a};
        r4 = semijoin r1 with r3;
        r5 = natural_join r4 with r0;
        r6 = rename S {b -> b, c -> c};
        r7 = project r6 keep {b};
        r8 = semijoin r5 with r7;
        r9 = natural_join r8 with r6;
        r10 = natural_join r9 with r2;
        r11 = project r10 keep {a, b, c};
        output r11 as answer(a, b, c);
    };
    let source = cq_index_policy::compile(&plan).unwrap();
    let policy = parse_policy(quote! { policy { R; S; T; } });
    let (target, _) = lower(&source, &policy);
    let keep = parse_policy(quote! { policy { R; S; T; project = keep; } });
    assert_eq!(target, lower(&source, &keep).0);
    assert_program(
        &target,
        quote! {
            r0 = let rename;
            r1 = pipe project;
            r2 = let rename;
            r3 = let project;
            r4 = pipe semi[a] r1, r3;
            r5 = pipe probe[a] r4, r0;
            r6 = let rename;
            r7 = let project;
            r8 = pipe semi[b] r5, r7;
            r9 = pipe probe[b] r8, r6;
            r10 = pipe probe[a, c] r9, r2;
            r11 = pipe canon project;
            return r11;
        },
    );
    let canon = parse_policy(quote! { policy { R; S; T; project = canon; } });
    let (canonical, _) = lower(&source, &canon);
    let mut expected = target;
    for index in [1, 3, 7] {
        expected.program.definitions[index].canon = Some(Default::default());
    }
    assert_eq!(canonical, expected);
}

#[test]
fn fine_triangle_canonicalizes_every_key_dropping_projection() {
    let source = triangle_ix();
    let keep = parse_policy(quote! { policy { R; S; T; } });
    let (kept, _) = lower(&source, &keep);
    let explicit_keep = parse_policy(quote! { policy { R; S; T; project = keep; } });
    assert_eq!(kept, lower(&source, &explicit_keep).0);
    for policy in [
        quote! { policy { R; S; T; project = canon; } },
        quote! { policy { R canon; S canon; T canon; project = canon; } },
    ] {
        let policy = parse_policy(policy);
        let (target, _) = lower(&source, &policy);
        let infos = cq_ir::relational_plan::check_in(
            source.relational_plan.program.inputs(),
            &source.relational_plan.plan,
        )
        .unwrap();
        let mut dropping = 0;
        for (logical, definition) in source
            .relational_plan
            .plan
            .definitions
            .iter()
            .zip(&target.program.definitions)
        {
            if let cq_ir::relational_plan::Operator::Project(project) = &logical.operator {
                let input = &infos[&cq_ir::symbol_name(&project.input)].heading;
                let kept = &infos[&cq_ir::symbol_name(&logical.result)].heading;
                if kept != input {
                    dropping += 1;
                    assert!(definition.canon.is_some(), "{}", definition.name);
                }
            }
        }
        assert!(dropping > 0);
        for (definition, kept) in target
            .program
            .definitions
            .iter()
            .zip(&kept.program.definitions)
        {
            assert_eq!(definition.mode, kept.mode);
            assert_eq!(definition.operator, kept.operator);
            if !matches!(definition.operator, physical::Operator::Project(_)) {
                assert_eq!(definition.canon, kept.canon);
            }
        }
        // Canonicalizing a prefix does not change the returned definition.
        assert_eq!(
            target.program.definitions.last(),
            kept.program.definitions.last()
        );
        assert!(!check(&source, &target, &keep));

        let mut tampered = target.clone();
        tampered.program.definitions[1].canon = None;
        physical::contract::check(&tampered).unwrap();
        assert!(!check(&source, &tampered, &policy));
        let mut tampered = target;
        tampered.program.definitions[1].mode = syn::parse_quote!(let);
        physical::contract::check(&tampered).unwrap();
        assert!(!check(&source, &tampered, &policy));
    }
}

#[test]
fn a_keyed_projection_is_a_canonical_let() {
    let plan = syn::parse_quote! {
        struct Projected;
        relation L(a: i32);
        relation R(a: i32, b: i32);
        answer(a) :- L(a), R(a, b);
        relational {
            l = rename L {a -> a};
            r = rename R {a -> a, b -> b};
            p = project r keep {a};
            j = natural_join l with p;
            output j as answer(a);
        }
    };
    let source = cq_index_policy::compile(&plan).unwrap();
    for policy in [
        quote! { policy { L; R; project = canon; } },
        quote! { policy { L canon; R keyed [a] canon; project = canon; } },
    ] {
        let policy = parse_policy(policy);
        let (target, _) = lower(&source, &policy);
        // A host's storage key [a] is not a uniqueness key: R can contain
        // several distinct b values for one a even when its rows are canonical.
        assert_program(
            &target,
            quote! {
                l = alias rename; r = alias rename; p = let canon project;
                j = pipe canon probe[a] l, p; return j;
            },
        );
    }
}

#[test]
fn projection_canon_uses_the_inferred_key_instead_of_the_whole_input_heading() {
    for (host, columns, canon) in [
        (quote! { R canon; }, quote! { a }, false),
        (quote! { R canon; }, quote! { a, b }, false),
        (quote! { R canon; }, quote! { b }, true),
        (quote! { R; }, quote! { a, b }, true),
    ] {
        let plan = syn::parse_quote! {
            struct Extended;
            relation R(a: i32);
            answer(#columns) :- R(a), let b: i32 = 0;
            relational {
                r = rename R {a -> a};
                e = extend r with {b: i32} value ((0,));
                p = project e keep {#columns};
                s = select p if (true);
                output s as answer(#columns);
            }
        };
        let source = cq_index_policy::compile(&plan).unwrap();
        let policy = parse_policy(quote! { policy { #host project = canon; } });
        let (target, _) = lower(&source, &policy);
        assert_eq!(target.program.definitions[2].canon.is_some(), canon);
        assert!(matches!(
            target.program.definitions[2].mode,
            physical::Mode::Pipe(_)
        ));
        let keep = parse_policy(quote! { policy { #host project = keep; } });
        let (kept, _) = lower(&source, &keep);
        assert_eq!(
            target.program.definitions.last(),
            kept.program.definitions.last()
        );
    }
}

/// The pass's output, its contract checked, as the authored program text.
fn lower(source: &ix::Module, policy: &Policy) -> (physical::Module, String) {
    let target = compile(source, policy).unwrap();
    physical::contract::check(&target).unwrap();
    assert!(check(source, &target, policy));
    let text = target.program.to_token_stream().to_string();
    (target, text)
}

fn assert_program(target: &physical::Module, program: proc_macro2::TokenStream) {
    let source = &target.index_requirements;
    let expected: physical::Module = syn::parse2(quote! { #source physical { #program } }).unwrap();
    assert_eq!(
        target.program.to_token_stream().to_string(),
        expected.program.to_token_stream().to_string()
    );
    assert_eq!(*target, expected);
}

#[test]
fn seek_policy_probes_the_hash_chain_and_flags_the_projected_return() {
    let source = chain_ix();
    let policy = parse_policy(quote! { policy { L; R; S; } });
    let (target, _) = lower(&source, &policy);
    assert_program(
        &target,
        quote! {
            r0 = alias rename;
            r1 = let rename;
            r2 = pipe probe[a] r0, r1;
            r3 = let rename;
            r4 = pipe probe[b] r2, r3;
            r5 = pipe canon project;
            return r5;
        },
    );
    // The merge policy decides a different program.
    let merge = parse_policy(quote! { policy { L; R; S; join = merge; } });
    assert!(!check(&source, &target, &merge));
}

#[test]
fn missing_demands_name_the_ra_join_and_do_not_fall_back_to_scans() {
    let policy = parse_policy(quote! { policy { L; R; S; } });
    let mut source = chain_ix();
    source.indexes.pop();
    ix::contract::check(&source).unwrap();
    assert_eq!(
        compile(&source, &policy).unwrap_err().to_string(),
        "natural join `r4` has no index use"
    );

    let mut semi: ix::Module = syn::parse_quote! {
        struct Semi;
        relation L(a:i32); relation R(a:i32);
        answer(a) :- L(a), R(a);
        relational {
            l = rename L {a -> a}; r = rename R {a -> a};
            j = semijoin l with r;
            output j as answer(a);
        }
        indexes {}
    };
    let policy = parse_policy(quote! { policy { L; R; } });
    ix::contract::check(&semi).unwrap();
    assert_eq!(
        compile(&semi, &policy).unwrap_err().to_string(),
        "semijoin `j` has no index use"
    );
    semi.indexes = syn::parse_quote! { arbitrary_name = @r[0]; };
    let (target, _) = lower(&semi, &policy);
    assert_program(
        &target,
        quote! {
            l = alias rename; r = let rename;
            j = pipe canon semi[a] l, r; return j;
        },
    );
}

#[test]
fn keyed_hosts_answer_probes_in_place_and_canon_hosts_make_the_return_unique() {
    let source = chain_ix();
    let policy = parse_policy(quote! {
        policy { L canon; R keyed [a] canon; S keyed [b] canon; }
    });
    let (target, _) = lower(&source, &policy);
    // `project` keeps `a` and `c` but drops the key column `b`: not unique.
    assert_program(
        &target,
        quote! {
            r0 = alias rename;
            r1 = alias rename;
            r2 = pipe probe[a] r0, r1;
            r3 = alias rename;
            r4 = pipe probe[b] r2, r3;
            r5 = pipe canon project;
            return r5;
        },
    );

    // `r3 = project r2 keep {a, b}` keeps the whole key of L ⋈ R.
    let full = ix_of(
        quote! {
            struct Full;
            relation L(a: i32);
            relation R(a: i32, b: i32);
            answer(a, b) :- L(a), R(a, b);
        },
        &UNREDUCED,
    );
    let policy = parse_policy(quote! { policy { L canon; R keyed [a] canon; } });
    let (_, text) = lower(&full, &policy);
    assert!(text.contains("r3 = pipe project ;"), "{text}");
    let policy = parse_policy(quote! { policy { L canon; R keyed [a]; } });
    let (_, text) = lower(&full, &policy);
    assert!(text.contains("r3 = pipe canon project ;"), "{text}");
}

#[test]
fn merge_policy_merges_every_join_and_materializes_the_side_whose_key_changes() {
    let source = chain_ix();
    let policy = parse_policy(quote! { policy { L; R; S; join = merge; } });
    let (target, _) = lower(&source, &policy);
    assert_program(
        &target,
        quote! {
            r0 = let rename;
            r1 = let rename;
            r2 = let merge[a] r0, r1;
            r3 = let rename;
            r4 = pipe merge[b] r2, r3;
            r5 = pipe canon project;
            return r5;
        },
    );

    // Ordered hosts are merged in place; a merge sorted on the next key
    // stays a pipe.
    let same_key = ix_of(
        quote! {
            struct SameKey;
            relation L(a: i32);
            relation R(a: i32, b: i32);
            relation S(a: i32, c: i32);
            answer(a, b, c) :- L(a), R(a, b), S(a, c);
        },
        &UNREDUCED,
    );
    let policy = parse_policy(quote! {
        policy { L keyed [a] ordered; R keyed [a] ordered; S keyed [a] ordered; join = merge; }
    });
    let (target, _) = lower(&same_key, &policy);
    assert_program(
        &target,
        quote! {
            r0 = alias rename;
            r1 = alias rename;
            r2 = pipe merge[a] r0, r1;
            r3 = alias rename;
            r4 = pipe merge[a] r2, r3;
            r5 = pipe canon project;
            return r5;
        },
    );
    // An unordered key does not answer a merge: the rename is built over.
    let unordered = parse_policy(quote! {
        policy { L keyed [a]; R keyed [a] ordered; S keyed [a] ordered; join = merge; }
    });
    let (_, text) = lower(&same_key, &unordered);
    assert!(text.contains("r0 = let rename ;"), "{text}");
    assert!(text.contains("r1 = alias rename ;"), "{text}");
}

#[test]
fn a_cartesian_join_is_a_unit_probe_under_either_join_rule() {
    let source = ix_of(
        quote! {
            struct Cross;
            relation L(a: i32);
            relation R(b: i32);
            answer(a, b) :- L(a), R(b);
        },
        &UNREDUCED,
    );
    for policy in [
        parse_policy(quote! { policy { L; R; } }),
        parse_policy(quote! { policy { L; R; join = merge; } }),
    ] {
        let (target, text) = lower(&source, &policy);
        assert!(text.contains("r2 = pipe probe [] r0 , r1 ;"), "{text}");
        let analysis = physical::contract::check(&target).unwrap();
        assert_eq!(analysis.key_reads["r1"], vec![("r2".to_string(), vec![])]);
    }
}

#[test]
fn comparisons_on_the_ordered_host_key_become_a_range() {
    let source = ix_of(
        quote! {
            struct Windowed;
            relation R(a: i32, b: i32);
            answer(a, b) :- R(a, b), if a >= 2 && 9 > a && b != 5, if b >= 0;
        },
        &UNREDUCED,
    );
    let ordered = parse_policy(quote! { policy { R keyed [a] ordered; } });
    let (target, _) = lower(&source, &ordered);
    // The second select's input is not a host rename: it keeps its guard.
    assert_program(
        &target,
        quote! {
            r0 = alias rename;
            r1 = pipe range[a];
            r2 = pipe select;
            r3 = pipe canon project;
            return r3;
        },
    );
    let analysis = physical::contract::check(&target).unwrap();
    assert_eq!(
        analysis.range_reads["r0"],
        vec![("r1".to_string(), "a".to_string())]
    );

    // The key must be ordered, single, and bounded by the condition.
    let unordered = parse_policy(quote! { policy { R keyed [a]; } });
    let (_, text) = lower(&source, &unordered);
    assert!(text.contains("r1 = pipe select ;"), "{text}");
    let other_column = parse_policy(quote! { policy { R keyed [b] ordered; } });
    let (_, text) = lower(&source, &other_column);
    assert!(text.contains("r1 = pipe select ;"), "{text}");
    let unbounded = ix_of(
        quote! {
            struct Unbounded;
            relation R(a: i32, b: i32);
            answer(a, b) :- R(a, b), if a != 3 && b < 4;
        },
        &UNREDUCED,
    );
    let (_, text) = lower(&unbounded, &ordered);
    assert!(text.contains("r1 = pipe select ;"), "{text}");
    let reversed = ix_of(
        quote! {
            struct Reversed;
            relation R(a: i32, b: i32);
            answer(a, b) :- R(a, b), if b < 4 && 7 <= a;
        },
        &UNREDUCED,
    );
    let (_, text) = lower(&reversed, &ordered);
    assert!(text.contains("r1 = pipe range [a] ;"), "{text}");
    // A strict bound past the end of `i32` is no window.
    let overflowing = ix_of(
        quote! {
            struct Overflowing;
            relation R(a: i32, b: i32);
            answer(a, b) :- R(a, b), if a > 2147483647;
        },
        &UNREDUCED,
    );
    let (_, text) = lower(&overflowing, &ordered);
    assert!(text.contains("r1 = pipe select ;"), "{text}");
}

#[test]
fn full_reduction_semijoins_test_membership_and_shared_results_are_lets() {
    // r0 = R; r1 = L; r2 = r0 ⋉ r1; r3 = r1 ⋉ r2; r4 = r3 ⋈ r2; r5 = π r4.
    let source = ix_of(
        quote! {
            struct Reduced;
            relation L(a: i32, b: i32);
            relation R(b: i32, c: i32);
            answer(a) :- L(a, b), R(b, c);
        },
        &REDUCED,
    );
    let policy = parse_policy(quote! { policy { L keyed [b]; R; } });
    let (target, _) = lower(&source, &policy);
    assert_program(
        &target,
        quote! {
            r0 = alias rename;
            r1 = alias rename;
            r2 = let semi[b] r0, r1;
            r3 = pipe semi[b] r1, r2;
            r4 = pipe probe[b] r3, r2;
            r5 = pipe canon project;
            return r5;
        },
    );
    let analysis = physical::contract::check(&target).unwrap();
    assert_eq!(
        analysis.key_reads["r1"],
        vec![("r2".to_string(), vec!["b".to_string()])]
    );
    assert_eq!(
        analysis.key_reads["r2"],
        vec![
            ("r3".to_string(), vec!["b".to_string()]),
            ("r4".to_string(), vec!["b".to_string()])
        ]
    );

    // Without the key, L is built over; with canonical hosts the return is
    // still flagged because `r5` projects the join key away.
    let policy = parse_policy(quote! { policy { L canon; R canon; } });
    let (_, text) = lower(&source, &policy);
    assert!(text.contains("r1 = let rename ;"), "{text}");
    assert!(text.contains("r5 = pipe canon project ;"), "{text}");
}

#[test]
fn every_output_of_the_chain_query_satisfies_the_contract_under_each_policy() {
    let source = ix_of(
        quote! {
            struct Chain;
            relation L(a: i32);
            relation R(a: i32, b: i32);
            relation S(b: i32, c: i32);
            answer(a, c) :- L(a), R(a, b), S(b, c);
        },
        &REDUCED,
    );
    for policy in [
        quote! { policy { L; R; S; } },
        quote! { policy { L; R; S; join = merge; } },
        quote! { policy { L keyed [a] ordered canon; R keyed [a]; S keyed [b] ordered; } },
        quote! { policy { L keyed [a] ordered canon; R keyed [b] ordered canon; S keyed [b] ordered canon; join = merge; semijoin = prefer_lazy; } },
    ] {
        let policy = parse_policy(policy);
        let (target, _) = lower(&source, &policy);
        assert!(physical::contract::check(&target).is_ok());
    }
}

#[test]
fn the_policy_must_describe_exactly_the_declared_hosts() {
    let source = chain_ix();
    let missing = parse_policy(quote! { policy { L; R; } });
    let error = compile(&source, &missing).unwrap_err().to_string();
    assert!(error.contains("`S`"), "{error}");
    let extra = parse_policy(quote! { policy { L; R; S; T; } });
    let error = compile(&source, &extra).unwrap_err().to_string();
    assert!(error.contains("undeclared"), "{error}");
    let column = parse_policy(quote! { policy { L; R keyed [z]; S; } });
    let error = compile(&source, &column).unwrap_err().to_string();
    assert!(error.contains("no column `z`"), "{error}");
    let twice = parse_policy(quote! { policy { L; R; S; S; } });
    let error = compile(&source, &twice).unwrap_err().to_string();
    assert!(error.contains("twice"), "{error}");
}
