use cq_index_policy::{check, compile};
use cq_lang_ix as ix;
use cq_lang_ra as ra;
use cq_testkit::{cases, corpus as scenarios, semantics as oracle, values};
#[test]
fn demands_cover_joins_preserve_answers_and_reject_mutation() {
    for case in scenarios::SCENARIOS {
        let source: ra::Program = syn::parse_str(case.ra).unwrap();
        let target = compile(&source).unwrap();
        check(&source, &target).unwrap();
        assert_eq!(target, case.parse_ix().unwrap(), "{}", case.name);
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
        r = rename R@0 {a -> a}; s = rename S@1 {b -> b};
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
        r = rename R@0 {a -> x, b -> y, c -> z};
        seed = natural_join unit with r;
        s = rename S@1 {a -> x, b -> y, c -> w};
        t = rename T@2 {k -> z};
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
            i0 = @r[] stream;
            i1 = @s[1, 2] stream;
            i2 = @r[0, 1] stream;
            i3 = @t[0] stream;
            i4 = @right[1, 2] stream;
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
        seed = rename R@0 {c -> z, a -> x, b -> y};
        first = rename S@1 {c -> w, b -> x, a -> z};
        second = rename S@2 {b -> x, c -> q, a -> z};
        joined = natural_join seed with first;
        repeated = natural_join joined with second;
        result = project repeated keep {x, w};
        output result as answer(x, w);
    }};
    let expected: ix::Program = syn::parse_quote! {
        #source indexes { i0 = @first[1, 2] stream; i1 = @second[1, 2] stream; }
    };
    assert_eq!(compile(&source).unwrap(), expected);
}

#[test]
fn join_free_plan_has_no_demands_and_missing_operands_are_diagnostic() {
    let source = scenarios::SCENARIOS
        .iter()
        .find(|scenario| scenario.name == "scan")
        .unwrap()
        .parse_ra()
        .unwrap();
    let expected: ix::Program = syn::parse_quote!(#source indexes {});
    assert_eq!(compile(&source).unwrap(), expected);
    let mut invalid = versions();
    invalid.plan.definitions[2].operator = syn::parse_quote!(natural_join missing with r);
    // An unexpected panic fails this test, as does accepting the operand.
    let error = compile(&invalid).unwrap_err().to_string();
    assert!(error.contains("missing"), "{error}");
}

#[test]
fn independent_derivation_rejects_mutations_with_construct_diagnostics() {
    let source = versions();
    let target = compile(&source).unwrap();
    let reject = |s: &ra::Program, t: &ix::Program, names: &[&str]| {
        let error = check(s, t).unwrap_err().to_string();
        for name in names {
            assert!(error.contains(name), "{error}");
        }
    };
    let mut bad_source = source.clone();
    bad_source.plan.definitions[2].operator = syn::parse_quote!(natural_join absent with r);
    reject(&bad_source, &target, &["absent"]);
    let mut bad = target.clone();
    bad.indexes[1].requirement.columns = syn::parse_quote!(2, 1);
    reject(&source, &bad, &["i1", "increasing"]);
    let mut bad = target.clone();
    bad.ra.plan.definitions[1].operator = syn::parse_quote!(rename R@0 {c -> z, b -> y, a -> x});
    reject(&source, &bad, &["r", "preserve"]);
    let mut bad = target.clone();
    bad.indexes = syn::parse_quote!(i0 = @r[] stream; i1 = @s[1,2] stream; i2 = @t[0] stream; i3 = @r[0,1] stream; i4 = @right[1,2] stream;);
    ix::contract::check(&bad).unwrap();
    reject(&source, &bad, &["right", "first-use"]);
    let mut bad = target.clone();
    bad.indexes[1].name = syn::parse_quote!(other);
    reject(&source, &bad, &["left", "i1", "named"]);
    let mut bad = target.clone();
    bad.indexes.pop();
    reject(&source, &bad, &["joined", "missing"]);
    let mut bad = target.clone();
    bad.indexes[1].requirement.columns = syn::parse_quote!(0, 1);
    reject(&source, &bad, &["i1", "no join"]);
    let mut bad = target.clone();
    bad.indexes.push(syn::parse_quote!(extra = @r[] stream));
    bad.indexes.push_punct(Default::default());
    reject(&source, &bad, &["extra", "duplicate"]);
}

#[test]
fn ix_const_covers_all_literal_conjunctions_and_keeps_supply_in_identity() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a: i32, b: i32); relation S(a: i32, b: i32);
        r = rename R@0 { a -> x, b -> y }; s = rename S@1 { a -> x, b -> y };
        fixed = select r if (y = 7, x = -2);
        repeated = select r if (x = 3, y = 8);
        streamed = natural_join s with r;
        restricted = natural_join fixed with repeated;
        result = natural_join restricted with streamed;
        output result as answer(x, y);
    }};
    let target = compile(&source).unwrap();
    let expected: ix::Program = syn::parse_quote! { #source indexes {
        i0 = @r[0,1] const;
        i1 = @r[0,1] stream;
        i2 = @repeated[0,1] stream;
        i3 = @streamed[0,1] stream;
    }};
    assert_eq!(target, expected);
    check(&source, &target).unwrap();
    let mut missing = target.clone();
    missing.indexes = syn::parse_quote! {
        i0 = @r[0,1] stream;
        i1 = @repeated[0,1] stream;
        i2 = @streamed[0,1] stream;
    };
    // The remaining demands are locally valid; correspondence detects IX-Const omission.
    ix::contract::check(&missing).unwrap();
    assert!(
        check(&source, &missing)
            .unwrap_err()
            .to_string()
            .contains("first-use")
    );
    let mut wrong_supply = target;
    wrong_supply.indexes[0].requirement.supply = syn::parse_quote!(stream);
    assert!(check(&source, &wrong_supply).is_err());
}

