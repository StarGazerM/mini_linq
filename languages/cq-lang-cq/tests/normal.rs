use cq::normal::Kernel;
use cq_lang_cq as cq;
use quote::{ToTokens, quote};

fn cq(tokens: proc_macro2::TokenStream) -> cq::Program {
    syn::parse2(tokens).unwrap()
}

#[test]
fn normalization_moves_every_non_variable_argument_out_of_atoms() {
    let source = cq(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(x, z) :- R(x, 5), S(_, z), R(x, x), if z > 1;
    });
    let normal = cq::normal::normalize(&source).unwrap();
    let text = normal.query.to_token_stream().to_string();
    // The constant becomes a relational selection at its atom position.
    assert!(
        text.starts_with("answer (x , z) : - R (x , __cq_0_1) , if __cq_0_1 == 5"),
        "{text}"
    );
    // The wildcard is a distinct witness; repeated variables get relational equality.
    assert!(
        text.contains("S (__cq_1_0 , z) , R (x , __cq_2_1) , if x == __cq_2_1 , if z > 1"),
        "{text}"
    );
    cq::normal::check(&normal).unwrap();
    assert!(cq::normal::check(&source).is_err());
    // Normalizing again is the identity.
    assert_eq!(cq::normal::normalize(&normal).unwrap(), normal);
}

