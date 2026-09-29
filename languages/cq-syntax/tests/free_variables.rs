use cq_syntax::rust::{bindings, free_variables};
use std::collections::BTreeSet;

fn names(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn lexical_free_names_cover_rust_binding_and_expression_shapes() {
    let cases: &[(&str, &[&str])] = &[
        ("x + y * x", &["x", "y"]),
        ("accept(x.field, y.method(z))", &["accept", "x", "y", "z"]),
        ("Pair { label: x, y, ..base }", &["base", "x", "y"]),
        ("{ let x = y; x + z }", &["y", "z"]),
        ("{ let x = x + 1; x + y }", &["x", "y"]),
        ("{ { let x = y; x }; x }", &["x", "y"]),
        ("(|x, y| x + y + z)(a, b)", &["a", "b", "z"]),
        ("(|x: [u8; N]| x[0])(bytes)", &["N", "bytes"]),
        ("for x in x { consume(x, y); }", &["consume", "x", "y"]),
        (
            "match x { Some(y) if y > z => y, _ => y }",
            &["x", "y", "z"],
        ),
        ("if let Some(x) = y { x } else { x }", &["x", "y"]),
        (
            "if let Some(x) = y && x > z { x } else { x }",
            &["x", "y", "z"],
        ),
        (
            "{ while let Some(x) = source.next() { consume(x); } x }",
            &["consume", "source", "x"],
        ),
        ("{ let Some(x) = y else { return x }; x }", &["x", "y"]),
        ("{ let (x, ref y) = pair; *y + x + z }", &["pair", "z"]),
        ("x[r#y] + y", &["x", "y"]),
        ("x += y", &["x", "y"]),
        ("generic::<i32, { N }>(x)", &["N", "generic", "x"]),
        ("module::generic::<{ N }>(x)", &["N", "x"]),
        ("x.method::<{ N }>(y)", &["N", "x", "y"]),
        ("<T as Trait<{ N }>>::method(x)", &["N", "x"]),
        ("async move { x.await + y }", &["x", "y"]),
        ("'again: loop { if x { break 'again y; } }", &["x", "y"]),
    ];
    for (source, expected) in cases {
        let expression = syn::parse_str(source).unwrap();
        let free = free_variables(&expression);
        assert_eq!(free.names, names(expected), "{source}");
        assert!(!free.opaque, "{source}");
    }
}

#[test]
fn opaque_syntax_retains_the_whole_environment() {
    let environment = names(&["x", "y", "z"]);
    for source in [
        "format!(\"{x}\")",
        "{ introduce_binding!(); x }",
        "{ fn local() {} local(); x }",
        "#[cfg(any())] x",
        "{ let pattern!() = x; y }",
    ] {
        let expression = syn::parse_str(source).unwrap();
        let free = free_variables(&expression);
        assert!(free.opaque, "{source}");
        assert_eq!(free.dependencies(&environment), environment, "{source}");
    }
    let hidden = syn::Expr::Verbatim(quote::quote!(unexpanded syntax));
    assert!(free_variables(&hidden).opaque);
    let ordinary = free_variables(&syn::parse_quote!(accept(x)));
    assert_eq!(ordinary.names, names(&["accept", "x"]));
    assert_eq!(ordinary.dependencies(&environment), names(&["x"]));
}

#[test]
fn pattern_bindings_are_separate_from_expression_uses() {
    use syn::parse::Parser;
    for (source, expected) in [
        ("(ref x, mut y)", vec!["x", "y"]),
        ("whole @ Some(x)", vec!["whole", "x"]),
        (
            "Pair { field: renamed, shorthand, .. }",
            vec!["renamed", "shorthand"],
        ),
        ("Some(x) | Other(r#x)", vec!["x"]),
        ("[first, rest @ ..]", vec!["first", "rest"]),
    ] {
        let pattern = syn::Pat::parse_multi.parse_str(source).unwrap();
        let actual: Vec<_> = bindings(&pattern)
            .iter()
            .map(cq_syntax::symbol_name)
            .collect();
        assert_eq!(actual, expected, "{source}");
    }
}
