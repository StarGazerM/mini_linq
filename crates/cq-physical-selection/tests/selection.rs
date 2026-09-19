use cq_ir::{index_requirements as ix, physical};
use cq_physical_selection::given_bindings;
use quote::{ToTokens, quote};

fn source() -> ix::Module {
    syn::parse_quote! {
        pub struct Chain;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        relation T(c: i32, d: i32);
        relation Unused(value: i32);
        answer(a, d) :- R(a, b), S(b, c), T(c, d);
        relational {
            r = rename R {a -> a, b -> b};
            s = rename S {b -> b, c -> c};
            t = rename T {c -> c, d -> d};
            j0 = natural_join r with s;
            j1 = natural_join j0 with t;
            result = project j1 keep {a, d};
            output result as answer(a, d);
        }
        indexes { i0 = @s[0]; i1 = @t[0]; }
        uses { j0.right = i0; j1.right = i1; }
    }
}

fn assignments() -> physical::Assignments {
    syn::parse_quote! {
        physical {
            relations {
                R = ext::rows;
                S = ext::hash_relation;
                T = ext::sorted_relation;
                r = ext::rows;
                s = ext::hash_relation;
                t = ext::sorted_relation;
                j0 = ext::borrowed_pairs;
                j1 = ext::borrowed_pairs;
                result = ext::borrowed_projection;
            }
            indexes { i0 = ext::hash_relation; i1 = ext::sorted_relation; }
            lowerings {
                r = ext::rename;
                s = ext::rename;
                t = ext::rename;
                j0 = ext::indexed_join;
                j1 = ext::indexed_join;
                result = ext::project;
            }
        }
    }
}

#[test]
fn selection_matches_complete_independently_authored_physical_output() {
    let selected = given_bindings(source(), assignments()).unwrap();
    let expected = quote! {
        pub struct Chain;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        relation T(c: i32, d: i32);
        relation Unused(value: i32);
        answer(a, d) :- R(a, b), S(b, c), T(c, d);
        relational {
            r = rename R {a -> a, b -> b};
            s = rename S {b -> b, c -> c};
            t = rename T {c -> c, d -> d};
            j0 = natural_join r with s;
            j1 = natural_join j0 with t;
            result = project j1 keep {a, d};
            output result as answer(a, d);
        }
        indexes { i0 = @s[0]; i1 = @t[0]; }
        uses { j0.right = i0; j1.right = i1; }
        physical {
            relations {
                R = ext::rows;
                S = ext::hash_relation;
                T = ext::sorted_relation;
                r = ext::rows;
                s = ext::hash_relation;
                t = ext::sorted_relation;
                j0 = ext::borrowed_pairs;
                j1 = ext::borrowed_pairs;
                result = ext::borrowed_projection;
            }
            indexes { i0 = ext::hash_relation; i1 = ext::sorted_relation; }
            lowerings {
                r = ext::rename;
                s = ext::rename;
                t = ext::rename;
                j0 = ext::indexed_join;
                j1 = ext::indexed_join;
                result = ext::project;
            }
        }
    };
    let parsed: physical::Module = syn::parse2(expected).unwrap();
    physical::contract::check_structure(&parsed).unwrap();
    assert_eq!(selected, parsed);
    let printed: physical::Module = syn::parse2(selected.to_token_stream()).unwrap();
    assert_eq!(printed, parsed);
}

#[test]
fn every_required_table_entry_is_explicit() {
    let mut choices = assignments();
    choices.relations.pop();
    assert_error(choices, "missing relation provider assignment for `result`");
    let mut choices = assignments();
    choices.indexes.pop();
    assert_error(choices, "missing index provider assignment for `i1`");
    let mut choices = assignments();
    choices.lowerings.pop();
    assert_error(choices, "missing lowering assignment for `result`");
    let mut choices = assignments();
    choices.lowerings = choices
        .lowerings
        .into_iter()
        .filter(|binding| binding.name != "r")
        .collect();
    choices.lowerings.push_punct(Default::default());
    assert_error(choices, "missing lowering assignment for `r`");
}

#[test]
fn no_assignments_for_unused_inputs_unknown_indexes_or_host_lowerings() {
    let mut choices = assignments();
    choices
        .relations
        .push(syn::parse_quote!(Unused = ext::rows));
    choices.relations.push_punct(Default::default());
    assert_error(
        choices,
        "unexpected relation provider assignment for `Unused`",
    );
    let mut choices = assignments();
    choices.indexes.push(syn::parse_quote!(other = ext::rows));
    choices.indexes.push_punct(Default::default());
    assert_error(choices, "unexpected index provider assignment for `other`");
    let mut choices = assignments();
    choices.lowerings.push(syn::parse_quote!(R = ext::rename));
    choices.lowerings.push_punct(Default::default());
    assert_error(choices, "unexpected lowering assignment for `R`");
}

