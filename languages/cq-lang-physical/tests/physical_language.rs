use cq_lang_physical as physical;
use quote::ToTokens;
#[test]
fn constant_occurrences_drop_source_columns_and_require_retention() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation R(a:i32,b:i32);
        r = let canon rename R@0 {a -> x} constants {b = -7};
        return r as answer(x);
    }};
    let facts = physical::contract::check(&source).unwrap();
    assert_eq!(facts.headings["r"], ["x"]);
    assert_eq!(facts.renames["r"].len(), 1);
    assert_eq!(
        syn::parse2::<physical::Program>(source.to_token_stream()).unwrap(),
        source
    );
    for definition in [
        syn::parse_quote!(r = pipe canon rename R@0 {a -> x} constants {b = -7};),
        syn::parse_quote!(r = let canon rename R@0 {a -> x} constants {a = 7};),
        syn::parse_quote!(r = let canon rename R@0 {a -> x} constants {missing = 7};),
        syn::parse_quote!(r = let canon rename R@0 {a -> x} constants {b = x};),
        syn::parse_quote!(r = let canon rename R@0 {a -> x} constants {};),
    ] {
        let mut invalid = source.clone();
        invalid.program.definitions[0] = definition;
        assert!(physical::contract::check(&invalid).is_err());
    }
}

const SOURCE: &str = "physical {
    relation R(a:i32,b:i32); relation S(b:i32,c:i32);
    r = pipe rename R@0 {a -> x,b -> y}; s = let rename S@1 {b -> y,c -> z};
    j = pipe probe first [y] r,s; f = pipe guard j if (*x < *z);
    e = pipe bind f with w:i32 = (*x + *z);
    p = pipe canon project e keep {x,w}; return p as answer(x,w);
}";
#[test]
fn complete_program_round_trip_and_analysis() {
    let source: physical::Program = syn::parse_str(SOURCE).unwrap();
    let a = physical::contract::check(&source).unwrap();
    assert_eq!(
        syn::parse2::<physical::Program>(source.to_token_stream()).unwrap(),
        source
    );
    assert_eq!(a.headings["p"], ["w", "x"]);
    assert_eq!(a.hosts["r"], "R");
    assert_eq!(a.renames["r"]["x"], "a");
    assert_eq!(a.rows["p"].bindings["x"], "x");
    assert!(a.rows["p"].computed.contains("w"));
    assert_eq!(a.key_reads["s"], [("j".into(), vec!["y".into()])]);
    assert!(a.scanned.contains("r"));
    assert!(!a.scanned.contains("s"));
}
#[test]
fn invalid_operators_modes_and_heads_report_the_construct() {
    for (before, after, construct) in [
        ("rename R", "rename Missing", "Missing"),
        ("a -> x,b -> y", "a -> x,a -> y", "r"),
        ("a -> x,b -> y", "a -> x,b -> x", "r"),
        ("a -> x,b -> y", "a -> x", "r"),
        ("a -> x,b -> y", "a -> x,q -> y", "r"),
        ("r,s", "r,missing", "missing"),
        ("s = let", "r = let", "r"),
        ("probe first [y]", "probe first [x]", "j"),
        ("probe first [y]", "probe first [y,y]", "j"),
        ("= (*x + *z)", "= (*q + *w)", "e"),
        ("with w:i32", "with x:i32", "e"),
        ("keep {x,w}", "keep {x,missing}", "p"),
        ("keep {x,w}", "keep {x,x}", "p"),
        ("answer(x,w)", "answer(x)", "p"),
        ("return p", "return missing", "missing"),
        (
            "return p",
            "unused = pipe rename R@0 {a -> a,b -> b}; return p",
            "unused",
        ),
        ("guard j if (*x < *z)", "guard p if (*x < *z)", "p"),
    ] {
        let changed: physical::Program = syn::parse_str(&SOURCE.replace(before, after)).unwrap();
        let error = physical::contract::check(&changed).unwrap_err().to_string();
        assert!(error.contains(construct), "{construct}: {error}");
    }
}
#[test]
fn merge_and_point_record_supply_kinds() {
    let source: physical::Program = syn::parse_str("physical { relation R(a:i32); relation S(a:i32); r = let rename R@0 {a -> x}; s = let rename S@1 {a -> x}; j = pipe merge[x] r,s; f = pipe point select j if (x = 1); return f as answer(x); }").unwrap();
    let a = physical::contract::check(&source).unwrap();
    assert_eq!(a.merge_reads["r"][0].0, "j");
    assert_eq!(a.merge_reads["s"][0].0, "j");
    assert_eq!(a.keyed_reads["j"][0].2, physical::syntax::SupplyKind::Const);
}

