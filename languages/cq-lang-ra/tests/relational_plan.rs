use cq_lang_ra as relational_plan;
use quote::{ToTokens, quote};

fn two_hop() -> proc_macro2::TokenStream {
    quote! {
        pub struct TwoHopRoads;
        relation road(src: i32, dst: i32);
        two_hop(from, to) :- road(from, via), road(via, to);
        relational {
            r0 = rename road {src -> from, dst -> via};
            r1 = rename road {src -> via, dst -> to};
            r2 = natural_join r0 with r1;
            r3 = project r2 keep {from, to};
            output r3 as two_hop(from, to);
        }
    }
}

fn check(tokens: proc_macro2::TokenStream) -> syn::Result<()> {
    let module: relational_plan::Module = syn::parse2(tokens)?;
    relational_plan::contract::check_module(&module)
}

#[test]
fn complete_plan_is_well_formed_and_round_trips() {
    let module: relational_plan::Module = syn::parse2(two_hop()).unwrap();
    relational_plan::contract::check_module(&module).unwrap();
    assert!(relational_plan::contract::well_formed_module(&module));
    assert_eq!(module.plan.definitions.len(), 4);

    let reparsed: relational_plan::Module = syn::parse2(module.to_token_stream()).unwrap();
    assert_eq!(module, reparsed);
}