#[test]
fn kernels_compare_up_to_renaming_of_variables() {
    let one = cq(quote! {
        struct Q; relation R(a: i32, b: i32);
        answer(x) :- R(x, y), if true;
    });
    let two = cq(quote! {
        struct Q; relation R(a: i32, b: i32);
        answer(p) :- R(p, q), if true;
    });
    let three = cq(quote! {
        struct Q; relation R(a: i32, b: i32);
        answer(p) :- R(q, p), if true;
    });
    assert_eq!(Kernel::of(&one), Kernel::of(&two));
    assert_ne!(Kernel::of(&one), Kernel::of(&three));
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
        answer(x, 100 / *x) :- R(x,-1), if *x != 0,
            S(10 / *x,y), R(y,y);
    });
    let normal = cq::normal::normalize(&source).unwrap();
    let text = normal.to_token_stream().to_string();
    assert!(text.contains("if __cq_0_1 == - 1"), "{text}");
    assert!(
        text.contains("if x != 0 , let __cq_2_0 : i32 = 10 / * x , S"),
        "{text}"
    );
    assert!(
        text.contains("R (y , __cq_3_1) , if y == __cq_3_1"),
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
fn kernels_preserve_cross_kind_chain_and_relational_predecessors() {
    let source = cq(quote! {
        struct Q; relation R(a:i32); relation S(a:i32);
        answer(x) :- R(x), S(w), if { *x != 0 }, let y:i32 = 10 / *x;
    });
    let original = Kernel::of(&source);
    let mut changed = source.clone();
    let item = changed.query.body[2].clone();
    changed.query.body[2] = changed.query.body[3].clone();
    changed.query.body[3] = item;
    assert!(!Kernel::of(&changed).refines(&original));
    let mut changed = source.clone();
    let item = changed.query.body[1].clone();
    changed.query.body[1] = changed.query.body[2].clone();
    changed.query.body[2] = item;
    assert!(!Kernel::of(&changed).refines(&original));
    let source = cq(quote! {
        struct Q; relation R(a:i32); relation S(a:i32);
        answer(x) :- R(x), if { *x != 0 }, S(w);
    });
    let mut earlier = source.clone();
    let item = earlier.query.body[1].clone();
    earlier.query.body[1] = earlier.query.body[2].clone();
    earlier.query.body[2] = item;
    assert!(Kernel::of(&earlier).refines(&Kernel::of(&source)));
}

#[test]
fn equality_and_result_dedup_reject_known_non_leibniz_columns() {
    for source in [
        quote!(struct Q; relation R(value:f64); answer(x) :- R(x);),
        quote!(struct Q; relation R(key:f64,value:i32); answer(y) :- R(1.0,y);),
        quote!(struct Q; relation R(key:f64,value:i32); answer(y) :- R(x,y), R(x,y);),
        quote!(struct Q; relation R(value:i32); relation answer(computed:f64); answer(*x as f64) :- R(x);),
    ] {
        let error = cq::normal::normalize(&cq(source)).unwrap_err().to_string();
        assert!(
            error.contains("Leibniz admissibility of a column type")
                && error.contains("Rust code cannot distinguish `==`-equal values")
                && error.contains("f64/f32 (0.0 == -0.0)")
                && error.contains("hand-written lenient Eq")
                && error.contains("column"),
            "{error}"
        );
    }
    let malformed: cq::Equality = cq::Equality {
        left: syn::parse_quote!(x),
        eq_token: Default::default(),
        right: cq::EqualityValue::Literal(syn::parse_quote!(host())),
    };
    assert!(malformed.check().is_err());
}

#[test]
fn kernel_never_alpha_rewrites_opaque_host_names_or_fields() {
    let source = cq(quote! {
        struct Q; relation R(a:i32);
        answer(x) :- R(x), if host(v0,x) && other(v0,x);
    });
    let changed = cq(quote! {
        struct Q; relation R(a:i32);
        answer(x) :- R(x), if host(x,v0) && other(v0,x);
    });
    assert!(!Kernel::of(&changed).refines(&Kernel::of(&source)));
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
fn tuple_annotations_expose_known_float_columns_to_the_leibniz_check() {
    for source in [
        quote! {
            struct Q; relation R(t:i32);
            answer(t) :- R(t), let (x,y):(f64,i32)=(1.0,2), if * x == 1.0;
        },
        quote! {
            struct Q; relation R(t:i32);
            answer(x) :- R(t), let (x,y):(f64,i32)=(1.0,2);
        },
        quote! {
            struct Q; relation R(t:i32);
            answer(x) :- R(t), let (_, (ref x, _)):(i32,(f64,i32))=(0,(1.0,2));
        },
    ] {
        let error = cq::normal::normalize(&cq(source)).unwrap_err().to_string();
        assert!(
            error.contains("column `x`") && error.contains("f64") && error.contains("Eq"),
            "{error}"
        );
        assert!(!error.contains("declare the result"), "{error}");
    }
    let source = cq(quote! {
        struct Q; relation R(t:i32);
        answer(x,y) :- R(t), let (x,y):(i32,i32)=(1,2);
    });
    cq::normal::normalize(&source).unwrap();
}

#[test]
fn tuple_binding_types_recurse_and_preserve_explicit_borrow_modes() {
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
        [
            ("a", "i32"),
            ("b", "& f64"),
            ("c", "& mut String"),
            ("z", "u8")
        ]
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
            ("a".to_owned(), "i32".to_owned()),
            ("z".to_owned(), "f64".to_owned())
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
fn rust_comparisons_round_trip_and_capture_only_host_names() {
    for body in [
        "if x == y, if *x != &y, if x < y, if x <= y, if x > y, if x >= y",
        "if -3 < *x, if x <= 5u32, if lower < x, if x >= upper, if lower == upper",
        "if x >= -3 && x <= upper, if x == -1 || x == 2 || x == 3",
        "if (x >= lower && x <= upper && (y == 1 || y == 2)) || (x != y && y >= 3)",
        "if (x == 1) || (x == 2) || (y == 3)",
        "if !(!(x == y) || (x < lower && !(y >= upper)))",
        "if (x < 0 || y < 0) && (x > -3 || y > -3)",
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
            assert!(c.parameters().all(|v| v == "lower" || v == "upper"));
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
        "struct Q; relation R(a:i32); answer(x) :- if * x < cutoff, R(x);",
        "struct Q; relation R(a:i32); answer(x) :- R(x), if * y < cutoff, let y:i32 = 1;",
    ] {
        let p: cq::Program = syn::parse_str(source).unwrap();
        let e = cq::normal::check(&p).unwrap_err().to_string();
        assert!(e.contains("before it is bound"), "{e}");
    }
    for ty in ["f64", "(i32,f32)", "fn()"] {
        let p: cq::Program = syn::parse_str(&format!(
            "struct Q; relation R(a:{ty},b:i32); answer(y) :- R(x,y), if * x < cutoff;"
        ))
        .unwrap();
        let e = cq::normal::check(&p).unwrap_err().to_string();
        assert!(e.contains("Eq") || e.contains("Ord"), "{e}");
    }
    let mut p: cq::Program =
        syn::parse_quote! { struct Q; relation R(a:i32); answer(x) :- R(x), if * x < cutoff; };
    let cq::BodyItem::Constraint(c) = &mut p.query.body[1] else {
        panic!()
    };
    c.map_terms(&mut |t| {
        if matches!(t, cq::syntax::constraint::Term::Parameter(_)) {
            *t = cq::syntax::constraint::Term::Parameter(syn::parse_quote!(x));
        }
    });
    assert!(
        cq::normal::check(&p)
            .unwrap_err()
            .to_string()
            .contains("query variable")
    );
}
