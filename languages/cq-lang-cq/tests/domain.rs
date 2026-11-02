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
            "domain __cq_c_0 : Code in [0] (modes) = exact (\"AIR\")",
            "domain __cq_c_1 : Code in [0] (modes) = exact (\"MAIL\")",
            "domain __cq_c_2 : Code in [0] (modes) = exact (\"SHIP\")",
            "domain __cq_c_3 : Code in [0] (modes) = exact (host ())",
        ]
    );
    let body = normal.query.body.to_token_stream().to_string();
    assert!(
        body.contains("R (a , __cq_0_1) , where __cq_0_1 == (__cq_c_0)"),
        "{body}"
    );
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
    let functions: Vec<_> = normal
        .query
        .captures
        .items
        .iter()
        .map(|capture| match capture {
            cq::syntax::Capture::Domain(capture) => capture.function,
            _ => panic!("expected a recorded domain translation"),
        })
        .collect();
    use cq::syntax::capture::DomainFunction::{Lower, Upper};
    assert_eq!(functions, [Upper, Upper, Lower, Upper]);
    let body = normal.query.body.to_token_stream().to_string();
    assert!(body.contains("m < (__cq_c_0)"), "{body}");
    assert!(body.contains("m >= (__cq_c_1)"), "{body}");
    assert!(
        body.contains("m >= (__cq_c_2)") && body.contains("m < (__cq_c_3)"),
        "{body}"
    );
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

#[test]
fn authored_domain_calls_remain_opaque_and_normalization_is_idempotent() {
    let source = cq(quote! {
        struct Q;
        relation R(m: Code in (names));
        relation answer(m: Code);
        answer(m) :- R(m), where m == (c)
        capture {
            let c = ::cq_storage::domain::exact(&(names), &("AIR"));
        };
    });
    let error = cq::normal::check(&source).unwrap_err().to_string();
    assert!(error.contains("WF-Domain"), "{error}");
    let normal = cq::normal::normalize(&source).unwrap();
    let [
        cq::syntax::Capture::Let(binding),
        cq::syntax::Capture::Domain(capture),
    ] = normal.query.captures.items.as_slice()
    else {
        panic!("an authored call must be distinct from an A-Domain record");
    };
    assert_eq!(
        binding.expression,
        syn::parse_quote!(::cq_storage::domain::exact(&(names), &("AIR")))
    );
    assert_eq!(capture.operand, syn::parse_quote!(c));
    cq::normal::check(&normal).unwrap();
    assert_eq!(cq::normal::normalize(&normal).unwrap(), normal);
    let reparsed: cq::Program = syn::parse2(normal.to_token_stream()).unwrap();
    assert_eq!(reparsed, normal);
    assert_eq!(cq::normal::normalize(&reparsed).unwrap(), normal);
}

#[test]
fn domain_capture_checks_recorded_domain_and_function() {
    let source = cq(quote! {
        struct Q;
        relation R(m: Code in (names));
        relation answer(m: Code);
        answer(m) :- R(m), where m == "AIR";
    });
    let normal = cq::normal::normalize(&source).unwrap();
    for wrong_domain in [false, true] {
        let mut invalid = normal.clone();
        let cq::syntax::Capture::Domain(capture) = &mut invalid.query.captures.items[0] else {
            panic!("expected a recorded translation");
        };
        if wrong_domain {
            capture.domain.id = Some(cq::syntax::DomainId(1));
        } else {
            capture.function = cq::syntax::capture::DomainFunction::Lower;
        }
        let error = cq::normal::check(&invalid).unwrap_err().to_string();
        assert!(error.contains("WF-Domain"), "{error}");
    }
}

