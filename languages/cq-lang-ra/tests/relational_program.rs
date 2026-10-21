use cq_lang_ra as ra;
use quote::{ToTokens, quote};

fn two_hop() -> proc_macro2::TokenStream {
    quote! {
            relational {
    relation road(src: i32, dst: i32);

                r0 = rename road@0 {src -> from, dst -> via};
                r1 = rename road@1 {src -> via, dst -> to};
                r2 = natural_join r0 with r1;
                r3 = project r2 keep {from, to};
                output r3 as two_hop(from, to);
            }
        }
}

fn check(tokens: proc_macro2::TokenStream) -> syn::Result<()> {
    let module: ra::Program = syn::parse2(tokens)?;
    ra::contract::check(&module).map(|_| ())
}

#[test]
fn complete_plan_is_well_formed_and_round_trips() {
    let module: ra::Program = syn::parse2(two_hop()).unwrap();
    let analysis = ra::contract::check(&module).unwrap();
    assert_eq!(analysis["r3"].heading, ["from".into(), "to".into()].into());
    assert_eq!(module.plan.definitions.len(), 4);

    let reparsed: ra::Program = syn::parse2(module.to_token_stream()).unwrap();
    assert_eq!(module, reparsed);
}

#[test]
fn result_ids_are_dag_references_not_prescribed_r_numbers() {
    check(quote! {
            relational {
    relation R(value: i32);

                source = rename R@0 {value -> x};
                answer_rows = project source keep {x};
                output answer_rows as answer(x);
            }
        })
    .unwrap();
}

#[test]
fn local_validity_does_not_enforce_cq_occurrences_or_source_order() {
    check(quote! {
            relational {
    relation R(value: i32);
            relation S(value: i32);

                from_other_declared_input = rename S@0 {value -> x};
                output from_other_declared_input as answer(x);
            }
        })
    .unwrap();

    check(quote! {
            relational {
    relation R(value: i32);
            relation S(value: i32);

                second = rename S@0 {value -> y};
                first = rename R@1 {value -> x};
                both = natural_join second with first;
                projected = project both keep {x, y};
                output projected as answer(x, y);
            }
        })
    .unwrap();
}

