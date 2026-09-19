use cq_ir::{cq, iterator_pipeline, relational_plan, rust_access_plan};
use quote::ToTokens;

#[test]
fn semicolons_are_required_at_every_language_boundary() {
    fn check<T: syn::parse::Parse + ToTokens>(source: &str) {
        let parsed: T = syn::parse_str(source).unwrap();
        syn::parse2::<T>(parsed.to_token_stream()).unwrap();
        let legacy = format!("{}.", source.strip_suffix(';').unwrap());
        assert!(syn::parse_str::<T>(&legacy).is_err(), "{legacy}");
    }
    check::<cq::Module>("struct P; relation R(a:i32); answer(x) :- R(x);");
    check::<relational_plan::Plan>("r0 = unit; output r0 as answer();");
    check::<rust_access_plan::Plan>("answer(x) => ((x,)) :- R(x) => bind (x,) <- (rows.iter());");
    check::<iterator_pipeline::Pipeline>("iter0 = unit yield (); return iter0;");
}

#[test]
fn declarations_require_items_and_preserve_written_order() {
    let source: cq::Module =
        syn::parse_str("struct P; relation Z(a:i32); relation A(a:i32); answer(x) :- Z(x), A(x);")
            .unwrap();
    assert_eq!(
        source
            .program
            .inputs()
            .map(|d| d.name.to_string())
            .collect::<Vec<_>>(),
        ["Z", "A"]
    );
    assert_eq!(
        syn::parse2::<cq::Module>(source.to_token_stream()).unwrap(),
        source
    );
    assert!(syn::parse_str::<cq::Module>("struct P; answer(x) :- R(x);").is_err());
}

#[test]
fn nested_plans_reject_missing_output_and_tokens_after_output() {
    let prefix = "struct P; relation R(a:i32); answer(x) :- R(x);";
    for plan in [
        "r0 = rename R {a -> x};",
        "r0 = rename R {a -> x}; output r0 as answer(x); junk",
        "r0 = rename R {a -> x}; output r0 as answer(x).",
        "r0 = rename R {a -> x extra}; output r0 as answer(x);",
    ] {
        let source = format!("{prefix} relational {{ {plan} }}");
        assert!(
            syn::parse_str::<relational_plan::Module>(&source).is_err(),
            "{source}"
        );
    }
    let source = format!(
        "{prefix} relational {{ r0 = rename R {{a -> x}}; output r0 as answer(x); }} indexes {{}} uses {{}}"
    );
    syn::parse_str::<cq_ir::index_requirements::Module>(&source).unwrap();
}