#[test]
fn rust_chain_nodes_and_column_equalities_create_no_demands() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a: i32, b: i32);
        r = rename R@0 { a -> x, b -> y };
        equal = select r if (x = y);
        guarded = guard equal if (*x != 0);
        bound = bind guarded with z: i32 = (10 / *x);
        projected = project bound keep {x, z};
        output projected as answer(x, z);
    }};
    let target = compile(&source).unwrap();
    assert!(target.indexes.is_empty());
    assert_eq!(target.ra, source);
    check(&source, &target).unwrap();
}

#[test]
fn membership_and_parameters_are_const_but_disjunction_and_variable_bounds_are_filters() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32);
        r = rename R@0 {a -> x,b -> y};
        points = select r if (x = @(captured), y in {1,2,3});
        alternatives = select points if ((x < 0 and y = 2) or (x > 5 and y = 3));
        variables = select alternatives if (x < y);
        output variables as answer(x,y);
    }};
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    assert_eq!(target.indexes.len(), 1);
    assert_eq!(
        ix::signature(&target.indexes[0].requirement),
        ("r".into(), vec![0, 1], ix::SupplyKind::Const)
    );
    let mut wrong = target.clone();
    wrong.indexes[0].requirement.supply = syn::parse_quote!(range);
    assert!(check(&source, &wrong).is_err());
}

#[test]
fn occurrence_constants_are_not_index_keys_or_selection_demands() {
    let source: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32,c:i32); relation S(a:i32);
        r = rename R@0 {a -> x, c -> y} constants {b = 7};
        s = rename S@1 {a -> x};
        j = natural_join s with r;
        output j as answer(x,y);
    }};
    let expected: ix::Program = syn::parse_quote! { #source indexes {
        i0 = @r[0] stream;
    }};
    let target = compile(&source).unwrap();
    assert_eq!(target, expected);
    check(&source, &target).unwrap();
    let lone: ra::Program = syn::parse_quote! { relational {
        relation R(a:i32);
        r = rename R@0 {} constants {a = 7};
        output r as answer();
    }};
    assert!(compile(&lone).unwrap().indexes.is_empty());
}
