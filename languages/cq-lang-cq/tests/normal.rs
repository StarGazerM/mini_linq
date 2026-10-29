use cq_lang_cq as cq;
use quote::{ToTokens, quote};

fn cq(tokens: proc_macro2::TokenStream) -> cq::Program {
    syn::parse2(tokens).unwrap()
}

#[test]
fn normalization_preserves_literals_and_extracts_other_non_variable_arguments() {
    let source = cq(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(x, z) :- R(x, 5), S(_, z), R(x, x), where z > 1;
    });
    let normal = cq::normal::normalize(&source).unwrap();
    let text = normal.query.to_token_stream().to_string();
    // A literal filters the occurrence without introducing a variable or guard.
    assert!(text.starts_with("answer (x , z) : - R (x , 5)"), "{text}");
    // The wildcard is a distinct witness; repeated variables get relational equality.
    assert!(
        text.contains("S (__cq_1_0 , z) , R (x , __cq_2_1) , where x == __cq_2_1 , where z > 1"),
        "{text}"
    );
    cq::normal::check(&normal).unwrap();
    assert!(cq::normal::check(&source).is_err());
    // Normalizing again is the identity.
    assert_eq!(cq::normal::normalize(&normal).unwrap(), normal);
    assert_eq!(
        syn::parse2::<cq::Program>(normal.to_token_stream()).unwrap(),
        normal
    );
}

#[test]
fn closed_atom_operands_are_relational_restrictions_not_rust_bindings() {
    for body in [
        quote!(R(x, { france })),
        quote!({ R(x, { france }) } | { R(x, { germany }) }),
    ] {
        let source = cq(quote! {
            struct Q;
            relation R(key:i32, value:CodeAlias);
            answer(x) :- #body;
        });
        let normal = cq::normal::normalize(&source).unwrap();
        let items = cq::blocks::walk(&normal.query.body);
        assert!(items.iter().all(|item| item.rust_step().is_none()));
        let closed = items
            .iter()
            .filter_map(|item| {
                let cq::BodyItem::Constraint(condition) = item else {
                    return None;
                };
                condition.parameters().next()
            })
            .map(ToTokens::to_token_stream)
            .map(|tokens| tokens.to_string())
            .collect::<Vec<_>>();
        assert!(closed.contains(&quote!({ france }).to_string()));
        if source.query.body.len() == 1 && matches!(source.query.body[0], cq::BodyItem::Block(_)) {
            assert!(closed.contains(&quote!({ germany }).to_string()));
        }
        assert_eq!(cq::normal::normalize(&normal).unwrap(), normal);
        cq::normal::check(&normal).unwrap();
    }
}

#[test]
fn repeated_head_variables_preserve_tuple_positions() {
    // Differential seed 4: normalization rejected a legal all-bare head even
    // though every downstream heading is a set and the output is a tuple.
    let source = cq(quote! {
        struct RepeatHead;
        relation R(x: i32);
        answer(x, x) :- R(x);
    });
    cq::contract::check(&source).unwrap();
    let normal = cq::normal::normalize(&source).unwrap();
    cq::normal::check(&normal).unwrap();
    assert_eq!(normal, source);
    assert_eq!(cq::normal::normalize(&normal).unwrap(), normal);
}

#[test]
fn literal_repeat_and_computed_terms_have_distinct_semantic_kinds() {
    let source = cq(quote! {
        struct Q; relation R(a:i32,b:i32); relation S(a:i32,b:i32);
        relation answer(a:i32,b:i32);
        answer(x, 100 / *x) :- R(x,-1), where x != 0,
            S(10 / *x,y), R(y,y);
    });
    let normal = cq::normal::normalize(&source).unwrap();
    let text = normal.to_token_stream().to_string();
    assert!(text.contains("R (x , - 1)"), "{text}");
    assert!(
        text.contains("where x != 0 , let __cq_2_0 : i32 = 10 / * x , S"),
        "{text}"
    );
    assert!(
        text.contains("R (y , __cq_3_1) , where y == __cq_3_1"),
        "{text}"
    );
    assert!(matches!(
        normal.query.body.last(),
        Some(cq::BodyItem::Let(_))
    ));
    assert!(
        normal
            .query
            .head
            .args
            .iter()
            .all(|a| cq::rust::variable(a).is_some())
    );
    assert_eq!(cq::normal::normalize(&normal).unwrap(), normal);
}