#[test]
fn rename_requires_a_declared_relation_and_exact_one_to_one_domain() {
    let undeclared = check(quote! {
            relational {
    relation R(value: i32);

                r0 = rename Missing@0 {value -> x};
                output r0 as answer(x);
            }
        })
    .unwrap_err();
    assert!(undeclared.to_string().contains("Missing"));

    for invalid_mapping in [
        quote!(c0 -> x),
        quote!(c0 -> x, c0 -> y),
        quote!(c0 -> x, c1 -> x),
    ] {
        let error = check(quote! {
                    relational {
        relation R(c0: i32, c1: i32);

                        r0 = rename R@0 {#invalid_mapping};
                        r1 = project r0 keep {x};
                        output r1 as answer(x);
                    }
                })
        .unwrap_err();
        assert!(
            error.to_string().contains("R")
                || error.to_string().contains("c0")
                || error.to_string().contains("x")
        );
    }
}

#[test]
fn natural_join_infers_union_heading_without_resolving_rust_types() {
    check(quote! {
            relational {
    relation People(id: PersonId, manager: PersonId);
            relation Logins(person: u64, day: u32);

                people = rename People@0 {id -> x, manager -> manager};
                logins = rename Logins@1 {person -> x, day -> day};
                joined = natural_join people with logins;
                result = project joined keep {x, day};
                output result as answer(x, day);
            }
        })
    .unwrap();
}

#[test]
fn natural_join_with_disjoint_headings_is_a_cartesian_product() {
    check(quote! {
            relational {
    relation R(value: i32);
            relation S(value: i32);

                r = rename R@0 {value -> x};
                s = rename S@1 {value -> y};
                product = natural_join r with s;
                output product as answer(x, y);
            }
        })
    .unwrap();
}

#[test]
fn project_is_a_distinct_subset_and_output_owns_tuple_order() {
    check(quote! {
            relational {
    relation R(first: i32, second: i32);

                rows = rename R@0 {first -> x, second -> z};
                result = project rows keep {x, z};
                output result as answer(z, x);
            }
        })
    .unwrap();

    for attributes in [quote!(x, x), quote!(missing)] {
        let error = check(quote! {
                    relational {
        relation R(value: i32);

                        rows = rename R@0 {value -> x};
                        result = project rows keep {#attributes};
                        output result as answer(x);
                    }
                })
        .unwrap_err();
        assert!(error.to_string().contains("x") || error.to_string().contains("missing"));
    }
}

#[test]
fn output_must_match_its_input_heading() {
    let wrong_heading = check(quote! {
            relational {
    relation R(first: i32, second: i32);

                rows = rename R@0 {first -> x, second -> y};
                output rows as answer(x);
            }
        })
    .unwrap_err();
    assert!(wrong_heading.to_string().contains("rows"));
}

#[test]
fn definitions_are_unique_and_operands_must_be_earlier() {
    let duplicate = check(quote! {
            relational {
    relation R(value: i32);

                r0 = rename R@0 {value -> x};
                r0 = project r0 keep {x};
                output r0 as answer(x);
            }
        })
    .unwrap_err();
    assert!(duplicate.to_string().contains("r0"));

    let forward = check(quote! {
            relational {
    relation R(value: i32);

                result = project rows keep {x};
                rows = rename R@0 {value -> x};
                output result as answer(x);
            }
        })
    .unwrap_err();
    assert!(forward.to_string().contains("rows"));
}

#[test]
fn every_definition_must_contribute_to_output() {
    let error = check(quote! {
            relational {
    relation R(value: i32);
            relation S(value: i32);

                used = rename R@0 {value -> x};
                unused = rename S@1 {value -> y};
                output used as answer(x);
            }
        })
    .unwrap_err();
    assert!(error.to_string().contains("unused"));
}

#[test]
fn unit_is_the_zero_attribute_relation() {
    check(quote! {
            relational {
    relation R(value: i32);

                seed = unit;
                rows = rename R@0 {value -> x};
                joined = natural_join seed with rows;
                output joined as answer(x);
            }
        })
    .unwrap();
}

#[test]
fn relational_contract_rejects_pruned_expression_dependencies() {
    let source: ra::Program = syn::parse_quote! {
            relational {
    relation R(a:i32,b:i32);
                r0 = rename R@0 {a -> x, b -> y};
                r1 = project r0 keep {x};
                r2 = guard r1 if (accept(y));
                output r2 as answer(x);
            }
        };
    assert!(ra::contract::check(&source).is_err());
}

#[test]
fn bind_requires_available_inputs_and_fresh_columns() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32);
        r = rename R@0 {a -> x,b -> y}; p = project r keep {x};
        e = bind p with z:i32 = (x + 1);
        output e as answer(x,z);
    }};
    ra::contract::check(&source).unwrap();
    for operator in [
        quote!(bind p with z:i32 = (y + 1)),
        quote!(bind p with x:i32 = (1)),
    ] {
        let mut changed = source.clone();
        changed.plan.definitions[2].operator = syn::parse2(operator).unwrap();
        assert!(ra::contract::check(&changed).is_err());
    }
}

#[test]
fn parsing_requires_an_output_consumes_the_body_and_checks_mapping_syntax() {
    for text in [
        "relational { relation R(a: i32); r = rename R@0 {a -> x}; }",
        "relational { relation R(a: i32); r = rename R@0 {a -> x}; output r as answer(x); junk }",
        "relational { relation R(a: i32); r = rename R@0 {a x}; output r as answer(x); }",
    ] {
        assert!(syn::parse_str::<ra::Program>(text).is_err(), "{text}");
    }
}

