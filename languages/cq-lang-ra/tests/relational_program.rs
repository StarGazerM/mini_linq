use cq_lang_ra as ra;
use quote::{ToTokens, quote};

fn two_hop() -> proc_macro2::TokenStream {
    quote! {
            relational {
    relation road(src: i32, dst: i32);

                r0 = rename road {src -> from, dst -> via};
                r1 = rename road {src -> via, dst -> to};
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

                source = rename R {value -> x};
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

                from_other_declared_input = rename S {value -> x};
                output from_other_declared_input as answer(x);
            }
        })
    .unwrap();

    check(quote! {
            relational {
    relation R(value: i32);
            relation S(value: i32);

                second = rename S {value -> y};
                first = rename R {value -> x};
                both = natural_join second with first;
                projected = project both keep {x, y};
                output projected as answer(x, y);
            }
        })
    .unwrap();
}

#[test]
fn rename_requires_a_declared_relation_and_exact_one_to_one_domain() {
    check(quote! {
            relational {
    relation R(value: i32);

                r0 = rename Missing {value -> x};
                output r0 as answer(x);
            }
        })
    .unwrap_err();

    for invalid_mapping in [
        quote!(c0 -> x),
        quote!(c0 -> x, c0 -> y),
        quote!(c0 -> x, c1 -> x),
    ] {
        check(quote! {
                    relational {
        relation R(c0: i32, c1: i32);

                        r0 = rename R {#invalid_mapping};
                        r1 = project r0 keep {x};
                        output r1 as answer(x);
                    }
                })
        .unwrap_err();
    }
}

#[test]
fn natural_join_infers_union_heading_without_resolving_rust_types() {
    check(quote! {
            relational {
    relation People(id: PersonId, manager: PersonId);
            relation Logins(person: u64, day: u32);

                people = rename People {id -> x, manager -> manager};
                logins = rename Logins {person -> x, day -> day};
                joined = natural_join people with logins;
                result = project joined keep {x, day};
                output result as answer(x, day);
            }
        })
    .unwrap();
}

#[test]
fn cross_type_join_drops_ambiguous_codec_identity() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a: HostType [7], b: HostType [8]);
        relation S(a: OtherType [9], c: OtherType [10]);
        r = rename R {a -> x, b -> y};
        s = rename S {a -> x, c -> z};
        joined = natural_join r with s;
        output joined as answer(x, y, z);
    }};
    let facts = ra::contract::check(&source).unwrap();
    assert!(!facts["joined"].logical_type_ids.contains_key("x"));
    assert_eq!(
        facts["joined"].logical_type_ids["y"],
        ra::syntax::LogicalTypeId(8)
    );
    assert_eq!(
        facts["joined"].logical_type_ids["z"],
        ra::syntax::LogicalTypeId(10)
    );
}