#[test]
fn result_ids_are_dag_references_not_prescribed_r_numbers() {
    check(quote! {
        struct P;
        relation R(value: i32);
        answer(x) :- R(x);
        relational {
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
        struct P;
        relation R(value: i32);
        relation S(value: i32);
        answer(x) :- R(x);
        relational {
            from_other_declared_input = rename S {value -> x};
            output from_other_declared_input as answer(x);
        }
    })
    .unwrap();

    check(quote! {
        struct P;
        relation R(value: i32);
        relation S(value: i32);
        answer(x, y) :- R(x), S(y);
        relational {
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
        struct P;
        relation R(value: i32);
        answer(x) :- R(x);
        relational {
            r0 = rename Missing {value -> x};
            output r0 as answer(x);
        }
    })
    .unwrap_err();
    assert!(undeclared.to_string().contains("undeclared relation"));

    for invalid_mapping in [
        quote!(c0 -> x),
        quote!(c0 -> x, c0 -> y),
        quote!(c0 -> x, c1 -> x),
    ] {
        let error = check(quote! {
            struct P;
            relation R(c0: i32, c1: i32);
            answer(x) :- R(x, y);
            relational {
                r0 = rename R {#invalid_mapping};
                r1 = project r0 keep {x};
                output r1 as answer(x);
            }
        })
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("every declared source attribute")
                || error.to_string().contains("more than once")
                || error.to_string().contains("not distinct")
        );
    }
}

#[test]
fn natural_join_infers_union_heading_without_resolving_rust_types() {
    check(quote! {
        struct P;
        relation People(id: PersonId, manager: PersonId);
        relation Logins(person: u64, day: u32);
        answer(x, day) :- People(x, manager), Logins(x, day);
        relational {
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
        struct P;
        relation R(value: i32);
        relation S(value: i32);
        answer(x, y) :- R(x), S(y);
        relational {
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
        struct P;
        relation R(first: i32, second: i32);
        answer(z, x) :- R(x, z);
        relational {
            rows = rename R {first -> x, second -> z};
            result = project rows keep {x, z};
            output result as answer(z, x);
        }
    })
    .unwrap();

    for attributes in [quote!(x, x), quote!(missing)] {
        let error = check(quote! {
            struct P;
            relation R(value: i32);
            answer(x) :- R(x);
            relational {
                rows = rename R {value -> x};
                result = project rows keep {#attributes};
                output result as answer(x);
            }
        })
        .unwrap_err();
        assert!(
            error.to_string().contains("repeats attribute")
                || error.to_string().contains("not present")
        );
    }
}

#[test]
fn output_must_match_the_source_head_and_input_heading() {
    let wrong_head = check(quote! {
        struct P;
        relation R(value: i32);
        answer(x) :- R(x);
        relational {
            rows = rename R {value -> x};
            output rows as other(x);
        }
    })
    .unwrap_err();
    assert!(wrong_head.to_string().contains("source query head"));

    let wrong_heading = check(quote! {
        struct P;
        relation R(first: i32, second: i32);
        answer(x) :- R(x, y);
        relational {
            rows = rename R {first -> x, second -> y};
            output rows as answer(x);
        }
    })
    .unwrap_err();
    assert!(wrong_heading.to_string().contains("input heading"));
}

#[test]
fn definitions_are_unique_and_operands_must_be_earlier() {
    let duplicate = check(quote! {
        struct P;
        relation R(value: i32);
        answer(x) :- R(x);
        relational {
            r0 = rename R {value -> x};
            r0 = project r0 keep {x};
            output r0 as answer(x);
        }
    })
    .unwrap_err();
    assert!(duplicate.to_string().contains("defined more than once"));

    let forward = check(quote! {
        struct P;
        relation R(value: i32);
        answer(x) :- R(x);
        relational {
            result = project rows keep {x};
            rows = rename R {value -> x};
            output result as answer(x);
        }
    })
    .unwrap_err();
    assert!(forward.to_string().contains("defined earlier"));
}

#[test]
fn every_definition_must_contribute_to_output() {
    let error = check(quote! {
        struct P;
        relation R(value: i32);
        relation S(value: i32);
        answer(x) :- R(x);
        relational {
            used = rename R {value -> x};
            unused = rename S {value -> y};
            output used as answer(x);
        }
    })
    .unwrap_err();
    assert!(error.to_string().contains("not reachable"));
}

#[test]
fn unit_is_the_zero_attribute_relation() {
    check(quote! {
        struct P;
        relation R(value: i32);
        answer(x) :- R(x);
        relational {
            seed = unit;
            rows = rename R {value -> x};
            joined = natural_join seed with rows;
            output joined as answer(x);
        }
    })
    .unwrap();
}

#[test]
fn erasure_reads_the_query_back_from_the_plan_in_the_program_order() {
    use cq_lang_cq as cq;
    use cq_lang_cq::normal::Kernel;
    // Atoms out of written order and named by the plan's own results, a
    // self-join renamed twice (one occurrence, restricted), a select and an
    // extend outside the elaborated `{ let …; (…) }` shape.
    let module: relational_plan::Module = syn::parse2(quote! {
        struct P;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(x, k) :- R(x, y), S(y, z), R(z, x), if x != z, let k: i32 = x + z;
        relational {
            last = rename R { a -> z, b -> x };
            first = rename R { a -> x, b -> y };
            only_x = project first keep { x };
            seed = natural_join last with only_x;
            again = rename R { a -> x, b -> y };
            both = natural_join seed with again;
            mid = rename S { b -> y, c -> z };
            all = natural_join both with mid;
            kept = select all if (x != z);
            wide = extend kept with { k: i32 } value ((x + z,));
            done = project wide keep { x, k };
            output done as answer(x, k);
        }
    })
    .unwrap();
    let erased = relational_plan::erase(&module);
    let mut expected = module.program.clone();
    expected.query = syn::parse_quote! {
        answer(x, k) :- R(x, y), S(y, z), R(z, x), if x != z, let (k,) = (x + z,);
    };
    assert_eq!(
        erased.program.query.to_token_stream().to_string(),
        expected.query.to_token_stream().to_string()
    );
    assert_eq!(
        relational_plan::kernel(&module),
        Kernel::of(&cq::Module { program: expected })
    );
    // What the plan lacks is missing from the kernel; the kernel is not
    // the program's.
    let mut fewer = module.clone();
    fewer.plan.definitions.retain(|d| d.result != "kept");
    fewer.plan.definitions[8].operator =
        syn::parse_quote!(extend all with { k: i32 } value ((x + z,)));
    assert!(relational_plan::contract::well_formed_module(&fewer));
    assert_eq!(relational_plan::kernel(&fewer).guards, Vec::<String>::new());
}