#[test]
fn declaration_table_interns_domains_across_inputs_and_output() {
    let source = cq(quote! {
        struct Q;
        relation R(m: Code in (dictionary.get::<Text>()));
        relation S(m: Code in (dictionary.get::<Text>()));
        relation T(m: Code in (other));
        relation answer(m: Code in (dictionary.get::<Text>()));
        answer(m) :- R(m), S(m), where m == "AIR";
    });
    let ids: Vec<_> = source
        .declarations
        .iter()
        .map(|declaration| declaration.columns[0].domain.as_ref().unwrap().id.unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            cq::syntax::DomainId(0),
            cq::syntax::DomainId(0),
            cq::syntax::DomainId(1),
            cq::syntax::DomainId(0),
        ]
    );
    let normal = cq::normal::normalize(&source).unwrap();
    let cq::syntax::Capture::Domain(capture) = &normal.query.captures.items[0] else {
        panic!("expected a recorded domain translation");
    };
    assert_eq!(capture.domain.id, Some(ids[0]));
    let reparsed: cq::Program = syn::parse2(normal.to_token_stream()).unwrap();
    assert_eq!(reparsed, normal);
    assert_eq!(cq::normal::normalize(&reparsed).unwrap(), normal);
}

#[test]
fn domain_checks_use_recorded_ids_not_rust_payloads() {
    let source = cq(quote! {
        struct Q;
        relation R(m: Code in [7] (left()));
        relation S(m: Code in [7] (right()));
        relation answer(m: Code in [7] (output()));
        answer(m) :- R(m), S(m), !S("MAIL"), where m == "AIR";
    });
    let mut normal = cq::normal::normalize(&source).unwrap();
    for capture in &mut normal.query.captures.items {
        let cq::syntax::Capture::Domain(capture) = capture else {
            panic!("expected a recorded domain translation");
        };
        assert_eq!(capture.domain.id, Some(cq::syntax::DomainId(7)));
        capture.domain.expression = syn::parse_quote!(an_opaque_runtime_payload());
    }
    cq::normal::check(&normal).unwrap();
    assert_eq!(cq::normal::normalize(&normal).unwrap(), normal);
    let reparsed: cq::Program = syn::parse2(normal.to_token_stream()).unwrap();
    assert_eq!(reparsed, normal);
    cq::normal::check(&reparsed).unwrap();
}

#[test]
fn matching_rust_payloads_do_not_override_distinct_domain_ids() {
    for body in [
        quote!(answer(x) :- R(x), S(x);),
        quote!(answer(x) :- R(x), S(y), where x == y;),
        quote!(answer(x) :- R(x), !S(x);),
    ] {
        let source = cq(quote! {
            struct Q;
            relation R(m: Code in [0] (same));
            relation S(m: Code in [1] (same));
            relation answer(m: Code in [0] (same));
            #body
        });
        let error = cq::normal::normalize(&source).unwrap_err().to_string();
        assert!(error.contains("WF-Domain"), "{error}");
    }
    let source = cq(quote! {
        struct Q;
        relation R(m: Code in [0] (same));
        relation answer(m: Code in [1] (same));
        answer(m) :- R(m);
    });
    let error = cq::normal::normalize(&source).unwrap_err().to_string();
    assert!(error.contains("WF-Domain"), "{error}");
}

#[test]
fn normalization_interns_programmatically_supplied_declarations() {
    let source = cq(quote! {
        struct Q;
        relation R(m: Code in (names));
        relation answer(m: Code in (names));
        answer(m) :- R(m), where m == "AIR";
    });
    let expected = cq::normal::normalize(&source).unwrap();
    let mut unrecorded = source;
    for declaration in &mut unrecorded.declarations {
        declaration.columns[0].domain.as_mut().unwrap().id = None;
    }
    assert_eq!(cq::normal::normalize(&unrecorded).unwrap(), expected);
}

#[test]
fn wellformedness_requires_recorded_declaration_and_capture_ids() {
    let source = cq(quote! {
        struct Q;
        relation R(m: Code in (names));
        relation answer(m: Code);
        answer(m) :- R(m), where m == "AIR";
    });
    let normal = cq::normal::normalize(&source).unwrap();
    for declaration in [false, true] {
        let mut invalid = normal.clone();
        if declaration {
            invalid.declarations[0].columns[0]
                .domain
                .as_mut()
                .unwrap()
                .id = None;
        } else {
            let cq::syntax::Capture::Domain(capture) = &mut invalid.query.captures.items[0] else {
                panic!("expected a recorded translation");
            };
            capture.domain.id = None;
        }
        let error = cq::normal::check(&invalid).unwrap_err().to_string();
        assert!(error.contains("WF-Domain"), "{error}");
    }
}