#[test]
fn relational_selection_has_only_column_equalities_and_literals() {
    check(quote! { relational {
        relation R(a: i32, b: i32);
        r = rename R@0 {a -> x, b -> y};
        s = select r if (x = -5, y = x);
        output s as answer(x, y);
    }})
    .unwrap();
    for condition in [
        quote!(x == y),
        quote!(accept(x)),
        quote!(x = y + 1),
        quote!(x = -true),
    ] {
        assert!(
            syn::parse2::<ra::Program>(quote! { relational {
                relation R(a: i32); r = rename R@0 {a -> x};
                s = select r if (#condition); output s as answer(x);
            }})
            .is_err()
        );
    }
    assert!(
        syn::parse2::<ra::Program>(quote! { relational {
            relation R(a: i32); r = rename R@0 {a -> x};
            s = extend r with {y:i32} value (1); output s as answer(x, y);
        }})
        .is_err()
    );
}

#[test]
fn opaque_nodes_form_one_ancestry_chain() {
    check(quote! { relational {
        relation R(a: i32); relation S(b: i32);
        r = rename R@0 {a -> x};
        g = guard r if (*x != 0);
        s = rename S@1 {b -> w};
        j = natural_join g with s;
        b = bind j with y: i32 = (10 / *x);
        p = project b keep {x, y}; output p as answer(x, y);
    }})
    .unwrap();
    let error = check(quote! { relational {
        relation R(a: i32); relation S(b: i32);
        r = rename R@0 {a -> x}; s = rename S@1 {b -> w};
        g = guard r if (*x != 0);
        b = bind s with y: i32 = (10 / *w);
        j = natural_join g with b;
        output j as answer(x, w, y);
    }})
    .unwrap_err();
    assert!(error.to_string().contains("one ancestry chain"));
}

#[test]
fn selection_scope_and_float_leibniz_are_checked_with_column_diagnostics() {
    for program in [
        quote! { relational {
            relation R(a: i32); r = rename R@0 {a -> x};
            s = select r if (y = 1); output s as answer(x);
        }},
        quote! { relational {
            relation R(a: f64); r = rename R@0 {a -> x};
            s = select r if (x = 1.0); output s as answer(x);
        }},
        quote! { relational {
            relation R(a: f64); relation S(a: f64);
            r = rename R@0 {a -> x}; s = rename S@1 {a -> x};
            j = natural_join r with s; p = project j keep {}; output p as answer();
        }},
    ] {
        let error = check(program).unwrap_err().to_string();
        assert!(
            error.contains("column") && (error.contains("x") || error.contains("y")),
            "{error}"
        );
    }
}

#[test]
fn rewrite_accepts_join_reordering_and_reassociation_with_existential_columns() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); relation S(b:i32,c:i32); relation T(c:i32,d:i32);
        r = rename R@0 {a -> x, b -> y};
        s = rename S@1 {b -> y, c -> z};
        t = rename T@2 {c -> z, d -> w};
        j = natural_join r with s;
        k = natural_join j with t;
        p = project k keep {x,w}; output p as answer(x,w);
    }};
    let mut target = source.clone();
    target.plan.definitions[3].operator = syn::parse_quote!(natural_join t with s);
    target.plan.definitions[4].operator = syn::parse_quote!(natural_join j with r);
    ra::contract::check(&target).unwrap();
    ra::check_rewrite(&source, &target).unwrap();
    ra::check_rewrite(&target, &source).unwrap();
}

#[test]
fn rewrite_rejects_dropped_relational_predicates_and_existential_witnesses() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); relation S(b:i32);
        r = rename R@0 {a -> x,b -> y}; s = rename S@1 {b -> y};
        j = semijoin r with s;
        f = select j if (x > 0, x = y);
        p = project f keep {x}; output p as answer(x);
    }};
    for operator in [
        syn::parse_quote!(select j if (x = y)),
        syn::parse_quote!(select j if (x > 0)),
    ] {
        let mut target = source.clone();
        target.plan.definitions[3].operator = operator;
        ra::contract::check(&target).unwrap();
        let error = ra::check_rewrite(&source, &target).unwrap_err();
        assert!(error.to_string().contains("source"), "{error}");
    }
    let mut target = source.clone();
    target.plan.definitions.remove(1);
    target.plan.definitions[1].operator = syn::parse_quote!(project r keep {x,y});
    ra::contract::check(&target).unwrap();
    let error = ra::check_rewrite(&source, &target).unwrap_err();
    assert!(error.to_string().contains("source"), "{error}");
}

#[test]
fn rewrite_rejects_reordered_or_changed_rust_payloads() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R@0 {a -> x};
        g = guard r if (first(x));
        h = guard g if (second(x)); output h as answer(x);
    }};
    let mut reordered = source.clone();
    reordered.plan.definitions[1].operator = syn::parse_quote!(guard r if (second(x)));
    reordered.plan.definitions[2].operator = syn::parse_quote!(guard g if (first(x)));
    let mut changed = source.clone();
    changed.plan.definitions[2].operator = syn::parse_quote!(guard g if (third(x)));
    for target in [reordered, changed] {
        ra::contract::check(&target).unwrap();
        let error = ra::check_rewrite(&source, &target).unwrap_err();
        assert!(error.to_string().contains("Rust chain"), "{error}");
    }
}

#[test]
fn rewrite_rejects_changed_bindings_even_when_projected_away() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R@0 {a -> x};
        b = bind r with y:i32 = (10 / *x);
        p = project b keep {x}; output p as answer(x);
    }};
    let mut target = source.clone();
    target.plan.definitions[1].operator = syn::parse_quote!(bind r with z:i32 = (10 / *x));
    ra::contract::check(&target).unwrap();
    let error = ra::check_rewrite(&source, &target).unwrap_err();
    assert!(error.to_string().contains("Rust chain"), "{error}");
}

