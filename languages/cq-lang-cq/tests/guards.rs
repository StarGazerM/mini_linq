use cq_lang_cq as cq;
use quote::ToTokens;

fn query(guard: &str) -> cq::Program {
    syn::parse_str(&format!(
        "struct Q; relation R(a:i32,b:i32); answer(x) :- R(x,y), if {guard};"
    ))
    .unwrap()
}

#[test]
fn the_operand_boundary_is_exact_and_mutations_remain_opaque() {
    for guard in [
        "x < cutoff",
        "*x < cutoff",
        "&x < cutoff",
        "(-3) < x",
        "x == y",
        "x != -2",
        "x <= 4u32",
        "x == true",
        "x == 'a'",
        "x == \"text\"",
        "x < host",
        "host < other",
        "!(x < cutoff || y >= 5)",
    ] {
        let q = query(guard);
        assert!(
            q.query
                .body
                .iter()
                .all(|i| !matches!(i, cq::BodyItem::Filter(_))),
            "{guard}"
        );
        assert_eq!(syn::parse2::<cq::Program>(q.to_token_stream()).unwrap(), q);
    }
    for guard in [
        "*x + 1 < 5",
        "f(*x) < 5",
        "x.abs() < 5",
        "(x as i64) < 5",
        "x[0] < 5",
        "{ *x } < 5",
        "number!() < 5",
        "x.field < 5",
        "x.field.next < 5",
        "x < module::cutoff",
        "x < *cutoff",
        "x < &cutoff",
        "&mut x < 5",
        "**x < 5",
        "&*x < 5",
        "#[allow(unused)] (x < 5)",
        "x < #[allow(unused)] 5",
        "f(*x) || x < 5",
        "!(x < 5 && f(*x))",
    ] {
        let q = query(guard);
        assert_eq!(q.query.body.len(), 2, "{guard}");
        assert!(
            matches!(q.query.body[1], cq::BodyItem::Filter(_)),
            "{guard}"
        );
        assert_eq!(syn::parse2::<cq::Program>(q.to_token_stream()).unwrap(), q);
    }
}

#[test]
fn split_retains_one_ordered_residual_at_the_written_position() {
    for guard in [
        "x < cutoff && f(*x) && y != 0 && g(*y)",
        "f(*x) && x < cutoff && g(*y) && y != 0",
    ] {
        let q = query(guard);
        assert_eq!(q.query.body.len(), 4);
        assert!(matches!(q.query.body[1], cq::BodyItem::Constraint(_)));
        assert!(matches!(q.query.body[2], cq::BodyItem::Constraint(_)));
        let cq::BodyItem::Filter(f) = &q.query.body[3] else {
            panic!()
        };
        assert_eq!(
            f.condition.to_token_stream().to_string(),
            "f (* x) && g (* y)"
        );
        cq::contract::check(&q).unwrap();
        assert_eq!(syn::parse2::<cq::Program>(q.to_token_stream()).unwrap(), q);
    }
    let q = query("(x < cutoff && f(*x)) || y < 5");
    assert_eq!(q.query.body.len(), 2);
    assert!(matches!(q.query.body[1], cq::BodyItem::Filter(_)));
}

#[test]
fn guard_local_bindings_are_not_captured_host_parameters() {
    let q = query("let Some(local) = host(*x) && local < cutoff");
    assert_eq!(q.query.body.len(), 2);
    assert!(matches!(q.query.body[1], cq::BodyItem::Filter(_)));
    let q = query("x < 5 && let Some(local) = host(*x) && local < cutoff");
    assert_eq!(q.query.body.len(), 3);
    let cq::BodyItem::Filter(f) = &q.query.body[2] else {
        panic!()
    };
    assert_eq!(
        f.condition.to_token_stream().to_string(),
        "let Some (local) = host (* x) && local < cutoff"
    );
}

#[test]
fn contract_rejects_unsplit_comparisons_and_malformed_recognized_terms() {
    let mut q = query("x < cutoff");
    q.query.body[1] = cq::BodyItem::Filter(syn::parse_quote!(if x < cutoff));
    assert!(
        cq::contract::check(&q)
            .unwrap_err()
            .to_string()
            .contains("must be split")
    );
    let mut q = query("x < cutoff");
    let cq::BodyItem::Constraint(c) = &mut q.query.body[1] else {
        panic!()
    };
    c.map_terms(&mut |t| {
        if let cq::syntax::constraint::Term::Parameter(_) = t {
            *t = cq::syntax::constraint::Term::Literal(Box::new(syn::parse_quote!(host())));
        }
    });
    assert!(cq::contract::check(&q).is_err());
}

