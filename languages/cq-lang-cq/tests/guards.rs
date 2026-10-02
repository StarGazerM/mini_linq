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