#[test]
fn rewrite_relational_movement_across_rust_is_directional() {
    let earlier: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R@0 {a -> x};
        s = select r if (x > 0);
        g = guard s if (10 / *x > 0); output g as answer(x);
    }};
    let later: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R@0 {a -> x};
        g = guard r if (10 / *x > 0);
        s = select g if (x > 0); output s as answer(x);
    }};
    ra::contract::check(&earlier).unwrap();
    ra::contract::check(&later).unwrap();
    ra::check_rewrite(&later, &earlier).unwrap();
    let error = ra::check_rewrite(&earlier, &later).unwrap_err();
    assert!(error.to_string().contains("source-before"), "{error}");
}

#[test]
fn rewrite_checks_the_full_source_prefix_even_outside_rust_free_variables() {
    let earlier: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(b:i32);
        r = rename R@0 {a -> x}; s = rename S@1 {b -> y};
        j = natural_join r with s;
        g = guard j if (10 / *x > 0);
        p = project g keep {x}; output p as answer(x);
    }};
    let later: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(b:i32);
        r = rename R@0 {a -> x}; s = rename S@1 {b -> y};
        g = guard r if (10 / *x > 0);
        j = natural_join g with s;
        p = project j keep {x}; output p as answer(x);
    }};
    ra::contract::check(&later).unwrap();
    ra::check_rewrite(&later, &earlier).unwrap();
    let error = ra::check_rewrite(&earlier, &later).unwrap_err();
    assert!(error.to_string().contains("source-before"), "{error}");
}

#[test]
fn shared_projected_inputs_keep_independent_existential_witnesses() {
    let independent: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32, b:i32);
        r = rename R@0 {a -> x, b -> y};
        px = project r keep {x}; py = project r keep {y};
        j = natural_join px with py; output j as answer(x, y);
    }};
    let correlated: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32, b:i32);
        r = rename R@0 {a -> x, b -> y}; output r as answer(x, y);
    }};
    for (source, target) in [(&independent, &correlated), (&correlated, &independent)] {
        assert!(ra::check_rewrite(source, target).is_err());
    }
    ra::check_rewrite(&independent, &independent).unwrap();
}

#[test]
fn occurrences_are_required_unique_numeric_identities_and_round_trip() {
    let program: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(a:i32);
        r = rename R@7 {a -> x}; s = rename S@12 {a -> x};
        j = natural_join r with s; output j as answer(x);
    }};
    ra::contract::check(&program).unwrap();
    assert_eq!(
        program,
        syn::parse2::<ra::Program>(quote!(#program)).unwrap()
    );
    for occurrence in [syn::parse_quote!(7), syn::parse_quote!(0x7)] {
        let mut duplicate = program.clone();
        let ra::Operator::Rename(rename) = &mut duplicate.plan.definitions[1].operator else {
            unreachable!()
        };
        rename.occurrence = occurrence;
        assert!(
            ra::contract::check(&duplicate)
                .unwrap_err()
                .to_string()
                .contains("occurrence")
        );
        assert!(ra::check_rewrite(&program, &duplicate).is_err());
        assert!(ra::check_rewrite(&duplicate, &program).is_err());
    }
    assert!(
        syn::parse2::<ra::Program>(quote! { relational {
            relation R(a:i32); r = rename R {a -> x}; output r as answer(x);
        }})
        .is_err()
    );
    let overflow: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R@18446744073709551616 {a -> x}; output r as answer(x);
    }};
    assert!(ra::contract::check(&overflow).is_err());
}

#[test]
fn occurrence_and_atom_bindings_cannot_change_even_for_existential_columns() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32);
        r = rename R@4 {a -> x,b -> y}; p = project r keep {x}; output p as answer(x);
    }};
    for rename in [
        syn::parse_quote!(rename R@5 {a -> x,b -> y}),
        syn::parse_quote!(rename R@4 {a -> x,b -> z}),
        syn::parse_quote!(rename R@4 {a -> y,b -> x}),
    ] {
        let mut target = source.clone();
        target.plan.definitions[0].operator = rename;
        ra::contract::check(&target).unwrap();
        let error = ra::check_rewrite(&source, &target).unwrap_err();
        assert!(error.to_string().contains("occurrence"), "{error}");
    }
}

#[test]
fn result_renames_and_earlier_selection_and_semijoin_are_accepted() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(a:i32);
        r = rename R@4 {a -> x}; s = rename S@8 {a -> x};
        g = guard r if (opaque(x)); j = semijoin g with s;
        f = select j if (x = 1); output f as answer(x);
    }};
    let target: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(a:i32);
        support = rename S@8 {a -> x}; rows = rename R@4 {a -> x};
        matched = semijoin rows with support;
        filtered = select matched if (x = 1);
        result = guard filtered if (opaque(x)); output result as answer(x);
    }};
    ra::check_rewrite(&source, &target).unwrap();
    assert!(
        ra::check_rewrite(&target, &source)
            .unwrap_err()
            .to_string()
            .contains("source-before")
    );
}

