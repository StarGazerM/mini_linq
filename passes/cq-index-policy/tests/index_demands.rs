//! Authored expected IX checks the complete RA → IX boundary.
use cq_lang_ix as ix;
use cq_lang_ra as ra;

fn source() -> ra::Module {
    syn::parse_quote! {
        struct Versions;
        relation R(a:i32,b:i32,c:i32);
        relation S(a:i32,b:i32,c:i32);
        relation T(k:i32);
        answer(x,y,z) :- R(x,y,z), S(x,y,w), T(z);
        relational {
            unit = unit;
            r = rename R {a -> x,b -> y,c -> z};
            seed = natural_join unit with r;
            s = rename S {a -> x,b -> y,c -> w};
            t = rename T {k -> z};
            left = semijoin seed with s;
            right = semijoin s with r;
            again = semijoin left with s;
            restricted = semijoin again with t;
            joined = natural_join restricted with right;
            result = project joined keep {x,y,z};
            output result as answer(x,y,z);
        }
    }
}

#[test]
fn first_use_names_share_only_exact_version_and_columns_including_empty_columns() {
    let source = source();
    let expected: ix::Module = syn::parse_quote! {
        #source
        indexes {
            i0 = @r[];
            i1 = @s[1,2];
            i2 = @r[0,1];
            i3 = @t[0];
            i4 = @right[1,2];
        }
    };
    let target = cq_index_policy::compile(&source).unwrap();
    assert_eq!(target, expected);
    assert!(cq_index_policy::check(&source, &expected));
    assert_eq!(target.relational_plan, source);
}

#[test]
fn logical_heading_coordinates_are_independent_of_input_declaration_and_rename_order() {
    let source: ra::Module = syn::parse_quote! {
        struct Reordered;
        relation R(a:i32,b:i32,c:i32);
        relation S(a:i32,b:i32,c:i32);
        answer(x,w) :- R(x,y,z), S(z,x,w), S(z,x,q);
        relational {
            seed = rename R {c -> z, a -> x, b -> y};
            first = rename S {c -> w, b -> x, a -> z};
            second = rename S {b -> x, c -> q, a -> z};
            joined = natural_join seed with first;
            repeated = natural_join joined with second;
            result = project repeated keep {x,w};
            output result as answer(x,w);
        }
    };
    let expected: ix::Module = syn::parse_quote! {
        #source
        indexes { i0 = @first[1,2]; i1 = @second[1,2]; }
    };
    assert_eq!(cq_index_policy::compile(&source).unwrap(), expected);
}

#[test]
fn join_free_plan_keeps_demands_explicitly_empty() {
    let source: ra::Module = syn::parse_quote! {
        struct Read;
        relation R(a:i32);
        answer(x) :- R(x);
        relational {
            r = rename R {a -> x};
            output r as answer(x);
        }
    };
    let expected: ix::Module = syn::parse_quote! { #source indexes {} };
    assert_eq!(cq_index_policy::compile(&source).unwrap(), expected);
}

#[test]
fn correspondence_rejects_changes_in_plan_and_noncanonical_serialization() {
    let source = source();
    let exact = cq_index_policy::compile(&source).unwrap();
    let mut changed_source = exact.clone();
    changed_source.relational_plan.program.name = syn::parse_quote!(DifferentProgram);
    let mut reordered = exact.clone();
    reordered.indexes = reordered.indexes.iter().rev().cloned().collect();
    reordered.indexes.push_punct(Default::default());
    let mut swapped = exact.clone();
    let first = swapped.indexes[0].requirement.clone();
    swapped.indexes[0].requirement = swapped.indexes[1].requirement.clone();
    swapped.indexes[1].requirement = first;
    let mut missing = exact.clone();
    missing.indexes.pop();
    let mut empty = exact.clone();
    empty.indexes.clear();
    let mut renamed = exact.clone();
    renamed.indexes[0].name = syn::parse_quote!(other);
    for mutation in [changed_source, reordered, swapped, missing, empty, renamed] {
        ix::contract::check_module(&mutation).unwrap();
        assert!(!cq_index_policy::check(&source, &mutation));
    }
}

#[test]
fn invalid_operand_is_diagnosed_before_analysis_instead_of_panicking() {
    let mut source = source();
    let ra::Operator::NaturalJoin(join) = &mut source.plan.definitions[2].operator else {
        unreachable!();
    };
    join.left = syn::parse_quote!(missing);
    let error = std::panic::catch_unwind(|| cq_index_policy::compile(&source))
        .expect("invalid RA must not panic")
        .expect_err("an unbound operand must be rejected");
    assert!(
        error
            .to_string()
            .contains("must name a result defined earlier")
    );
}

#[allow(unused_imports, dead_code)]
mod cq_index_policy {
    pub use ::cq_index_policy::*;
    pub fn compile(source: &cq_lang_ra::Module) -> syn::Result<cq_lang_ix::Module> {
        Ok(cq_lang_ix::Module::new(
            source.clone(),
            ::cq_index_policy::compile(source)?,
        ))
    }
    pub fn check(source: &cq_lang_ra::Module, target: &cq_lang_ix::Module) -> bool {
        target.relational_plan == *source && ::cq_index_policy::check(source, &target.section)
    }
}
