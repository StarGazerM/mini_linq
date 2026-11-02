//! A-Domain / WF-Domain: closed operands at code-domain columns are translated
//! once on the host; the decision reads only the authored `in (D)` fact.
use cq_lang_cq as cq;
use quote::{ToTokens, quote};

fn cq(tokens: proc_macro2::TokenStream) -> cq::Program {
    syn::parse2(tokens).unwrap()
}

fn captures(program: &cq::Program) -> Vec<String> {
    program
        .query
        .captures
        .items
        .iter()
        .map(|item| item.to_token_stream().to_string())
        .collect()
}

#[test]
fn closed_operands_at_domain_columns_become_translation_captures() {
    let source = cq(quote! {
        struct Q;
        relation R(a: i32, m: Code in (modes));
        relation answer(a: i32);
        answer(a) :- R(a, "AIR"), R(a, m), where m == "MAIL" || m == "SHIP", where m != host();
    });
    let normal = cq::normal::normalize(&source).unwrap();
    assert_eq!(
        captures(&normal),
        [
            "let __cq_c_0 : Code = :: cq_storage :: domain :: exact (& (modes) , & (\"AIR\"))",
            "let __cq_c_1 : Code = :: cq_storage :: domain :: exact (& (modes) , & (\"MAIL\"))",
            "let __cq_c_2 : Code = :: cq_storage :: domain :: exact (& (modes) , & (\"SHIP\"))",
            "let __cq_c_3 : Code = :: cq_storage :: domain :: exact (& (modes) , & (host ()))",
        ]
    );
    let body = normal.query.body.to_token_stream().to_string();
    assert!(body.contains("R (a , __cq_0_1) , where __cq_0_1 == (__cq_c_0)"), "{body}");
    assert!(body.contains("where m != (__cq_c_3)"), "{body}");
    cq::normal::check(&normal).unwrap();
    // Normalization is idempotent: translated operands stay translated.
    assert_eq!(cq::normal::normalize(&normal).unwrap(), normal);
    // The untranslated source is not a normal form.
    assert!(cq::normal::check(&source).is_err());
}

#[test]
fn ordered_comparisons_and_ranges_use_order_preserving_bounds() {
    let source = cq(quote! {
        struct Q;
        relation R(m: Code in (names));
        relation answer(m: Code);
        answer(m) :- R(m), where m <= "m", where m > "c", where m in "a"..="z";
    });
    let normal = cq::normal::normalize(&source).unwrap();
    let functions: Vec<_> = captures(&normal)
        .iter()
        .map(|capture| capture.split(" :: ").nth(3).unwrap().split(' ').next().unwrap().to_owned())
        .collect();
    assert_eq!(functions, ["upper", "upper", "lower", "upper"]);
    let body = normal.query.body.to_token_stream().to_string();
    assert!(body.contains("m < (__cq_c_0)"), "{body}");
    assert!(body.contains("m >= (__cq_c_1)"), "{body}");
    assert!(body.contains("m >= (__cq_c_2)") && body.contains("m < (__cq_c_3)"), "{body}");
    cq::normal::check(&normal).unwrap();
}

#[test]
fn columns_without_domains_keep_their_operands() {
    let source = cq(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation answer(a: i32);
        answer(a) :- R(a, 7), where a == 3;
    });
    let normal = cq::normal::normalize(&source).unwrap();
    assert!(normal.query.captures.items.is_empty());
}

#[test]
fn joins_and_comparisons_across_domains_are_rejected() {
    for body in [
        quote!(answer(x) :- R(x), S(x);),
        quote!(answer(x) :- R(x), T(x);),
        quote!(answer(x) :- R(x), T(y), where x == y;),
    ] {
        let source = cq(quote! {
            struct Q;
            relation R(a: Code in (left));
            relation S(a: Code in (right));
            relation T(a: Code);
            relation answer(a: Code);
            #body
        });
        let error = cq::normal::normalize(&source).unwrap_err().to_string();
        assert!(error.contains("WF-Domain"), "{error}");
    }
}