#[test]
fn equality_edges_are_compared_after_transitive_closure() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32,c:i32);
        r = rename R@0 {a -> x,b -> y,c -> z};
        f = select r if (x = y, y = z, x > 0);
        p = project f keep {x}; output p as answer(x);
    }};
    let mut target = source.clone();
    target.plan.definitions[1].operator = syn::parse_quote!(select r if (z = y, x = z, z > 0));
    ra::check_rewrite(&source, &target).unwrap();
    ra::check_rewrite(&target, &source).unwrap();
    target.plan.definitions[1].operator = syn::parse_quote!(select r if (z = y, x > 0));
    assert!(
        ra::check_rewrite(&source, &target)
            .unwrap_err()
            .to_string()
            .contains("equality closure")
    );
}

#[test]
fn a_later_equality_cannot_repair_an_earlier_predicate_binding() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(b:i32);
        r = rename R@0 {a -> x}; s = rename S@1 {b -> y};
        j = natural_join r with s; f = select j if (x = 1);
        g = guard f if (opaque()); e = select g if (x = y);
        p = project e keep {x}; output p as answer(x);
    }};
    let mut target = source.clone();
    target.plan.definitions[3].operator = syn::parse_quote!(select j if (y = 1));
    ra::contract::check(&target).unwrap();
    assert!(
        ra::check_rewrite(&source, &target)
            .unwrap_err()
            .to_string()
            .contains("source-before")
    );
}

#[test]
fn before_is_dag_ancestry_even_when_a_selection_is_written_early() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R@0 {a -> x};
        f = select r if (x = 1); g = guard f if (opaque(x)); output g as answer(x);
    }};
    let target: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R@0 {a -> x};
        f = select r if (x = 1); g = guard r if (opaque(x));
        j = natural_join g with f; output j as answer(x);
    }};
    ra::contract::check(&target).unwrap();
    assert!(
        ra::check_rewrite(&source, &target)
            .unwrap_err()
            .to_string()
            .contains("source-before")
    );
}

#[test]
fn output_layout_declarations_and_chain_length_are_contracts() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R@0 {a -> x,b -> y};
        g = guard r if (opaque(x)); output g as answer(x,y);
    }};
    let mut head = source.clone();
    head.plan.output.head = syn::parse_quote!(answer(y, x));
    let mut declaration = source.clone();
    declaration.plan.declarations[0].columns[0].ty = syn::parse_quote!(u32);
    let mut chain = source.clone();
    chain.plan.definitions[1].operator = syn::parse_quote!(project r keep {x,y});
    for target in [head, declaration, chain] {
        ra::contract::check(&target).unwrap();
        assert!(ra::check_rewrite(&source, &target).is_err());
    }
    let mut malformed = source.clone();
    malformed.plan.output.input = syn::parse_quote!(missing);
    assert!(
        ra::check_rewrite(&source, &malformed)
            .unwrap_err()
            .to_string()
            .contains("not defined")
    );
    assert!(
        ra::check_rewrite(&malformed, &source)
            .unwrap_err()
            .to_string()
            .contains("not defined")
    );
}

#[test]
fn repeated_shared_dag_operands_do_not_create_new_occurrences() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R@0 {a -> x,b -> y}; output r as answer(x,y);
    }};
    let mut target = source.clone();
    let mut previous = target.plan.output.input.clone();
    for index in 0..2048 {
        let result = quote::format_ident!("shared{index}");
        target
            .plan
            .definitions
            .push(syn::parse_quote!(#result = semijoin #previous with #previous;));
        previous = result;
    }
    target.plan.output.input = previous;
    ra::check_rewrite(&source, &target).unwrap();
    ra::check_rewrite(&target, &source).unwrap();
}

#[test]
fn predicates_belong_to_their_independent_projected_witnesses() {
    let independent: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R@0 {a -> x,b -> y};
        px = project r keep {x}; py = project r keep {y};
        filtered = select py if (y = 1);
        j = semijoin px with filtered; output j as answer(x);
    }};
    let correlated: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R@0 {a -> x,b -> y};
        filtered = select r if (y = 1); p = project filtered keep {x}; output p as answer(x);
    }};
    ra::check_rewrite(&independent, &independent).unwrap();
    for (source, target) in [(&independent, &correlated), (&correlated, &independent)] {
        ra::contract::check(target).unwrap();
        assert!(
            ra::check_rewrite(source, target)
                .unwrap_err()
                .to_string()
                .contains("projection correlations")
        );
    }
    let left: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R@0 {a -> x,b -> y};
        f = select r if (y = 1); px = project f keep {x}; py = project r keep {y};
        j = natural_join px with py; output j as answer(x,y);
    }};
    let right: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R@0 {a -> x,b -> y};
        f = select r if (y = 1); px = project r keep {x}; py = project f keep {y};
        j = natural_join px with py; output j as answer(x,y);
    }};
    assert!(ra::check_rewrite(&left, &right).is_err());
    assert!(ra::check_rewrite(&right, &left).is_err());
    let hidden: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R@0 {a -> x,b -> y};
        px = project r keep {x}; f = select r if (y = 1); existence = project f keep {};
        j = semijoin px with existence; output j as answer(x);
    }};
    assert!(ra::check_rewrite(&hidden, &correlated).is_err());
    let subset: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R@0 {a -> x,b -> y};
        f = select r if (y = 1); px = project f keep {x};
        j = semijoin r with px; output j as answer(x,y);
    }};
    let filtered: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R@0 {a -> x,b -> y};
        f = select r if (y = 1); output f as answer(x,y);
    }};
    assert!(ra::check_rewrite(&subset, &filtered).is_err());
}