#[test]
fn natural_join_with_disjoint_headings_is_a_cartesian_product() {
    check(quote! {
            relational {
    relation R(value: i32);
            relation S(value: i32);

                r = rename R {value -> x};
                s = rename S {value -> y};
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

                rows = rename R {first -> x, second -> z};
                result = project rows keep {x, z};
                output result as answer(z, x);
            }
        })
    .unwrap();

    for attributes in [quote!(x, x), quote!(missing)] {
        check(quote! {
                    relational {
        relation R(value: i32);

                        rows = rename R {value -> x};
                        result = project rows keep {#attributes};
                        output result as answer(x);
                    }
                })
        .unwrap_err();
    }
}

#[test]
fn output_must_match_its_input_heading() {
    check(quote! {
            relational {
    relation R(first: i32, second: i32);

                rows = rename R {first -> x, second -> y};
                output rows as answer(x);
            }
        })
    .unwrap_err();
}

#[test]
fn definitions_are_unique_and_operands_must_be_earlier() {
    check(quote! {
            relational {
    relation R(value: i32);

                r0 = rename R {value -> x};
                r0 = project r0 keep {x};
                output r0 as answer(x);
            }
        })
    .unwrap_err();

    check(quote! {
            relational {
    relation R(value: i32);

                result = project rows keep {x};
                rows = rename R {value -> x};
                output result as answer(x);
            }
        })
    .unwrap_err();
}

#[test]
fn every_definition_must_contribute_to_output() {
    check(quote! {
            relational {
    relation R(value: i32);
            relation S(value: i32);

                used = rename R {value -> x};
                unused = rename S {value -> y};
                output used as answer(x);
            }
        })
    .unwrap_err();
}

#[test]
fn unit_is_the_zero_attribute_relation() {
    check(quote! {
            relational {
    relation R(value: i32);

                seed = unit;
                rows = rename R {value -> x};
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
                r0 = rename R {a -> x, b -> y};
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
        r = rename R {a -> x,b -> y}; p = project r keep {x};
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
        "relational { relation R(a: i32); r = rename R {a -> x}; }",
        "relational { relation R(a: i32); r = rename R {a -> x}; output r as answer(x); junk }",
        "relational { relation R(a: i32); r = rename R {a x}; output r as answer(x); }",
    ] {
        assert!(syn::parse_str::<ra::Program>(text).is_err(), "{text}");
    }
}

#[test]
fn relational_selection_has_only_column_equalities_and_literals() {
    check(quote! { relational {
        relation R(a: i32, b: i32);
        r = rename R {a -> x, b -> y};
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
                relation R(a: i32); r = rename R {a -> x};
                s = select r if (#condition); output s as answer(x);
            }})
            .is_err()
        );
    }
    assert!(
        syn::parse2::<ra::Program>(quote! { relational {
            relation R(a: i32); r = rename R {a -> x};
            s = extend r with {y:i32} value (1); output s as answer(x, y);
        }})
        .is_err()
    );
}

#[test]
fn independent_rust_nodes_need_only_their_free_variables() {
    check(quote! { relational {
        relation R(a: i32); relation S(b: i32);
        r = rename R {a -> x};
        g = guard r if (*x != 0);
        s = rename S {b -> w};
        j = natural_join g with s;
        b = bind j with y: i32 = (10 / (*x).max(1));
        p = project b keep {x, y}; output p as answer(x, y);
    }})
    .unwrap();
    check(quote! { relational {
        relation R(a: i32); relation S(b: i32);
        r = rename R {a -> x}; s = rename S {b -> w};
        g = guard r if (*x != 0);
        b = bind s with y: i32 = (10 / (*w).max(1));
        j = natural_join g with b;
        output j as answer(x, w, y);
    }})
    .unwrap();
}

#[test]
fn selection_scope_is_checked_but_host_trait_bounds_are_left_to_rustc() {
    check(quote! { relational {
        relation R(a: i32); r = rename R {a -> x};
        s = select r if (y = 1); output s as answer(x);
    }})
    .unwrap_err();
    for program in [
        quote! { relational {
            relation R(a: f64); r = rename R {a -> x};
            s = select r if (x = 1.0); output s as answer(x);
        }},
        quote! { relational {
            relation R(a: f64); relation S(a: f64);
            r = rename R {a -> x}; s = rename S {a -> x};
            j = natural_join r with s; p = project j keep {}; output p as answer();
        }},
    ] {
        check(program).unwrap();
    }
}

#[test]
fn rewrite_accepts_join_reordering_and_reassociation_with_existential_columns() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); relation S(b:i32,c:i32); relation T(c:i32,d:i32);
        r = rename R {a -> x, b -> y};
        s = rename S {b -> y, c -> z};
        t = rename T {c -> z, d -> w};
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
        r = rename R {a -> x,b -> y}; s = rename S {b -> y};
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
        assert!(ra::check_rewrite(&source, &target).is_err());
    }
    let mut target = source.clone();
    target.plan.definitions.remove(1);
    target.plan.definitions[1].operator = syn::parse_quote!(project r keep {x,y});
    ra::contract::check(&target).unwrap();
    assert!(ra::check_rewrite(&source, &target).is_err());
}

