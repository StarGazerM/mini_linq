use cq_lang_cq as cq;
use quote::ToTokens;

fn query(guard: &str) -> cq::Program {
    syn::parse_str(&format!(
        "struct Q; relation R(a:i32,b:i32); answer(x) :- R(x,y), if {guard};"
    ))
    .unwrap()
}

#[test]
fn every_rust_guard_is_preserved_without_recognition() {
    for source in [
        "x < cutoff",
        "*x < cutoff",
        "&x < cutoff",
        "x == y",
        "x != -2",
        "x <= 4u32",
        "x == true",
        "x == 'a'",
        "x == \"text\"",
        "!(x < cutoff || y >= 5)",
        "*x + 1 < 5",
        "f(*x) < 5",
        "x.abs() < 5",
        "(x as i64) < 5",
        "x[0] < 5",
        "{ *x } < 5",
        "number!() < 5",
        "x.field < 5",
        "x < module::cutoff",
        "x < *cutoff",
        "x < &cutoff",
        "f(*x) || x < 5",
        "x < cutoff && f(*x) && y != 0 && g(*y)",
        "f(*x) && x < cutoff && g(*y) && y != 0",
        "let Some(local) = host(*x) && local < cutoff",
        "x < 5 && let Some(local) = host(*x) && local < cutoff",
    ] {
        let q = query(source);
        assert_eq!(q.query.body.len(), 2, "{source}");
        let cq::BodyItem::Filter(filter) = &q.query.body[1] else {
            panic!("{source}")
        };
        assert_eq!(
            filter.condition,
            syn::parse_str::<syn::Expr>(source).unwrap()
        );
        cq::contract::check(&q).unwrap();
        assert_eq!(cq::normal::normalize(&q).unwrap(), q);
        assert_eq!(syn::parse2::<cq::Program>(q.to_token_stream()).unwrap(), q);
    }
}

fn relational(predicate: &str) -> cq::Program {
    syn::parse_str(&format!(
        "struct Q; relation R(a:i32,b:i32); answer(x) :- R(x,y), where {predicate};"
    ))
    .unwrap()
}

#[test]
fn where_supports_comparisons_membership_ranges_and_closed_rust() {
    for predicate in [
        "x == y",
        "x != -2",
        "x < cutoff",
        "x <= constants.limit",
        "x > limit()",
        "x >= values[0]",
        "x == (limit + 1)",
        "x == { let temporary = host(); temporary }",
        "x == call::<First, Second>()",
        "x == || host()",
        "x in [1, 2, 3]",
        "x in [host(), y, constants.limit]",
        "x in [host()]",
        "x in []",
        "x in low..high",
        "x in low..=high",
        "x in host()..=constants.limit",
        "x == 1 && y >= 2",
        "(x == 1 && y >= 2) || (x == 3 && y <= 4)",
        "(x == 1 || y == 2) && (x > 0 || y > 0)",
    ] {
        let q = relational(predicate);
        cq::normal::check(&q).unwrap();
        assert!(matches!(
            q.query.body[1],
            cq::BodyItem::Constraint(_) | cq::BodyItem::Equality(_)
        ));
        assert_eq!(
            syn::parse2::<cq::Program>(q.to_token_stream()).unwrap(),
            q,
            "{predicate}"
        );
        assert_eq!(cq::normal::normalize(&q).unwrap(), q);
    }
}

#[test]
fn relational_conjunctions_keep_range_and_point_access_facts() {
    for predicate in ["x >= 2 && x < 9", "x in 2..9"] {
        let source = relational(predicate);
        let mut theta = cq_syntax::algebra::Theta::default();
        for item in source.query.body.iter().skip(1) {
            let cq::BodyItem::Constraint(constraint) = item else {
                panic!()
            };
            theta.push_constraint(constraint.clone());
        }
        let (column, bounds) = theta.range_bounds().unwrap();
        assert_eq!(column, "x");
        assert_eq!(bounds.len(), 2);
        assert_eq!(
            syn::parse2::<cq_syntax::algebra::Theta>(theta.to_token_stream()).unwrap(),
            theta
        );
    }
    let source = relational("x == 2 && y == 9");
    let mut theta = cq_syntax::algebra::Theta::default();
    for item in source.query.body.iter().skip(1) {
        let cq::BodyItem::Equality(equality) = item else {
            panic!("literal equality uses the canonical point-access clause")
        };
        theta.equalities.push(equality.clone());
    }
    assert_eq!(theta.equalities.len(), 2);
    assert_eq!(
        theta.point_columns().unwrap(),
        ["x".to_owned(), "y".to_owned()].into()
    );
}