#[test]
fn split_witness_wiring_accepts_local_semijoin_identities() {
    let compact: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R@0 {a -> x,b -> y};
        px = project r keep {x}; py = project r keep {y};
        filtered = select py if (y = 1);
        j = semijoin px with filtered; output j as answer(x);
    }};
    let expanded: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R@0 {a -> x,b -> y};
        px = project r keep {x}; py = project r keep {y};
        filtered = select py if (y = 1);
        identity = select filtered if (y = y);
        j = natural_join px with identity;
        p = project j keep {x}; output p as answer(x);
    }};
    let projected: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R@0 {a -> x,b -> y};
        px = project r keep {x}; py = project r keep {y};
        filtered = select py if (y = 1);
        exists = project filtered keep {};
        j = natural_join px with exists;
        p = project j keep {x}; output p as answer(x);
    }};
    for target in [&expanded, &projected] {
        ra::check_rewrite(&compact, target).unwrap();
        ra::check_rewrite(target, &compact).unwrap();
    }
}

#[test]
fn semijoin_wiring_cannot_restore_a_hidden_shared_support_column() {
    let independent: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R@0 {a -> x,b -> y};
        px = project r keep {x}; filtered = select r if (y = 1);
        exists = project filtered keep {};
        j = semijoin px with exists; output j as answer(x);
    }};
    let correlated: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R@0 {a -> x,b -> y};
        px = project r keep {x}; filtered = select r if (y = 1);
        j = semijoin px with filtered; output j as answer(x);
    }};
    ra::check_rewrite(&independent, &independent).unwrap();
    ra::check_rewrite(&correlated, &correlated).unwrap();
    for (source, target) in [(&independent, &correlated), (&correlated, &independent)] {
        assert!(
            ra::check_rewrite(source, target)
                .unwrap_err()
                .to_string()
                .contains("projection correlations")
        );
    }
}

#[test]
fn dead_expansion_cannot_erase_a_downstream_implicit_join_key() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(a:i32,b:i32); relation T(b:i32);
        r = rename R@0 {a -> x};
        s = rename S@1 {a -> x,b -> y};
        t = rename T@2 {b -> y};
        expansion = natural_join r with s;
        later = natural_join expansion with t;
        answer_rows = project later keep {x};
        output answer_rows as answer(x);
    }};
    for later in [
        syn::parse_quote!(natural_join expansion with t),
        syn::parse_quote!(semijoin expansion with t),
    ] {
        let mut source = source.clone();
        source.plan.definitions[4].operator = later;
        let mut target = source.clone();
        target.plan.definitions[3].operator = syn::parse_quote!(semijoin r with s);
        // Both target operators remain locally well headed, but replacing
        // the expansion removes the later implicit equality on y. Witnesses
        // S(x, y1) and T(y2) must not become independently existential.
        ra::contract::check(&source).unwrap();
        ra::contract::check(&target).unwrap();
        assert!(ra::check_rewrite(&source, &target).is_err());
        let schedule = ra::contract::Schedule::from_program(&source).unwrap();
        assert!(schedule.check_dead_expansions(&target, &[]).is_err());
    }
}