#[test]
fn duplicate_assignments_include_equivalent_raw_identifier_spellings() {
    let mut choices = assignments();
    choices
        .relations
        .push(syn::parse_quote!(r#r = ext::different));
    choices.relations.push_punct(Default::default());
    assert_error(choices, "duplicate relation provider assignment for `r`");
    let mut choices = assignments();
    choices
        .indexes
        .push(syn::parse_quote!(r#i0 = ext::different));
    choices.indexes.push_punct(Default::default());
    assert_error(choices, "duplicate index provider assignment for `i0`");
    let mut choices = assignments();
    choices
        .lowerings
        .push(syn::parse_quote!(r#j0 = ext::different));
    choices.lowerings.push_punct(Default::default());
    assert_error(choices, "duplicate lowering assignment for `j0`");
}

#[test]
fn every_assignment_in_a_nonempty_table_requires_its_terminating_semicolon() {
    let mut choices = assignments();
    choices.relations.pop_punct();
    assert_error(
        choices,
        "each relation provider assignment must end with `;`",
    );
    let mut choices = assignments();
    choices.indexes.pop_punct();
    assert_error(choices, "each index provider assignment must end with `;`");
    let mut choices = assignments();
    choices.lowerings.pop_punct();
    assert_error(choices, "each lowering assignment must end with `;`");
}

#[test]
fn provider_paths_are_opaque_and_structural_success_does_not_claim_admission() {
    let mut choices = assignments();
    for binding in &mut choices.relations {
        binding.implementation = syn::parse_quote!(::an_unknown_crate::r#type);
    }
    choices.indexes[0].implementation = syn::parse_quote!(arbitrary::first);
    choices.indexes[1].implementation = syn::parse_quote!(arbitrary::second);
    choices.lowerings[3].implementation = syn::parse_quote!(arbitrary::pruned_join);
    let selected = given_bindings(source(), choices.clone()).unwrap();
    assert_eq!(selected.assignments, choices);
}

#[test]
fn one_relation_version_can_have_different_index_providers() {
    let input: ix::Module = syn::parse_quote! {
        struct TwoIndexes;
        relation L(a: i32);
        relation R(a: i32, b: i32);
        answer(a, b) :- L(a), R(a, b);
        relational {
            l = rename L {a -> a};
            r = rename R {a -> a, b -> b};
            joined = natural_join l with r;
            result = semijoin joined with r;
            output result as answer(a, b);
        }
        indexes { i0 = @r[0]; i1 = @r[0, 1]; }
        uses { joined.right = i0; result.right = i1; }
    };
    let choices: physical::Assignments = syn::parse_quote! {
        physical {
            relations {
                L = ext::rows;
                R = ext::array_rows;
                l = ext::rows;
                r = ext::array_rows;
                joined = ext::borrowed_pairs;
                result = ext::borrowed_pairs;
            }
            indexes { i0 = ext::hash_buckets; i1 = ext::sorted_buckets; }
            lowerings {
                l = ext::rename;
                r = ext::rename;
                joined = ext::indexed_join;
                result = ext::indexed_semijoin;
            }
        }
    };
    let selected = given_bindings(input.clone(), choices.clone()).unwrap();
    assert_eq!(selected.index_requirements, input);
    assert_eq!(selected.assignments, choices);
}

#[test]
fn invalid_ix_is_rejected_before_physical_assignment_checks() {
    let mut input = source();
    input.uses[0].index = syn::parse_quote!(not_defined);
    assert!(given_bindings(input, assignments()).is_err());
}

#[test]
fn unit_has_a_result_representation_and_a_lowering() {
    let input: ix::Module = syn::parse_quote! {
        struct Unit;
        relation R(value: i32);
        answer(x) :- R(x);
        relational {
            seed = unit;
            rows = rename R {value -> x};
            joined = natural_join seed with rows;
            output joined as answer(x);
        }
        indexes { i0 = @rows[]; }
        uses { joined.right = i0; }
    };
    let choices: physical::Assignments = syn::parse_quote! {
        physical {
            relations {
                R = ext::rows;
                seed = ext::unit;
                rows = ext::rows;
                joined = ext::rows;
            }
            indexes { i0 = ext::unit_tuple_index; }
            lowerings {
                seed = ext::unit;
                rows = ext::rename;
                joined = ext::indexed_join;
            }
        }
    };
    given_bindings(input.clone(), choices.clone()).unwrap();
    let mut incomplete = choices;
    incomplete.lowerings = incomplete
        .lowerings
        .into_iter()
        .filter(|binding| binding.name != "seed")
        .collect();
    incomplete.lowerings.push_punct(Default::default());
    assert!(
        given_bindings(input, incomplete)
            .unwrap_err()
            .to_string()
            .contains("missing lowering assignment for `seed`")
    );
}

#[test]
fn host_and_result_name_collision_is_rejected_without_changing_ra() {
    let input: ix::Module = syn::parse_quote! {
        struct Collision;
        relation R(value: i32);
        answer(x) :- R(x);
        relational {
            R = rename R {value -> x};
            output R as answer(x);
        }
        indexes {}
        uses {}
    };
    let choices: physical::Assignments = syn::parse_quote! {
        physical {
            relations { R = ext::rows; }
            indexes {}
            lowerings { R = ext::rename; }
        }
    };
    ix::contract::check(&input).unwrap();
    assert!(
        given_bindings(input, choices)
            .unwrap_err()
            .to_string()
            .contains("denotes both a host input and an RA result")
    );
}

fn assert_error(choices: physical::Assignments, expected: &str) {
    let actual = given_bindings(source(), choices).unwrap_err().to_string();
    assert!(actual.contains(expected), "{actual}");
}
