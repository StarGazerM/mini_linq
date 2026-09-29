use cq_lang_physical as physical;
use quote::ToTokens;
const SOURCE: &str = "physical {
    relation R(a:i32,b:i32); relation S(b:i32,c:i32);
    r = pipe rename R {a -> x,b -> y}; s = let rename S {b -> y,c -> z};
    j = pipe probe[y] r,s; f = pipe select j if (x < z);
    e = pipe extend f with {w:i32} value (x + z);
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
        ("probe[y]", "probe[x]", "j"),
        ("probe[y]", "probe[y,y]", "j"),
        ("value (x + z)", "value (q + w)", "e"),
        ("{w:i32}", "{x:i32}", "e"),
        ("keep {x,w}", "keep {x,missing}", "p"),
        ("keep {x,w}", "keep {x,x}", "p"),
        ("answer(x,w)", "answer(x)", "p"),
        ("return p", "return missing", "missing"),
        (
            "return p",
            "unused = pipe rename R {a -> a,b -> b}; return p",
            "unused",
        ),
        ("select j if (x < z)", "select p if (x < z)", "p"),
    ] {
        let changed: physical::Program = syn::parse_str(&SOURCE.replace(before, after)).unwrap();
        let error = physical::contract::check(&changed).unwrap_err().to_string();
        assert!(error.contains(construct), "{construct}: {error}");
    }
}
#[test]
fn merge_and_range_record_ordered_reads() {
    let source: physical::Program = syn::parse_str("physical { relation R(a:i32); relation S(a:i32); r = let rename R {a -> x}; s = let rename S {a -> x}; j = let merge[x] r,s; f = pipe range[x] j if (x > 1); return f as answer(x); }").unwrap();
    let a = physical::contract::check(&source).unwrap();
    assert_eq!(a.merge_reads["r"][0].0, "j");
    assert_eq!(a.merge_reads["s"][0].0, "j");
    assert_eq!(a.range_reads["j"], [("f".into(), "x".into())]);
    let mut invalid = source.clone();
    invalid.program.definitions[2].mode = syn::parse_quote!(pipe);
    assert!(physical::contract::check(&invalid).is_err());
}

const HASH_CHAIN: &str = "physical {
    relation L(a: i32); relation R(a: i32, b: i32); relation S(b: i32, c: i32);
    l = let canon rename L {a -> a};
    r = let canon rename R {a -> a, b -> b};
    s = let canon rename S {b -> b, c -> c};
    j0 = pipe canon probe[a] l, r;
    j1 = pipe canon probe[b] j0, s;
    x = pipe canon semi[a] j1, l;
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
        l = let rename L {a -> a};
        r = let rename R {a -> a, b -> b};
        s = let rename S {b -> b, c -> c};
        j0 = let merge[a] l, r;
        j1 = let merge[b] j0, s;
        x = pipe semi[a] l, j1;
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
fn host_ranges_require_layout_and_canon_host_pipes_require_linear_reads() {
    let mut p: physical::Program = syn::parse_quote!(physical {
        relation R(a:i32,b:i32); r = pipe rename R {a -> x,b -> y};
        w = pipe canon range[x] r if (x >= 2); return w as answer(x,y);
    });
    physical::contract::check(&p).unwrap();
    p.program.definitions[1] = syn::parse_quote!(w = pipe canon range[y] r if (y >= 2););
    assert!(
        physical::contract::check(&p)
            .unwrap_err()
            .to_string()
            .contains("leading declared column")
    );
    let mut p: physical::Program = syn::parse_str(HASH_CHAIN).unwrap();
    p.program.definitions[0].mode = syn::parse_quote!(pipe);
    assert!(physical::contract::check(&p).is_err());
    p.program.definitions[0].canon = None;
    physical::contract::check(&p).unwrap();
}