#[test]
fn inverse_dead_expansion_restores_only_a_checked_primary_witness() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(a:i32,b:i32); relation T(b:i32);
        r = rename R@0 {a -> x};
        s = rename S@1 {a -> x,b -> y};
        t = rename T@2 {b -> y};
        support = semijoin s with t;
        lookup = semijoin r with support;
        expansion = natural_join lookup with support;
        answer_rows = project expansion keep {x};
        output answer_rows as answer(x);
    }};
    let target: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(a:i32,b:i32); relation T(b:i32);
        r = rename R@0 {a -> x};
        s = rename S@1 {a -> x,b -> y};
        t = rename T@2 {b -> y};
        support = semijoin s with t;
        lookup = semijoin r with support;
        answer_rows = project lookup keep {x};
        output answer_rows as answer(x);
    }};
    let schedule = ra::contract::Schedule::from_program(&source).unwrap();
    // The original s-with-t reduction remains required even though its
    // equalities need a certified primary expansion to become visible.
    schedule.check_dead_expansions(&target, &[(1, 2)]).unwrap();
    ra::check_rewrite(&source, &target).unwrap();
}

#[test]
fn final_equality_does_not_deduplicate_distinct_before_constraints() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R@0 {a -> x,b -> y};
        f = select r if (x = 1, y = 1); g = guard f if (opaque());
        e = select g if (x = y); p = project e keep {x}; output p as answer(x);
    }};
    let target: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R@0 {a -> x,b -> y};
        f = select r if (x = 1); g = guard f if (opaque());
        e = select g if (x = y, y = 1); p = project e keep {x}; output p as answer(x);
    }};
    assert!(
        ra::check_rewrite(&source, &target)
            .unwrap_err()
            .to_string()
            .contains("source-before")
    );
    ra::check_rewrite(&target, &source).unwrap();
    let mut earlier = target.clone();
    // The existing x constraint is retained even if an additional y predicate
    // is serialized first and only joins x's equality class after Rust.
    earlier.plan.definitions[1].operator = syn::parse_quote!(select r if (y = 1, x = 1));
    ra::check_rewrite(&target, &earlier).unwrap();
}

#[test]
fn a_projected_semijoin_cannot_rebind_the_supports_hidden_columns() {
    let independent: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(a:i32,b:i32);
        r = rename R@0 {a -> x}; s = rename S@1 {a -> x,b -> y};
        matched = semijoin r with s; py = project s keep {y};
        j = natural_join matched with py; output j as answer(x,y);
    }};
    let correlated: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(a:i32,b:i32);
        r = rename R@0 {a -> x}; s = rename S@1 {a -> x,b -> y};
        j = natural_join r with s; output j as answer(x,y);
    }};
    ra::check_rewrite(&independent, &independent).unwrap();
    assert!(ra::check_rewrite(&independent, &correlated).is_err());
    assert!(ra::check_rewrite(&correlated, &independent).is_err());
}

#[test]
fn a_cut_of_a_joined_prefix_keeps_its_witness_scope() {
    let independent: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); relation S(b:i32);
        r = rename R@0 {a -> x,b -> y}; s = rename S@1 {b -> y};
        j = natural_join r with s; exists = project j keep {}; px = project r keep {x};
        out = natural_join px with exists; output out as answer(x);
    }};
    let correlated: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); relation S(b:i32);
        r = rename R@0 {a -> x,b -> y}; s = rename S@1 {b -> y};
        j = natural_join r with s; out = project j keep {x}; output out as answer(x);
    }};
    ra::check_rewrite(&independent, &independent).unwrap();
    assert!(ra::check_rewrite(&independent, &correlated).is_err());
    assert!(ra::check_rewrite(&correlated, &independent).is_err());
}

#[test]
fn before_constraints_use_current_equality_classes_among_all_aliases() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32,c:i32); r = rename R@0 {a -> x,b -> y,c -> z};
        f = select r if (x = z, z = 1); g = guard f if (opaque());
        e = select g if (x = y, y = 1); p = project e keep {x}; output p as answer(x);
    }};
    let target: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32,c:i32); r = rename R@0 {a -> x,b -> y,c -> z};
        f = select r if (x = z, y = 1, x = 1); g = guard f if (opaque());
        e = select g if (x = y); p = project e keep {x}; output p as answer(x);
    }};
    ra::check_rewrite(&source, &target).unwrap();
    assert!(
        ra::check_rewrite(&target, &source)
            .unwrap_err()
            .to_string()
            .contains("source-before")
    );
}

#[test]
fn reducer_column_matches_do_not_replace_a_before_generator_equality() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(b:i32);
        r = rename R@0 {a -> x}; s = rename S@1 {b -> y};
        j = natural_join r with s; e = select j if (x = y);
        g = guard e if (*x == *y || panic!()); output g as answer(x,y);
    }};
    let target: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(b:i32);
        r = rename R@0 {a -> x}; s = rename S@1 {b -> y};
        rs = natural_join r with s; re = select rs if (x = y); rr = project re keep {x};
        sr = natural_join s with r; se = select sr if (x = y); ss = project se keep {y};
        j = natural_join rr with ss;
        g = guard j if (*x == *y || panic!()); e = select g if (x = y);
        output e as answer(x,y);
    }};
    ra::contract::check(&target).unwrap();
    assert!(
        ra::check_rewrite(&source, &target)
            .unwrap_err()
            .to_string()
            .contains("source-before")
    );
}

