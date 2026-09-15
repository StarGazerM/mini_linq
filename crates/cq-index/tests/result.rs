//! Named-result index selection checked directly on authored relational algebra.
use cq_ir::{index_requirements as ix, relational_plan as ra};

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
fn complete_keys_reuse_only_the_same_rhs_version_in_first_use_order() {
    let source = source();
    let target = cq_index::result::compile(&source).unwrap();
    // Sorted RHS headings determine ordinals. Repeated probes of s reuse its
    // key, while its reduced version right needs a separate key. The initial
    // unit join has no shared columns and therefore requests no empty key.
    let expected: ix::Module = syn::parse_quote! {
        #source
        indexes { @s[1,2]; @r[0,1]; @t[0]; @right[1,2]; }
    };
    assert_eq!(target, expected);
    assert!(cq_index::result::check(&source, &expected));
}

#[test]
fn correspondence_rejects_well_formed_wrong_keys_versions_order_and_extras() {
    let source = source();
    let exact = cq_index::result::compile(&source).unwrap();

    let mut missing = exact.clone();
    missing.indexes = missing.indexes.iter().skip(1).cloned().collect();
    let mut wrong_key = exact.clone();
    wrong_key.indexes[0].key_columns = syn::parse_quote!(0, 1);
    let mut wrong_version = exact.clone();
    wrong_version.indexes[0].relation = syn::parse_quote!(left);
    let mut wrong_namespace = exact.clone();
    wrong_namespace.indexes[0].result_token = None;
    wrong_namespace.indexes[0].relation = syn::parse_quote!(S);
    let mut reordered = exact.clone();
    reordered.indexes = reordered.indexes.iter().rev().cloned().collect();
    let mut extra = exact.clone();
    extra.indexes.push(syn::parse_quote!(@seed[0]));
    let mut changed_source = exact.clone();
    changed_source.relational_plan.program.name = syn::parse_quote!(DifferentProgram);

    for mutation in [
        missing,
        wrong_key,
        wrong_version,
        wrong_namespace,
        reordered,
        extra,
        changed_source,
    ] {
        ix::contract::check(&mutation).unwrap();
        assert!(!cq_index::result::check(&source, &mutation));
    }
}

#[test]
fn invalid_operand_returns_a_diagnostic_before_index_analysis() {
    let mut source = source();
    let ra::Operator::NaturalJoin(join) = &mut source.plan.definitions[2].operator else {
        unreachable!();
    };
    join.left = syn::parse_quote!(missing);
    let result = std::panic::catch_unwind(|| cq_index::result::compile(&source))
        .expect("invalid typed RA must not panic");
    let error = result.expect_err("an unbound operand must be rejected");
    assert!(
        error
            .to_string()
            .contains("must name a result defined earlier")
    );
}