const HASH_CHAIN: &str = "physical {
    relation L(a: i32); relation R(a: i32, b: i32); relation S(b: i32, c: i32);
    l = let canon rename L@0 {a -> a};
    r = let canon rename R@1 {a -> a, b -> b};
    s = let canon rename S@2 {b -> b, c -> c};
    j0 = pipe canon probe first [a] l, r;
    j1 = pipe canon probe first [b] j0, s;
    x = pipe canon semi eager [a] j1, l;
    p = pipe canon project x keep {a, c}; return p as answer(a, c);
}";

#[test]
fn intermediate_lets_and_pipes_admit_canon_and_semijoins_record_key_reads() {
    let source: physical::Program = syn::parse_str(HASH_CHAIN).unwrap();
    let analysis = physical::contract::check(&source).unwrap();
    assert_eq!(
        analysis.canon,
        ["l", "r", "s", "j0", "j1", "x", "p"]
            .map(String::from)
            .into()
    );
    assert_eq!(analysis.key_reads["l"], [("x".into(), vec!["a".into()])]);
    assert_eq!(physical::ordinals(&analysis, "r", &["a".into()]), [0]);
}

#[test]
fn a_merge_result_read_by_key_stays_materialized() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation L(a: i32); relation R(a: i32, b: i32); relation S(b: i32, c: i32);
        l = let rename L@0 {a -> a};
        r = let rename R@1 {a -> a, b -> b};
        s = let rename S@2 {b -> b, c -> c};
        j0 = let merge[a] l, r;
        j1 = let merge[b] j0, s;
        x = pipe semi eager [a] l, j1;
        p = pipe canon project x keep {a}; return p as answer(a);
    }};
    let analysis = physical::contract::check(&source).unwrap();
    assert_eq!(analysis.modes["j1"], physical::ModeKind::Let);
    assert_eq!(analysis.key_reads["j1"], [("x".into(), vec!["a".into()])]);
    let mut invalid = source;
    invalid.program.definitions[4] = syn::parse_quote!(j1 = pipe merge[b] j0, s;);
    let error = physical::contract::check(&invalid).unwrap_err().to_string();
    assert!(error.contains("j1"), "{error}");
}

#[test]
fn point_reads_require_literal_keys_and_canon_host_pipes_require_linear_reads() {
    let mut p: physical::Program = syn::parse_quote!(physical {
        relation R(a:i32,b:i32); r = pipe rename R@0 {a -> x,b -> y};
        w = pipe canon point select r if (x = 2); return w as answer(x,y);
    });
    physical::contract::check(&p).unwrap();
    p.program.definitions[1] = syn::parse_quote!(w = pipe canon point select r if (x = y););
    assert!(
        physical::contract::check(&p)
            .unwrap_err()
            .to_string()
            .contains("literal")
    );
    let mut p: physical::Program = syn::parse_str(HASH_CHAIN).unwrap();
    p.program.definitions[0].mode = syn::parse_quote!(pipe);
    assert!(physical::contract::check(&p).is_err());
    p.program.definitions[0].canon = None;
    physical::contract::check(&p).unwrap();
}

#[test]
fn rust_branches_are_independent_and_types_are_host_obligations() {
    let p: physical::Program = syn::parse_quote!(physical {
        relation R(a:i32); r = pipe rename R@0 {a -> x};
        g = pipe guard r if (*x != 0);
        b = let bind r with y:i32 = (10 / *x);
        j = pipe probe first [x] g,b; return j as answer(x,y);
    });
    physical::contract::check(&p).unwrap();
    let p: physical::Program = syn::parse_quote!(physical {
        relation R(price:f64); r = pipe rename R@0 {price -> price};
        p = pipe canon point select r if (price = 0.0); return p as answer(price);
    });
    physical::contract::check(&p).unwrap();
    assert!(
        syn::parse_str::<physical::Program>(
            "physical { e = pipe extend u with {x:i32} value (1); return e as answer(x); }"
        )
        .is_err()
    );
}

#[test]
fn rewrite_accepts_physical_choices_and_rejects_changed_logical_operands() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation R(a:i32); relation S(a:i32);
        r = let rename R@0 {a -> x}; s = let rename S@1 {a -> x};
        j = pipe probe first [x] r,s; return j as answer(x);
    }};
    let mut merged = source.clone();
    merged.program.definitions[2].operator = syn::parse_quote!(merge[x] r,s);
    physical::check_rewrite(&source, &merged).unwrap();
    physical::check_rewrite(&merged, &source).unwrap();
    for operator in [
        syn::parse_quote!(merge[x] s,r),
        syn::parse_quote!(semi eager [x] r,s),
    ] {
        let mut target = source.clone();
        target.program.definitions[2].operator = operator;
        physical::contract::check(&target).unwrap();
        assert!(physical::check_rewrite(&source, &target).is_err());
    }
}