#[test]
fn generator_bind_round_trip_freshness_and_structural_mode() {
    let source: ra::Program = syn::parse_quote! { relational {
        u = unit;
        g = bind u with for x = ([1, 1, 2]);
        output g as answer(x);
    } };
    let facts = ra::contract::check(&source).unwrap();
    assert!(facts["g"].computed.contains("x"));
    assert_eq!(
        syn::parse2::<ra::Program>(source.to_token_stream()).unwrap(),
        source
    );
    let mut scalar = source.clone();
    let ra::Operator::Bind(bind) = &mut scalar.plan.definitions[1].operator else {
        panic!()
    };
    bind.for_token = None;
    ra::contract::check(&scalar).unwrap();
    assert!(ra::check_rewrite(&source, &scalar).is_err());
    for replacement in [
        "bind u with for x = ([*x])",
        "bind absent with for x = ([1])",
    ] {
        let mut invalid = source.clone();
        invalid.plan.definitions[1].operator = syn::parse_str(replacement).unwrap();
        assert!(ra::contract::check(&invalid).is_err());
    }
    let duplicate: ra::Program = syn::parse_quote! { relational {
        u = unit;
        a = bind u with x = (1);
        g = bind a with for x = ([1]);
        output g as answer(x);
    } };
    assert!(ra::contract::check(&duplicate).is_err());
}

#[test]
fn generator_before_facts_are_structural() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32);
        r = rename R@0 {a -> x};
        s = select r if (x = 1);
        g = bind s with for y = ([*x]);
        output g as answer(x,y);
    } };
    let moved: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32);
        r = rename R@0 {a -> x};
        g = bind r with for y = ([*x]);
        s = select g if (x = 1);
        output s as answer(x,y);
    } };
    assert!(ra::check_rewrite(&source, &moved).is_err());
}

#[test]
fn occurrence_constants_roundtrip_drop_columns_and_preserve_source_types() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:bool,c:i64);
        r = rename R@0 {c -> x} constants {a = -7, b = true};
        output r as answer(x);
    }};
    let facts = ra::contract::check(&source).unwrap();
    assert_eq!(facts["r"].heading, ["x".into()].into());
    assert_eq!(facts["r"].types["x"], syn::parse_quote!(i64));
    assert_eq!(
        syn::parse2::<ra::Program>(source.to_token_stream()).unwrap(),
        source
    );
    let all: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32);
        r = rename R@0 {} constants {a = 7};
        output r as answer();
    }};
    assert!(ra::contract::check(&all).unwrap()["r"].heading.is_empty());
}

#[test]
fn occurrence_constants_require_an_exact_literal_source_partition() {
    for operator in [
        quote!(rename R@0 {a -> x} constants {}),
        quote!(rename R@0 {a -> x} constants {b = x}),
        quote!(rename R@0 {a -> x} constants {b = 7, b = 8}),
        quote!(rename R@0 {a -> x} constants {a = 7}),
        quote!(rename R@0 {a -> x} constants {missing = 7}),
        quote!(rename R@0 {} constants {b = 7}),
    ] {
        assert!(
            check(quote! { relational {
                relation R(a:i32,b:i32);
                r = #operator;
                output r as answer();
            }})
            .is_err()
        );
    }
    assert!(
        check(quote! { relational {
            relation R(a:i32,b:f64);
            r = rename R@0 {a -> x} constants {b = 1.0};
            output r as answer(x);
        }})
        .is_err()
    );
}

#[test]
fn rewrites_preserve_occurrence_constants_without_inventing_union_disjointness() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32);
        r = rename R@0 {a -> x} constants {b = 7};
        s = rename R@1 {a -> x} constants {b = 8};
        u = union r, s;
        output u as answer(x);
    }};
    ra::contract::check(&source).unwrap();
    assert!(ra::disjoint_unions(&source).unwrap().is_empty());
    let mut changed = source.clone();
    if let ra::Operator::Rename(rename) = &mut changed.plan.definitions[0].operator {
        rename.constants = Some(syn::parse_quote!(constants {b = 9}));
    }
    ra::contract::check(&changed).unwrap();
    assert!(ra::check_rewrite(&source, &changed).is_err());
}
