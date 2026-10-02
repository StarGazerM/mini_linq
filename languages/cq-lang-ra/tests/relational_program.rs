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
    let undeclared = check(quote! {
            relational {
    relation R(value: i32);

                r0 = rename Missing {value -> x};
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

                        r0 = rename R {#invalid_mapping};
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
        let error = check(quote! {
                    relational {
        relation R(value: i32);

                        rows = rename R {value -> x};
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

                rows = rename R {first -> x, second -> y};
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

                r0 = rename R {value -> x};
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
                rows = rename R {value -> x};
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

                used = rename R {value -> x};
                unused = rename S {value -> y};
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
fn opaque_nodes_form_one_ancestry_chain() {
    check(quote! { relational {
        relation R(a: i32); relation S(b: i32);
        r = rename R {a -> x};
        g = guard r if (*x != 0);
        s = rename S {b -> w};
        j = natural_join g with s;
        b = bind j with y: i32 = (10 / *x);
        p = project b keep {x, y}; output p as answer(x, y);
    }})
    .unwrap();
    let error = check(quote! { relational {
        relation R(a: i32); relation S(b: i32);
        r = rename R {a -> x}; s = rename S {b -> w};
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
            relation R(a: i32); r = rename R {a -> x};
            s = select r if (y = 1); output s as answer(x);
        }},
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
    ra::correspondence::check_rewrite(&source, &target).unwrap();
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
        let error = ra::check_rewrite(&source, &target).unwrap_err();
        assert!(error.to_string().contains("tableau"), "{error}");
    }
    let mut target = source.clone();
    target.plan.definitions.remove(1);
    target.plan.definitions[1].operator = syn::parse_quote!(project r keep {x,y});
    ra::contract::check(&target).unwrap();
    let error = ra::check_rewrite(&source, &target).unwrap_err();
    assert!(error.to_string().contains("tableau"), "{error}");
}

#[test]
fn rewrite_rejects_reordered_or_changed_rust_payloads() {
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
    for target in [reordered, changed] {
        ra::contract::check(&target).unwrap();
        let error = ra::check_rewrite(&source, &target).unwrap_err();
        assert!(error.to_string().contains("Rust chain"), "{error}");
    }
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
    let error = ra::check_rewrite(&source, &target).unwrap_err();
    assert!(error.to_string().contains("Rust chain"), "{error}");
}

#[test]
fn rewrite_relational_movement_across_rust_is_directional() {
    let earlier: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R {a -> x};
        s = select r if (x > 0);
        g = guard s if (10 / *x > 0); output g as answer(x);
    }};
    let later: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); r = rename R {a -> x};
        g = guard r if (10 / *x > 0);
        s = select g if (x > 0); output s as answer(x);
    }};
    ra::contract::check(&earlier).unwrap();
    ra::contract::check(&later).unwrap();
    ra::check_rewrite(&later, &earlier).unwrap();
    let error = ra::correspondence::check_rewrite(&earlier, &later).unwrap_err();
    assert!(error.to_string().contains("source-before"), "{error}");
}

#[test]
fn rewrite_checks_the_full_source_prefix_even_outside_rust_free_variables() {
    let earlier: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(b:i32);
        r = rename R {a -> x}; s = rename S {b -> y};
        j = natural_join r with s;
        g = guard j if (10 / *x > 0);
        p = project g keep {x}; output p as answer(x);
    }};
    let later: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(b:i32);
        r = rename R {a -> x}; s = rename S {b -> y};
        g = guard r if (10 / *x > 0);
        j = natural_join g with s;
        p = project j keep {x}; output p as answer(x);
    }};
    ra::contract::check(&later).unwrap();
    ra::check_rewrite(&later, &earlier).unwrap();
    let error = ra::check_rewrite(&earlier, &later).unwrap_err();
    assert!(error.to_string().contains("source-before"), "{error}");
}
