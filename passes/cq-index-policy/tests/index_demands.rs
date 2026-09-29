use cq_index_policy::{check, compile};
use cq_lang_ix as ix;
use cq_lang_ra as ra;
#[allow(dead_code)]
#[path = "../../../test-support/relational/cases.rs"]
mod cases;
#[allow(dead_code)]
#[path = "../../../test-support/relational/semantics.rs"]
mod oracle;
#[path = "../../../test-support/relational/scenarios.rs"]
mod scenarios;
#[allow(dead_code)]
#[path = "../../../test-support/relational/values.rs"]
mod values;
#[test]
fn demands_cover_joins_preserve_answers_and_reject_mutation() {
    for case in scenarios::SCENARIOS {
        let source: ra::Program = syn::parse_str(case.ra).unwrap();
        let target = compile(&source).unwrap();
        check(&source, &target).unwrap();
        assert_eq!(
            target,
            syn::parse_str::<ix::Program>(&case.ix()).unwrap(),
            "{}",
            case.name
        );
        let mut changed = target.clone();
        changed.ra.plan.output.head.relation = syn::parse_quote!(wrong_answer);
        assert!(check(&source, &changed).is_err());
        if !target.indexes.is_empty() {
            let mut missing = target.clone();
            missing.indexes.clear();
            assert!(check(&source, &missing).is_err());
        }
        if let Some(data) = cases::CASES.iter().find(|c| c.name == case.name) {
            let database = data
                .inputs
                .iter()
                .map(|(n, rows)| ((*n).into(), rows.iter().map(|r| r.to_vec()).collect()))
                .collect();
            let expected = data
                .expected
                .iter()
                .map(|r| r.to_vec())
                .collect::<values::Rows>();
            assert_eq!(oracle::evaluate_ra(&source, &database).output, expected);
        }
    }
}
#[test]
fn cartesian_and_repeated_join_reads_share_one_demand() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32); relation S(b:i32);
        r = rename R {a -> a}; s = rename S {b -> b};
        x = semijoin r with s; y = natural_join x with s;
        output y as answer(a,b);
    }};
    let target = compile(&source).unwrap();
    assert_eq!(target.indexes.len(), 1);
    assert!(target.indexes[0].requirement.columns.is_empty());
}

fn versions() -> ra::Program {
    syn::parse_quote! { relational {
        relation R(a: i32, b: i32, c: i32);
        relation S(a: i32, b: i32, c: i32);
        relation T(k: i32);
        unit = unit;
        r = rename R {a -> x, b -> y, c -> z};
        seed = natural_join unit with r;
        s = rename S {a -> x, b -> y, c -> w};
        t = rename T {k -> z};
        left = semijoin seed with s;
        right = semijoin s with r;
        again = semijoin left with s;
        restricted = semijoin again with t;
        joined = natural_join restricted with right;
        result = project joined keep {x, y, z};
        output result as answer(x, y, z);
    }}
}

#[test]
fn first_use_names_share_only_exact_version_and_columns_including_empty_columns() {
    let source = versions();
    let expected: ix::Program = syn::parse_quote! {
        #source indexes {
            i0 = @r[];
            i1 = @s[1, 2];
            i2 = @r[0, 1];
            i3 = @t[0];
            i4 = @right[1, 2];
        }
    };
    assert_eq!(compile(&source).unwrap(), expected);
    check(&source, &expected).unwrap();
}

#[test]
fn logical_heading_coordinates_ignore_declaration_and_rename_order() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a: i32, b: i32, c: i32);
        relation S(a: i32, b: i32, c: i32);
        seed = rename R {c -> z, a -> x, b -> y};
        first = rename S {c -> w, b -> x, a -> z};
        second = rename S {b -> x, c -> q, a -> z};
        joined = natural_join seed with first;
        repeated = natural_join joined with second;
        result = project repeated keep {x, w};
        output result as answer(x, w);
    }};
    let expected: ix::Program = syn::parse_quote! {
        #source indexes { i0 = @first[1, 2]; i1 = @second[1, 2]; }
    };
    assert_eq!(compile(&source).unwrap(), expected);
}

#[test]
fn join_free_plan_has_no_demands_and_missing_operands_are_diagnostic() {
    let source: ra::Program = syn::parse_str(include_str!(
        "../../../test-support/relational/scenarios/scan.ra"
    ))
    .unwrap();
    let expected: ix::Program = syn::parse_quote!(#source indexes {});
    assert_eq!(compile(&source).unwrap(), expected);
    let mut invalid = versions();
    invalid.plan.definitions[2].operator = syn::parse_quote!(natural_join missing with r);
    // An unexpected panic fails this test, as does accepting the operand.
    let error = compile(&invalid).unwrap_err().to_string();
    assert!(error.contains("missing"), "{error}");
}