#[test]
fn rewrite_accepts_reordered_but_rejects_changed_rust_payloads() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R {a -> x};
        g = guard r if (first(x));
        h = guard g if (second(x)); output h as answer(x);
    }};
    let mut reordered = source.clone();
    reordered.plan.definitions[1].operator = syn::parse_quote!(guard r if (second(x)));
    reordered.plan.definitions[2].operator = syn::parse_quote!(guard g if (first(x)));
    let mut changed = source.clone();
    changed.plan.definitions[2].operator = syn::parse_quote!(guard g if (third(x)));
    ra::contract::check(&reordered).unwrap();
    ra::check_rewrite(&source, &reordered).unwrap();
    ra::contract::check(&changed).unwrap();
    assert!(ra::check_rewrite(&source, &changed).is_err());
}

#[test]
fn independent_rust_bindings_keep_identity_when_reordered() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R {a -> x};
        a = bind r with y:i32 = (*x + 1);
        b = bind a with z:i32 = (*x + 2);
        output b as answer(x,y,z);
    }};
    let target: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R {a -> x};
        b = bind r with z:i32 = (*x + 2);
        a = bind b with y:i32 = (*x + 1);
        output a as answer(x,y,z);
    }};
    ra::check_rewrite(&source, &target).unwrap();
    ra::check_rewrite(&target, &source).unwrap();
}

#[test]
fn rewrite_rejects_changed_bindings_even_when_projected_away() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R {a -> x};
        b = bind r with y:i32 = (10 / *x);
        p = project b keep {x}; output p as answer(x);
    }};
    let mut target = source.clone();
    target.plan.definitions[1].operator = syn::parse_quote!(bind r with z:i32 = (10 / *x));
    ra::contract::check(&target).unwrap();
    assert!(ra::check_rewrite(&source, &target).is_err());
}

#[test]
fn relational_selection_can_cross_total_rust_in_either_direction() {
    let earlier: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R {a -> x};
        s = select r if (x > 0);
        g = guard s if (10 / (*x).max(1) > 0); output g as answer(x);
    }};
    let later: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R {a -> x};
        g = guard r if (10 / (*x).max(1) > 0);
        s = select g if (x > 0); output s as answer(x);
    }};
    ra::contract::check(&earlier).unwrap();
    ra::contract::check(&later).unwrap();
    ra::check_rewrite(&later, &earlier).unwrap();
    ra::check_rewrite(&earlier, &later).unwrap();
}

#[test]
fn relational_atoms_can_cross_rust_without_free_variable_dependencies() {
    let earlier: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(b:i32);
        r = rename R {a -> x}; s = rename S {b -> y};
        j = natural_join r with s;
        g = guard j if (10 / (*x).max(1) > 0);
        p = project g keep {x}; output p as answer(x);
    }};
    let later: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(b:i32);
        r = rename R {a -> x}; s = rename S {b -> y};
        g = guard r if (10 / (*x).max(1) > 0);
        j = natural_join g with s;
        p = project j keep {x}; output p as answer(x);
    }};
    ra::contract::check(&later).unwrap();
    ra::check_rewrite(&later, &earlier).unwrap();
    ra::check_rewrite(&earlier, &later).unwrap();
}

#[test]
fn shared_projected_inputs_keep_independent_existential_witnesses() {
    let independent: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32, b:i32);
        r = rename R {a -> x, b -> y};
        px = project r keep {x}; py = project r keep {y};
        j = natural_join px with py; output j as answer(x, y);
    }};
    let correlated: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32, b:i32);
        r = rename R {a -> x, b -> y}; output r as answer(x, y);
    }};
    for (source, target) in [(&independent, &correlated), (&correlated, &independent)] {
        assert!(ra::check_rewrite(source, target).is_err());
    }
    ra::check_rewrite(&independent, &independent).unwrap();
}