#[test]
fn negation_uses_the_total_order_complements() {
    for (rust, ir) in [
        ("!(x == 1)", "x != 1"),
        ("!(x != 1)", "x = 1"),
        ("!(x < 1)", "x >= 1"),
        ("!(x <= 1)", "x > 1"),
        ("!(x > 1)", "x <= 1"),
        ("!(x >= 1)", "x < 1"),
    ] {
        let q = query(rust);
        let expected: cq::Constraint = syn::parse_str(ir).unwrap();
        match &q.query.body[1] {
            cq::BodyItem::Constraint(c) => assert_eq!(c, &expected),
            cq::BodyItem::Equality(e) => assert_eq!(cq::Constraint::from_equality(e), expected),
            _ => panic!("{rust}"),
        }
    }
}

#[test]
fn total_guards_require_a_typed_allowlist_or_explicit_assertion() {
    for guard in [
        "if name.contains(\"green\")",
        "if name.starts_with(\"g\") || name.ends_with(\"n\")",
        "if name.contains(\"green\") && *id > 0",
        "#[total] if custom(name, id)",
    ] {
        let q: cq::Program = syn::parse_str(&format!(
            "struct Q; relation R(a:String,b:i32); answer(name) :- R(name,id), {guard};"
        ))
        .unwrap();
        cq::contract::check(&q).unwrap();
        assert!(
            q.query
                .body
                .iter()
                .any(|item| matches!(item, cq::BodyItem::Constraint(cq::Constraint::Total { .. }))),
            "{guard}"
        );
        assert_eq!(syn::parse2::<cq::Program>(q.to_token_stream()).unwrap(), q);
    }
    for (ty, guard) in [
        ("Custom", "name.contains(\"green\")"),
        ("String", "name.contains(pattern())"),
        ("String", "name.contains(\"green\") || partial(name)"),
        ("String", "name.as_bytes()[0] == 1"),
    ] {
        let q: cq::Program = syn::parse_str(&format!(
            "struct Q; relation R(a:{ty}); answer(name) :- R(name), if {guard};"
        ))
        .unwrap();
        assert!(
            matches!(q.query.body[1], cq::BodyItem::Filter(_)),
            "{guard}"
        );
    }
}

#[test]
fn disjunctive_domains_require_every_branch_to_constrain_the_variable() {
    let q = query("(x == 1 && y >= 2) || (x == 3 && y <= 4)");
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
    let q = query("(x == 1 && y >= 2) || x == 3");
    let cq::BodyItem::Constraint(c) = &q.query.body[1] else {
        panic!()
    };
    let domains = c.implied_domains();
    assert_eq!(domains.len(), 1);
    assert!(domains[0].variables().all(|v| v == "x"));
}

#[test]
fn total_predicates_survive_multi_rule_alpha_renaming() {
    let q: cq::Program = syn::parse_quote! {
        struct Q; relation R(name:String);
        answer(name) :- R(name), if name.contains("green");
        answer(name) :- R(name), #[total] if custom(name);
    };
    cq::contract::check(&q).unwrap();
    let round_trip: cq::Program = syn::parse2(q.to_token_stream()).unwrap();
    assert_eq!(round_trip, q);
    let normal = cq::normal::normalize(&q).unwrap();
    cq::normal::check(&normal).unwrap();
    assert_eq!(
        cq::blocks::walk(&q.query.body)
            .into_iter()
            .filter(|item| matches!(item, cq::BodyItem::Constraint(cq::Constraint::Total { .. })))
            .count(),
        2
    );
}

#[test]
fn macro_forwarded_column_types_and_expressions_remain_allowlisted() {
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
    assert!(matches!(
        q.query.body[1],
        cq::BodyItem::Constraint(cq::Constraint::Total { .. })
    ));
    cq::contract::check(&q).unwrap();
}

#[test]
fn total_is_an_ordinary_identifier_and_only_the_attribute_asserts_totality() {
    for source in [
        "struct Q; relation R(a:i32,b:i32); answer(total) :- R(x,y), let total:i32 = *x + *y, if *total > 0;",
        "struct Q; relation R(a:bool,b:bool); answer(total) :- R(total,y), if total && y;",
        "struct Q; relation R(a:bool); answer(total) :- R(total), if total;",
        "struct Q; relation R(a:i32); answer(total) :- R(total), #[total] if custom(total);",
    ] {
        let q: cq::Program = syn::parse_str(source).unwrap();
        cq::contract::check(&q).unwrap();
        assert_eq!(syn::parse2::<cq::Program>(q.to_token_stream()).unwrap(), q);
        cq::normal::check(&cq::normal::normalize(&q).unwrap()).unwrap();
    }
    for guard in [
        "#[total(false)] if custom(x)",
        "#[other] if custom(x)",
        "#[total] #[total] if custom(x)",
        "if total custom(x)",
    ] {
        assert!(
            syn::parse_str::<cq::Program>(&format!(
                "struct Q; relation R(a:i32); answer(x) :- R(x), {guard};"
            ))
            .is_err(),
            "{guard}"
        );
    }
    let mut q = query("custom(x)");
    q.query.body[1] = cq::BodyItem::Filter(syn::parse_quote!(#[total] if custom(x)));
    assert!(
        cq::contract::check(&q).is_err(),
        "the contract rejects an assertion not normalized into a total constraint"
    );
}
