use cq_order::{Options, check, compile};
#[test]
fn schedules_a_root_section_without_a_reference_adapter() {
    let source = syn::parse_quote!(struct Q; relation R(a:i32); answer(x) :- R(x););
    let target = compile(&source, &(), &Options::default()).unwrap();
    assert_eq!(target, syn::parse_quote!(valuation { R@0(x); }));
    assert!(check(&source, &(), &target, &Options::default()));
}