#[test]
fn atom_bindings_cannot_change_even_for_existential_columns() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32);
        r = rename R {a -> x,b -> y}; p = project r keep {x}; output p as answer(x);
    }};
    for rename in [
        syn::parse_quote!(rename R {a -> x,b -> z}),
        syn::parse_quote!(rename R {a -> y,b -> x}),
    ] {
        let mut target = source.clone();
        target.plan.definitions[0].operator = rename;
        ra::contract::check(&target).unwrap();
        assert!(ra::check_rewrite(&source, &target).is_err());
    }
}

#[test]
fn result_renames_and_earlier_selection_and_semijoin_are_accepted() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(a:i32);
        r = rename R {a -> x}; s = rename S {a -> x};
        g = guard r if (opaque(x)); j = semijoin g with s;
        f = select j if (x = 1); output f as answer(x);
    }};
    let target: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(a:i32);
        support = rename S {a -> x}; rows = rename R {a -> x};
        matched = semijoin rows with support;
        filtered = select matched if (x = 1);
        result = guard filtered if (opaque(x)); output result as answer(x);
    }};
    ra::check_rewrite(&source, &target).unwrap();
    ra::check_rewrite(&target, &source).unwrap();
}

#[test]
fn equality_edges_are_compared_after_transitive_closure() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32,c:i32);
        r = rename R {a -> x,b -> y,c -> z};
        f = select r if (x = y, y = z, x > 0);
        p = project f keep {x}; output p as answer(x);
    }};
    let mut target = source.clone();
    target.plan.definitions[1].operator = syn::parse_quote!(select r if (z = y, x = z, z > 0));
    ra::check_rewrite(&source, &target).unwrap();
    ra::check_rewrite(&target, &source).unwrap();
    target.plan.definitions[1].operator = syn::parse_quote!(select r if (z = y, x > 0));
    assert!(ra::check_rewrite(&source, &target).is_err());
}

#[test]
fn total_rust_allows_equality_closure_across_predicate_placement() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(b:i32);
        r = rename R {a -> x}; s = rename S {b -> y};
        j = natural_join r with s; f = select j if (x = 1);
        g = guard f if (opaque()); e = select g if (x = y);
        p = project e keep {x}; output p as answer(x);
    }};
    let mut target = source.clone();
    target.plan.definitions[3].operator = syn::parse_quote!(select j if (y = 1));
    ra::contract::check(&target).unwrap();
    ra::check_rewrite(&source, &target).unwrap();
}

#[test]
fn selection_can_follow_rust_through_shared_dag_ancestry() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R {a -> x};
        f = select r if (x = 1); g = guard f if (opaque(x)); output g as answer(x);
    }};
    let target: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R {a -> x};
        f = select r if (x = 1); g = guard r if (opaque(x));
        j = natural_join g with f; output j as answer(x);
    }};
    ra::contract::check(&target).unwrap();
    ra::check_rewrite(&source, &target).unwrap();
}

#[test]
fn output_layout_declarations_and_rust_occurrences_are_contracts() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R {a -> x,b -> y};
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
    assert!(ra::check_rewrite(&source, &malformed).is_err());
    assert!(ra::check_rewrite(&malformed, &source).is_err());
}