#[test]
fn same_atom_expressions_follow_the_atom_even_before_their_binding_column() {
    for arguments in [quote!(10 / *x, x), quote!(x, 10 / *x)] {
        let source = cq(quote! {
            struct Q; relation R(a:i32,b:i32);
            answer(x) :- R(#arguments);
        });
        let normal = cq::normal::normalize(&source).unwrap();
        assert!(matches!(
            normal.query.body[0],
            cq::BodyItem::Positive { .. }
        ));
        assert!(matches!(normal.query.body[1], cq::BodyItem::Let(_)));
        assert!(matches!(normal.query.body[2], cq::BodyItem::Equality(_)));
        assert_eq!(cq::normal::normalize(&normal).unwrap(), normal);
    }
}

#[test]
fn computed_atom_arguments_cannot_use_a_later_atoms_binder() {
    let source = cq(quote! {
        struct Q; relation R(a:i32); relation S(b:i32);
        answer(x) :- R(10 / *x), S(x);
    });
    let error = cq::contract::check(&source).unwrap_err().to_string();
    assert!(error.contains("`x` is used before it is bound"), "{error}");
}

#[test]
fn rust_metadata_includes_host_captures_and_pattern_bindings() {
    let item: cq::BodyItem = syn::parse_quote!(let (y,z): (i32,i32) = (host(*x), *x + captured));
    let metadata = item.rust_step().unwrap();
    assert_eq!(
        metadata.free_variables.names,
        ["host", "x", "captured"].map(String::from).into()
    );
    assert_eq!(
        metadata
            .bindings
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["y", "z"]
    );
}

#[test]
fn relational_type_obligations_are_left_to_rustc() {
    for source in [
        quote!(struct Q; relation R(value:f64); answer(x) :- R(x);),
        quote!(struct Q; relation R(key:f64,value:i32); answer(y) :- R(1.0,y);),
        quote!(struct Q; relation R(key:f64,value:i32); answer(y) :- R(x,y), R(x,y);),
        quote!(struct Q; relation R(value:i32); relation answer(computed:f64); answer(*x as f64) :- R(x);),
    ] {
        cq::normal::normalize(&cq(source)).unwrap();
    }
    let malformed: cq::Equality = cq::Equality {
        left: syn::parse_quote!(x),
        eq_token: Default::default(),
        right: cq::EqualityValue::Literal(syn::parse_quote!(host())),
    };
    assert!(malformed.check().is_err());
}

#[test]
fn a_host_capture_in_the_head_becomes_a_final_binding() {
    let source = cq(quote! {
        struct Q; relation R(a:i32); relation answer(a:i32);
        answer(host_value) :- R(x);
    });
    assert!(cq::normal::check(&source).is_err());
    let normal = cq::normal::normalize(&source).unwrap();
    assert!(matches!(
        normal.query.body.last(),
        Some(cq::BodyItem::Let(_))
    ));
    cq::normal::check(&normal).unwrap();
}

#[test]
fn tuple_annotations_do_not_supply_destructured_output_types() {
    let source = cq(quote! {
        struct Q; relation R(t:i32);
        answer(x,y) :- R(t), let (x,y):(i32,i32)=(1,2);
    });
    assert!(
        cq::normal::normalize(&source)
            .unwrap_err()
            .to_string()
            .contains("declare the result")
    );
    let declared = cq(quote! {
        struct Q; relation R(t:i32); relation answer(x:f64,y:i32);
        answer(x,y) :- R(t), let (x,y):(f64,i32)=(1.0,2);
    });
    cq::normal::normalize(&declared).unwrap();
}

#[test]
fn tuple_binding_types_remain_host_inferred() {
    let binding: cq::Let = syn::parse_quote!(
        let (a, (ref b, ref mut c), z):(i32,(f64,String),u8)=(1,(2.0,String::new()),0)
    );
    let types: Vec<_> = binding
        .binding_types()
        .into_iter()
        .map(|(name, ty)| (name.to_string(), ty.to_token_stream().to_string()))
        .collect();
    assert_eq!(
        types,
        [("a", "_"), ("b", "_"), ("c", "_"), ("z", "_")]
            .map(|(name, ty)| (name.to_owned(), ty.to_owned()))
    );
    let binding: cq::Let = syn::parse_quote!(let (a,..,z):(i32,u8,u16,f64)=(1,2,3,4.0));
    let types: Vec<_> = binding
        .binding_types()
        .into_iter()
        .map(|(name, ty)| (name.to_string(), ty.to_token_stream().to_string()))
        .collect();
    assert_eq!(
        types,
        [
            ("a".to_owned(), "_".to_owned()),
            ("z".to_owned(), "_".to_owned())
        ]
    );
}

#[test]
fn tuple_annotations_do_not_guess_named_types_or_reference_match_ergonomics() {
    for binding in [
        syn::parse_quote!(let (x,y):Pair=pair),
        syn::parse_quote!(let (x,y): &(i32,f64)=pair),
        syn::parse_quote!(let &(x,y): &(i32,f64)=pair),
    ] {
        let binding: cq::Let = binding;
        assert!(
            binding
                .binding_types()
                .iter()
                .all(|(_, ty)| matches!(ty, syn::Type::Infer(_)))
        );
    }
}

#[test]
fn relational_comparisons_round_trip_and_capture_only_host_names() {
    for body in [
        "where x == y, where x != y, where x < y, where x <= y, where x > y, where x >= y",
        "where x > -3, where x <= 5u32, where x > lower, where x >= upper",
        "where x >= -3 && x <= upper, where x in [-1, 2, 3]",
        "where (x >= lower && x <= upper && (y == 1 || y == 2)) || (x != y && y >= 3)",
        "where (x == 1) || (x == 2) || (y == 3)",
        "where (x < 0 || y < 0) && (x > -3 || y > -3)",
    ] {
        let source: cq::Program = syn::parse_str(&format!(
            "struct Q; relation R(a:i32,b:i32); answer(x) :- R(x,y), {body};"
        ))
        .unwrap();
        cq::normal::check(&source).unwrap();
        let printed = source.to_token_stream();
        assert!(!printed.to_string().contains('@'));
        assert_eq!(syn::parse2::<cq::Program>(printed).unwrap(), source);
        assert_eq!(cq::normal::normalize(&source).unwrap(), source);
        assert!(
            source
                .query
                .body
                .iter()
                .all(|i| !matches!(i, cq::BodyItem::Filter(_)))
        );
        for c in source.query.body.iter().filter_map(|i| match i {
            cq::BodyItem::Constraint(c) => Some(c),
            _ => None,
        }) {
            assert!(c.variables().all(|v| v == "x" || v == "y"));
            assert!(
                c.parameters()
                    .all(|v| ["lower", "upper"].contains(&v.to_token_stream().to_string().as_str()))
            );
        }
    }
}

#[test]
fn custom_cq_constraint_syntax_is_rejected() {
    for body in [
        "x = 1",
        "x != y",
        "*x < 1",
        "x <= upper",
        "x between -1 and 3",
        "x in {1,2}",
        "(x = 1) or (x = 2)",
        "(x < 1 and y < 2) or (x = 1)",
        "x == 1",
        "(x < 1) || (y < 2)",
        "@cutoff < x",
        "if x between 1 and 3",
        "if x in {1,2}",
    ] {
        assert!(
            syn::parse_str::<cq::Program>(&format!(
                "struct Q; relation R(a:i32,b:i32); answer(x) :- R(x,y), {body};"
            ))
            .is_err(),
            "{body}"
        );
    }
}

#[test]
fn constraint_scope_and_premises_are_local_contract_obligations() {
    for source in [
        "struct Q; relation R(a:i32); answer(x) :- where x < cutoff, R(x);",
        "struct Q; relation R(a:i32); answer(x) :- R(x), where y < cutoff, let y:i32 = 1;",
    ] {
        let p: cq::Program = syn::parse_str(source).unwrap();
        let e = cq::normal::check(&p).unwrap_err().to_string();
        assert!(e.contains("before it is bound"), "{e}");
    }
    for ty in ["f64", "(i32,f32)", "fn()"] {
        let p: cq::Program = syn::parse_str(&format!(
            "struct Q; relation R(a:{ty},b:i32); answer(y) :- R(x,y), where x < cutoff;"
        ))
        .unwrap();
        cq::normal::check(&p).unwrap();
    }
    let mut p: cq::Program =
        syn::parse_quote! { struct Q; relation R(a:i32); answer(x) :- R(x), where x < cutoff; };
    let cq::BodyItem::Constraint(c) = &mut p.query.body[1] else {
        panic!()
    };
    c.map_terms(&mut |t| {
        if matches!(t, cq::syntax::constraint::Term::Closed(_)) {
            *t = cq::syntax::constraint::Term::Closed(syn::parse_quote!(x));
        }
    });
    assert!(
        cq::normal::check(&p)
            .unwrap_err()
            .to_string()
            .contains("query variable")
    );
}

#[test]
fn literals_preserve_argument_positions_and_do_not_bind_head_or_block_names() {
    let source = cq(quote! {
        struct Q; relation R(a:i32,b:bool,c:&str,d:i64);
        answer(x) :- R(-7,true,"fixed",x);
    });
    assert_eq!(cq::normal::normalize(&source).unwrap(), source);
    cq::normal::check(&source).unwrap();
    assert!(source.query.is_basic());
    let literal_head = cq(quote! {
        struct Q; relation R(a:i32);
        relation answer(a:i32);
        answer(7) :- R(7);
    });
    assert!(!literal_head.query.is_basic());
    assert!(cq::normal::check(&literal_head).is_err());
    let normal = cq::normal::normalize(&literal_head).unwrap();
    assert!(matches!(
        normal.query.body[0],
        cq::BodyItem::Positive { .. }
    ));
    assert!(matches!(normal.query.body[1], cq::BodyItem::Let(_)));
    let shifted_float = cq(quote! {
        struct Q; relation R(a:i32,b:f64);
        answer(x) :- R(7,x);
    });
    cq::normal::check(&shifted_float).unwrap();
}