#[test]
fn rewrite_preserves_selection_and_rust_payload_without_an_ra_dependency() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation R(a:i32); r = pipe rename R@0 {a -> x};
        s = pipe select r if (x = 2);
        g = pipe guard s if (accept(x)); return g as answer(x);
    }};
    let mut point = source.clone();
    point.program.definitions[1].operator = syn::parse_quote!(point select r if (x = 2));
    physical::check_rewrite(&source, &point).unwrap();
    let mut changed_predicate = point.clone();
    changed_predicate.program.definitions[1].operator =
        syn::parse_quote!(point select r if (x = 3));
    let mut changed_payload = point;
    changed_payload.program.definitions[2].operator = syn::parse_quote!(guard s if (other(x)));
    for target in [changed_predicate, changed_payload] {
        physical::contract::check(&target).unwrap();
        assert!(physical::check_rewrite(&source, &target).is_err());
    }
}

#[test]
fn generator_bind_round_trip_checks_and_key_transfer() {
    let source: physical::Program = syn::parse_quote! { physical {
        u = pipe unit;
        g = pipe bind u with for x = ([1, 1, 2]);
        return canon g as answer(x);
    } };
    let facts = physical::contract::check(&source).unwrap();
    assert!(facts.rows["g"].computed.contains("x"));
    assert_eq!(
        syn::parse2::<physical::Program>(source.to_token_stream()).unwrap(),
        source
    );
    let op = &source.program.definitions[1].operator;
    assert!(physical::inherited_key(op, &facts.headings["g"], &[], |_| Some(Vec::new())).is_none());
    for replacement in [
        "bind u with for x = ([*x])",
        "bind absent with for x = ([1])",
    ] {
        let mut invalid = source.clone();
        invalid.program.definitions[1].operator = syn::parse_str(replacement).unwrap();
        assert!(physical::contract::check(&invalid).is_err());
    }
    let mut scalar = source;
    let physical::Operator::Bind(bind) = &mut scalar.program.definitions[1].operator else {
        panic!()
    };
    bind.for_token = None;
    assert_eq!(
        physical::inherited_key(
            &scalar.program.definitions[1].operator,
            &["x".into()],
            &[],
            |_| Some(Vec::new())
        ),
        Some(Vec::new())
    );
}

#[test]
fn interleaved_context_facts_round_trip_and_remain_distinct() {
    let program: physical::Program = syn::parse_quote! { physical {
        relation R(a:i32); relation S(a:i32);
        host_set R discharged;
        consumer canonical;
        host_set S;
        r = pipe rename R@0 {a -> x};
        s = pipe rename S@1 {a -> x};
        u = pipe union r,s;
        return u as answer(x);
    }};
    let facts = physical::contract::check(&program).unwrap();
    assert!(facts.consumer_canonical);
    assert_eq!(facts.discharged_host_sets, ["R".to_owned()].into());
    assert_eq!(
        syn::parse2::<physical::Program>(program.to_token_stream()).unwrap(),
        program
    );
    let mut certified = program.clone();
    certified.program.host_sets[0].discharged = None;
    assert_ne!(certified, program);
    assert!(physical::check_rewrite(&program, &certified).is_err());
    let mut duplicate = program.clone();
    duplicate
        .program
        .host_sets
        .push(program.program.host_sets[0].clone());
    assert!(physical::contract::check(&duplicate).is_err());
    let mut invalid = program;
    invalid.program.return_canon = Some(Default::default());
    assert!(physical::contract::check(&invalid).is_err());
}

#[test]
fn context_fact_grammar_rejects_duplicates_and_malformed_packets() {
    assert!(syn::parse_str::<physical::Program>(
        "physical { consumer canonical; consumer canonical; u = pipe unit; return u as answer(); }"
    ).is_err());
    for arguments in [
        "consumer = canonical, consumer = canonical",
        "host_set R discharged, host_set R discharged",
        "host_set R",
        "consumer canonical",
        "consumer = unknown",
        "host_set R discharged consumer = canonical",
    ] {
        assert!(
            syn::parse_str::<physical::FactPacket>(arguments).is_err(),
            "{arguments}"
        );
    }
    let packet: physical::FactPacket =
        syn::parse_str("host_set R discharged, consumer = canonical,").unwrap();
    assert!(packet.context().consumer_canonical);
    assert!(packet.context().discharged_host_sets.contains("R"));
}