#[test]
fn where_equalities_use_canonical_clauses_after_scope_resolution() {
    for (predicate, equality) in [("x == 2", true), ("x == y", true), ("x == host", false)] {
        let source = relational(predicate);
        assert_eq!(
            matches!(source.query.body[1], cq::BodyItem::Equality(_)),
            equality,
            "{predicate}"
        );
        assert_eq!(
            syn::parse2::<cq::Program>(source.to_token_stream()).unwrap(),
            source
        );
    }
}

#[test]
fn where_conjuncts_keep_independent_columns_and_all_bounds() {
    for (predicate, clauses) in [
        ("x < y && y < cutoff", 2),
        ("(x == first || x == second) && y == expected", 2),
        ("x in low..high && y != 0", 3),
        ("x >= low && x <= high && y != 0", 2),
        ("x >= low && y != 0 && x <= high", 3),
    ] {
        let source = relational(predicate);
        assert_eq!(source.query.body.len(), clauses + 1, "{predicate}");
        let constraints: Vec<_> = source
            .query
            .body
            .iter()
            .skip(1)
            .map(|item| match item {
                cq::BodyItem::Constraint(constraint) => constraint,
                _ => panic!("where clauses stay relational"),
            })
            .collect();
        assert!(constraints.iter().all(|constraint| {
            !matches!(constraint, cq::Constraint::Or(branches) if branches.len() == 1)
        }));
        assert_eq!(
            syn::parse2::<cq::Program>(source.to_token_stream()).unwrap(),
            source
        );
        cq::contract::check(&source).unwrap();
    }
    let source = relational("x in low..high && y != 0");
    let constraints = source
        .query
        .body
        .iter()
        .skip(1)
        .map(ToTokens::to_token_stream);
    let text = quote::quote!(#(#constraints),*).to_string();
    assert!(text.contains("x >= (low)"), "{text}");
    assert!(text.contains("x < (high)"), "{text}");
    assert!(text.contains("y != 0"), "{text}");
}

#[test]
fn literal_disjunctions_use_membership_inside_derived_domains() {
    let disjunction = relational("x == 1 || x == 2 || x == 3");
    let membership = relational("x in [1, 2, 3]");
    assert_eq!(disjunction, membership);
    let source = relational("(x == 1 && (y == 2 || y == 3)) || (x == 4 && (y == 5 || y == 6))");
    let cq::BodyItem::Constraint(constraint) = &source.query.body[1] else {
        panic!()
    };
    let cq::Constraint::Or(branches) = constraint else {
        panic!("the exact two-column disjunction remains")
    };
    assert_eq!(branches.len(), 2);
    assert!(branches.iter().all(|branch| {
        matches!(
            branch.as_slice(),
            [cq::Constraint::Compare { .. }, cq::Constraint::In { .. }]
        )
    }));
    let domains = constraint.implied_domains();
    assert_eq!(domains.len(), 2);
    assert!(
        domains
            .iter()
            .any(|domain| { domain.variables().all(|variable| variable == "y") })
    );
}

#[test]
fn opaque_membership_remains_in_its_written_rust_conjunction() {
    let source: cq::Program = syn::parse_quote! {
        struct Q; relation R(kind:String,size:i32);
        answer(kind) :- R(kind,size), where size != 0,
            if !kind.starts_with("MEDIUM POLISHED") && [49, 14, 23, 45, 19, 3, 36, 9].contains(size);
    };
    assert_eq!(source.query.body.len(), 3);
    let cq::BodyItem::Filter(filter) = &source.query.body[2] else {
        panic!("array membership in Rust is not relational syntax")
    };
    assert_eq!(
        filter.condition,
        syn::parse_quote!(
            !kind.starts_with("MEDIUM POLISHED") && [49, 14, 23, 45, 19, 3, 36, 9].contains(size)
        )
    );
    assert_eq!(cq::normal::normalize(&source).unwrap(), source);
}

#[test]
fn closedness_is_a_free_variable_fact_not_an_expression_whitelist() {
    for operand in ["x.field", "host(x)", "*x", "&x", "values[*x]", "hidden!()"] {
        let q = relational(&format!("x == {operand}"));
        assert!(
            cq::contract::check(&q)
                .unwrap_err()
                .to_string()
                .contains("query variable"),
            "{operand}"
        );
    }
    for operand in [
        "constants.field",
        "module::value()",
        "values[0]",
        "*host",
        "{ let x = host(); x }",
        "(|x| x)(host())",
    ] {
        cq::contract::check(&relational(&format!("x == {operand}"))).unwrap();
    }
}

#[test]
fn malformed_relational_literals_are_rejected() {
    let mut q = relational("x < cutoff");
    let cq::BodyItem::Constraint(c) = &mut q.query.body[1] else {
        panic!()
    };
    c.map_terms(&mut |t| {
        if matches!(t, cq::syntax::constraint::Term::Closed(_)) {
            *t = cq::syntax::constraint::Term::Literal(Box::new(syn::parse_quote!(host())));
        }
    });
    assert!(cq::contract::check(&q).is_err());
}

#[test]
fn arbitrary_rust_guards_stay_opaque_regardless_of_declared_type() {
    for ty in ["String", "&str", "Custom", "StringAlias"] {
        for guard in [
            "name.contains(\"green\")",
            "name.starts_with(\"g\") || name.ends_with(\"n\")",
            "name.contains(pattern())",
            "name.contains(\"green\") || partial(name)",
            "name.as_bytes()[0] == 1",
            "custom(name)",
        ] {
            let q: cq::Program = syn::parse_str(&format!(
                "struct Q; relation R(a:{ty}); answer(name) :- R(name), if {guard};"
            ))
            .unwrap();
            cq::contract::check(&q).unwrap();
            assert_eq!(q.query.body.len(), 2, "{ty}: {guard}");
            assert!(matches!(q.query.body[1], cq::BodyItem::Filter(_)));
            assert!(q.query.body[1].rust_step().is_some());
            assert_eq!(syn::parse2::<cq::Program>(q.to_token_stream()).unwrap(), q);
        }
    }
    let q: cq::Program = syn::parse_quote! {
        struct Q; relation R(a:String,b:i32);
        answer(name) :- R(name,id), if name.contains("green") && *id > 0;
    };
    let cq::BodyItem::Filter(filter) = &q.query.body[1] else {
        panic!("Rust conditions remain whole");
    };
    assert_eq!(
        filter.condition.to_token_stream().to_string(),
        quote::quote!(name.contains("green") && *id > 0).to_string()
    );
}

#[test]
fn disjunctive_domains_require_every_branch_to_constrain_the_variable() {
    let q = relational("(x == 1 && y >= 2) || (x == 3 && y <= 4)");
    let cq::BodyItem::Constraint(c) = &q.query.body[1] else {
        panic!()
    };
    let domains = c.implied_domains();
    assert_eq!(domains.len(), 2);
    assert!(domains.iter().all(|c| {
        c.variables()
            .map(cq::symbol_name)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == 1
    }));
    let q = relational("(x == 1 && y >= 2) || x == 3");
    let cq::BodyItem::Constraint(c) = &q.query.body[1] else {
        panic!()
    };
    let domains = c.implied_domains();
    assert_eq!(domains.len(), 1);
    assert!(domains[0].variables().all(|v| v == "x"));
}

#[test]
fn closed_disjunction_operands_preserve_unary_domains_without_rust_bindings() {
    for predicate in [
        "(x == france && y == germany) || (x == germany && y == france)",
        "(x in lower()..=upper() && y == first()) || (x in lower2()..=upper2() && y == second())",
    ] {
        let source = relational(predicate);
        // These are already `where` operands. Atom normalization is not involved.
        assert_eq!(cq::normal::normalize(&source).unwrap(), source);
        let cq::BodyItem::Constraint(exact) = &source.query.body[1] else {
            panic!()
        };
        let preserved = exact.clone();
        let domains = exact.implied_domains();
        assert_eq!(domains.len(), 2, "{predicate}");
        assert_eq!(exact, &preserved);
        for domain in &domains {
            assert_eq!(
                domain
                    .variables()
                    .map(cq::symbol_name)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                1
            );
            assert!(domain.parameters().next().is_some());
        }
        let mut expected: Vec<_> = exact
            .parameters()
            .map(|expr| expr.to_token_stream().to_string())
            .collect();
        let mut projected: Vec<_> = domains
            .iter()
            .flat_map(|domain| domain.parameters())
            .map(|expr| expr.to_token_stream().to_string())
            .collect();
        expected.sort();
        projected.sort();
        assert_eq!(projected, expected, "closed Rust operands remain unchanged");
    }
    let source = relational("(x == first && y >= lower) || x == second");
    let cq::BodyItem::Constraint(exact) = &source.query.body[1] else {
        panic!()
    };
    let domains = exact.implied_domains();
    assert_eq!(domains.len(), 1);
    assert!(domains[0].variables().all(|variable| variable == "x"));
}

#[test]
fn opaque_predicates_survive_multi_rule_alpha_renaming() {
    let q: cq::Program = syn::parse_quote! {
        struct Q; relation R(name:String);
        answer(name) :- R(name), if name.contains("green");
        answer(name) :- R(name), if custom(name);
    };
    cq::contract::check(&q).unwrap();
    let round_trip: cq::Program = syn::parse2(q.to_token_stream()).unwrap();
    assert_eq!(round_trip, q);
    let normal = cq::normal::normalize(&q).unwrap();
    cq::normal::check(&normal).unwrap();
    assert_eq!(
        cq::blocks::walk(&q.query.body)
            .into_iter()
            .filter(|item| matches!(item, cq::BodyItem::Filter(_)))
            .count(),
        2
    );
}

#[test]
fn macro_forwarded_column_types_and_expressions_remain_opaque() {
    let ty = proc_macro2::Group::new(proc_macro2::Delimiter::None, quote::quote!(String));
    let expr = proc_macro2::Group::new(
        proc_macro2::Delimiter::None,
        quote::quote!(name.contains("green")),
    );
    let q: cq::Program = syn::parse2(quote::quote! {
        struct Q; relation Part(name:#ty);
        answer(name) :- Part(name), if #expr;
    })
    .unwrap();
    assert!(matches!(q.query.body[1], cq::BodyItem::Filter(_)));
    cq::contract::check(&q).unwrap();
}

#[test]
fn total_is_an_ordinary_identifier_but_the_removed_attribute_is_rejected() {
    for source in [
        "struct Q; relation R(a:i32,b:i32); answer(total) :- R(x,y), let total:i32 = *x + *y, if *total > 0;",
        "struct Q; relation R(a:bool,b:bool); answer(total) :- R(total,y), if total && y;",
        "struct Q; relation R(a:bool); answer(total) :- R(total), if total;",
        "struct Q; relation R(a:i32); answer(total) :- R(total), if custom(total);",
    ] {
        let q: cq::Program = syn::parse_str(source).unwrap();
        cq::contract::check(&q).unwrap();
        assert_eq!(syn::parse2::<cq::Program>(q.to_token_stream()).unwrap(), q);
        cq::normal::check(&cq::normal::normalize(&q).unwrap()).unwrap();
    }
    let error = syn::parse_str::<cq::Filter>("#[total] if custom(x)")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("attributes are not supported on guards or relational constraints"),
        "{error}"
    );
}

#[test]
fn normalization_preserves_written_rust_expressions() {
    let query: cq::Program = syn::parse_quote! {
        struct Q;
        relation R(a:String);
        relation S(a:String);
        relation Blocked(a:String);
        answer(name) :-
            R(name),
            if name.contains("green"),
            let length:usize = name.len(),
            !Blocked(name),
            if custom(name, length),
            S(name),
            if name.ends_with("n");
    };
    cq::contract::check(&query).unwrap();
    assert_eq!(cq::normal::normalize(&query).unwrap(), query);
    let positions: Vec<_> = query
        .query
        .body
        .iter()
        .enumerate()
        .filter_map(|(index, item)| item.rust_step().map(|_| index))
        .collect();
    assert_eq!(positions, [1, 2, 4, 6]);
    let step = query.query.body[4].rust_step().unwrap();
    assert_eq!(
        step.free_variables
            .dependencies(&cq::blocks::bound(&query.query.body).into_keys().collect()),
        ["length".into(), "name".into()].into()
    );
}

#[test]
fn multi_rule_alpha_renaming_preserves_closedness_obligations() {
    for operand in ["host(x)", "constants.limit"] {
        let source: cq::Program = syn::parse_str(&format!(
            "struct Q; relation R(a:i32); answer(x) :- R(x), where x == {operand}; answer(x) :- R(x);"
        )).unwrap();
        if operand == "host(x)" {
            assert!(
                cq::contract::check(&source)
                    .unwrap_err()
                    .to_string()
                    .contains("query variable")
            );
        } else {
            cq::contract::check(&source).unwrap();
            assert_eq!(
                syn::parse2::<cq::Program>(source.to_token_stream()).unwrap(),
                source
            );
        }
    }
}

#[test]
fn where_rejects_noncolumns_and_implicit_rust_predicates() {
    for predicate in ["*x == 1", "x.field == 1", "host(x)", "!(x == 1)", "x = 1"] {
        assert!(
            syn::parse_str::<cq::Program>(&format!(
                "struct Q; relation R(a:i32); answer(x) :- R(x), where {predicate};"
            ))
            .is_err(),
            "{predicate}"
        );
    }
    let source = relational("unknown == 1");
    assert!(cq::contract::check(&source).is_err());
}