#[test]
fn repeated_shared_dag_operands_do_not_create_new_occurrences() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R {a -> x,b -> y}; output r as answer(x,y);
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
        relation R(a:i32,b:i32); r = rename R {a -> x,b -> y};
        px = project r keep {x}; py = project r keep {y};
        filtered = select py if (y = 1);
        j = semijoin px with filtered; output j as answer(x);
    }};
    let correlated: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R {a -> x,b -> y};
        filtered = select r if (y = 1); p = project filtered keep {x}; output p as answer(x);
    }};
    ra::check_rewrite(&independent, &independent).unwrap();
    for (source, target) in [(&independent, &correlated), (&correlated, &independent)] {
        ra::contract::check(target).unwrap();
        assert!(ra::check_rewrite(source, target).is_err());
    }
    let left: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R {a -> x,b -> y};
        f = select r if (y = 1); px = project f keep {x}; py = project r keep {y};
        j = natural_join px with py; output j as answer(x,y);
    }};
    let right: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R {a -> x,b -> y};
        f = select r if (y = 1); px = project r keep {x}; py = project f keep {y};
        j = natural_join px with py; output j as answer(x,y);
    }};
    assert!(ra::check_rewrite(&left, &right).is_err());
    assert!(ra::check_rewrite(&right, &left).is_err());
    let hidden: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R {a -> x,b -> y};
        px = project r keep {x}; f = select r if (y = 1); existence = project f keep {};
        j = semijoin px with existence; output j as answer(x);
    }};
    assert!(ra::check_rewrite(&hidden, &correlated).is_err());
    let subset: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R {a -> x,b -> y};
        f = select r if (y = 1); px = project f keep {x};
        j = semijoin r with px; output j as answer(x,y);
    }};
    let filtered: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R {a -> x,b -> y};
        f = select r if (y = 1); output f as answer(x,y);
    }};
    assert!(ra::check_rewrite(&subset, &filtered).is_err());
}

#[test]
fn split_witness_wiring_accepts_local_semijoin_identities() {
    let compact: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R {a -> x,b -> y};
        px = project r keep {x}; py = project r keep {y};
        filtered = select py if (y = 1);
        j = semijoin px with filtered; output j as answer(x);
    }};
    let expanded: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R {a -> x,b -> y};
        px = project r keep {x}; py = project r keep {y};
        filtered = select py if (y = 1);
        identity = select filtered if (y = y);
        j = natural_join px with identity;
        p = project j keep {x}; output p as answer(x);
    }};
    let projected: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R {a -> x,b -> y};
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
        relation R(a:i32,b:i32); r = rename R {a -> x,b -> y};
        px = project r keep {x}; filtered = select r if (y = 1);
        exists = project filtered keep {};
        j = semijoin px with exists; output j as answer(x);
    }};
    let correlated: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R {a -> x,b -> y};
        px = project r keep {x}; filtered = select r if (y = 1);
        j = semijoin px with filtered; output j as answer(x);
    }};
    ra::check_rewrite(&independent, &independent).unwrap();
    ra::check_rewrite(&correlated, &correlated).unwrap();
    for (source, target) in [(&independent, &correlated), (&correlated, &independent)] {
        assert!(ra::check_rewrite(source, target).is_err());
    }
}

#[test]
fn dead_expansion_cannot_erase_a_downstream_implicit_join_key() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(a:i32,b:i32); relation T(b:i32);
        r = rename R {a -> x};
        s = rename S {a -> x,b -> y};
        t = rename T {b -> y};
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
    }
}

#[test]
fn inverse_dead_expansion_restores_only_a_checked_primary_witness() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(a:i32,b:i32); relation T(b:i32);
        r = rename R {a -> x};
        s = rename S {a -> x,b -> y};
        t = rename T {b -> y};
        support = semijoin s with t;
        lookup = semijoin r with support;
        expansion = natural_join lookup with support;
        answer_rows = project expansion keep {x};
        output answer_rows as answer(x);
    }};
    let target: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(a:i32,b:i32); relation T(b:i32);
        r = rename R {a -> x};
        s = rename S {a -> x,b -> y};
        t = rename T {b -> y};
        support = semijoin s with t;
        lookup = semijoin r with support;
        answer_rows = project lookup keep {x};
        output answer_rows as answer(x);
    }};
    ra::check_rewrite(&source, &target).unwrap();
}

