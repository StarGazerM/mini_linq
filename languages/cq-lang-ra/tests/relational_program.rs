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
                r2 = select r1 if (accept(y));
                output r2 as answer(x);
            }
        };
    assert!(ra::contract::check(&source).is_err());
}

#[test]
fn extend_requires_available_inputs_and_fresh_distinct_columns() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32);
        r = rename R {a -> x,b -> y}; p = project r keep {x};
        e = extend p with {z:i32} value (x + 1);
        output e as answer(x,z);
    }};
    ra::contract::check(&source).unwrap();
    for operator in [
        quote!(extend p with {z:i32} value (y + 1)),
        quote!(extend p with {x:i32} value (1)),
        quote!(extend p with {z:i32,z:i32} value ((1,2))),
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