#[test]
fn inverse_dead_expansion_accepts_independent_names_but_preserves_keys_and_predicates() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(a:i32,b:i32); relation T(b:i32);
        r = rename R {a -> x};
        s = rename S {a -> x,b -> y};
        t = rename T {b -> y};
        filtered = select s if (y > 0);
        support = semijoin filtered with t;
        lookup = semijoin r with support;
        expansion = natural_join lookup with support;
        answer_rows = project expansion keep {x};
        output answer_rows as answer(x);
    }};
    let target: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(a:i32,b:i32); relation T(b:i32);
        rows = rename R {a -> x};
        candidates = rename S {a -> x,b -> y};
        witnesses = rename T {b -> y};
        positive = select candidates if (y > 0);
        matched = semijoin positive with witnesses;
        checked = semijoin rows with matched;
        result = project checked keep {x};
        output result as answer(x);
    }};
    ra::check_rewrite(&source, &target).unwrap();

    let mut dropped_predicate = target.clone();
    dropped_predicate.plan.definitions[3].operator =
        syn::parse_quote!(project candidates keep {x,y});
    ra::contract::check(&dropped_predicate).unwrap();
    assert!(ra::check_rewrite(&source, &dropped_predicate).is_err());

    let mut changed_key = target.clone();
    changed_key.plan.definitions.insert(
        5, syn::parse_quote!(keyless = project matched keep {y};),
    );
    changed_key.plan.definitions[6].operator =
        syn::parse_quote!(semijoin rows with keyless);
    ra::contract::check(&changed_key).unwrap();
    assert!(ra::check_rewrite(&source, &changed_key).is_err());
}

#[test]
fn final_equality_deduplicates_relational_constraints_across_total_rust() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R {a -> x,b -> y};
        f = select r if (x = 1, y = 1); g = guard f if (opaque());
        e = select g if (x = y); p = project e keep {x}; output p as answer(x);
    }};
    let target: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); r = rename R {a -> x,b -> y};
        f = select r if (x = 1); g = guard f if (opaque());
        e = select g if (x = y, y = 1); p = project e keep {x}; output p as answer(x);
    }};
    ra::check_rewrite(&source, &target).unwrap();
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
        r = rename R {a -> x}; s = rename S {a -> x,b -> y};
        matched = semijoin r with s; py = project s keep {y};
        j = natural_join matched with py; output j as answer(x,y);
    }};
    let correlated: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(a:i32,b:i32);
        r = rename R {a -> x}; s = rename S {a -> x,b -> y};
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
        r = rename R {a -> x,b -> y}; s = rename S {b -> y};
        j = natural_join r with s; exists = project j keep {}; px = project r keep {x};
        out = natural_join px with exists; output out as answer(x);
    }};
    let correlated: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); relation S(b:i32);
        r = rename R {a -> x,b -> y}; s = rename S {b -> y};
        j = natural_join r with s; out = project j keep {x}; output out as answer(x);
    }};
    ra::check_rewrite(&independent, &independent).unwrap();
    assert!(ra::check_rewrite(&independent, &correlated).is_err());
    assert!(ra::check_rewrite(&correlated, &independent).is_err());
}

#[test]
fn relational_constraints_use_equality_classes_across_total_rust() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32,c:i32); r = rename R {a -> x,b -> y,c -> z};
        f = select r if (x = z, z = 1); g = guard f if (opaque());
        e = select g if (x = y, y = 1); p = project e keep {x}; output p as answer(x);
    }};
    let target: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32,c:i32); r = rename R {a -> x,b -> y,c -> z};
        f = select r if (x = z, y = 1, x = 1); g = guard f if (opaque());
        e = select g if (x = y); p = project e keep {x}; output p as answer(x);
    }};
    ra::check_rewrite(&source, &target).unwrap();
    ra::check_rewrite(&target, &source).unwrap();
}

#[test]
fn reducer_column_matches_preserve_total_rust_selection() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(b:i32);
        r = rename R {a -> x}; s = rename S {b -> y};
        j = natural_join r with s; e = select j if (x = y);
        g = guard e if (*x == *y); output g as answer(x,y);
    }};
    let target: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(b:i32);
        r = rename R {a -> x}; s = rename S {b -> y};
        rs = natural_join r with s; re = select rs if (x = y); rr = project re keep {x};
        sr = natural_join s with r; se = select sr if (x = y); ss = project se keep {y};
        j = natural_join rr with ss;
        g = guard j if (*x == *y); e = select g if (x = y);
        output e as answer(x,y);
    }};
    ra::contract::check(&target).unwrap();
    ra::check_rewrite(&source, &target).unwrap();
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
fn total_generators_commute_with_relational_selection() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32);
        r = rename R {a -> x};
        s = select r if (x = 1);
        g = bind s with for y = ([*x]);
        output g as answer(x,y);
    } };
    let moved: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32);
        r = rename R {a -> x};
        g = bind r with for y = ([*x]);
        s = select g if (x = 1);
        output s as answer(x,y);
    } };
    ra::check_rewrite(&source, &moved).unwrap();
    ra::check_rewrite(&moved, &source).unwrap();
}

#[test]
fn occurrence_constants_roundtrip_drop_columns_and_preserve_source_types() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:bool,c:i64);
        r = rename R {c -> x} constants {a = -7, b = true};
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
        r = rename R {} constants {a = 7};
        output r as answer();
    }};
    assert!(ra::contract::check(&all).unwrap()["r"].heading.is_empty());
}

#[test]
fn logical_identity_survives_rename_projection_and_payload_mutation() {
    let mut source: ra::Program = syn::parse_quote! { relational {
        relation R(a: HostType [7], b: HostType [9]);
        r = rename R {a -> x, b -> y};
        p = project r keep {x};
        output p as answer(x);
    }};
    source.plan.declarations[0].columns[0].ty = syn::parse_quote!(ChangedPayload);
    let facts = ra::contract::check(&source).unwrap();
    assert_eq!(
        facts["r"].logical_type_ids["x"],
        ra::syntax::LogicalTypeId(7)
    );
    assert_eq!(
        facts["r"].logical_type_ids["y"],
        ra::syntax::LogicalTypeId(9)
    );
    assert_eq!(facts["p"].logical_type_ids.len(), 1);
    assert_eq!(
        facts["p"].logical_type_ids["x"],
        ra::syntax::LogicalTypeId(7)
    );
}

#[test]
fn union_compares_declared_identities_not_rust_type_payloads() {
    let mut source: ra::Program = syn::parse_quote! { relational {
        relation R(a: HostType [7], b: HostType [9]);
        r = rename R {a -> x, b -> unused_r};
        rp = project r keep {x};
        s = rename R {a -> unused_s, b -> x};
        sp = project s keep {x};
        u = union rp, sp;
        output u as answer(x);
    }};
    assert!(ra::contract::check(&source).is_err());
    source.plan.declarations[0].columns[1].logical_type_id = Some(ra::syntax::LogicalTypeId(7));
    source.plan.declarations[0].columns[1].ty = syn::parse_quote!(DifferentPayload);
    ra::contract::check(&source).unwrap();
}

#[test]
fn occurrence_constants_require_an_exact_literal_source_partition() {
    for operator in [
        quote!(rename R {a -> x} constants {}),
        quote!(rename R {a -> x} constants {b = x}),
        quote!(rename R {a -> x} constants {b = 7, b = 8}),
        quote!(rename R {a -> x} constants {a = 7}),
        quote!(rename R {a -> x} constants {missing = 7}),
        quote!(rename R {} constants {b = 7}),
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
            r = rename R {a -> x} constants {b = 1.0};
            output r as answer(x);
        }})
        .is_ok()
    );
}

#[test]
fn rewrites_preserve_occurrence_constants_without_inventing_union_disjointness() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32);
        r = rename R {a -> x} constants {b = 7};
        s = rename R {a -> x} constants {b = 8};
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
