use cq_gpu_datalog_core::domains::DomainComponents;
use cq_lang_datalog_storage::incremental::strata::source::{BodyItem, Program};

#[test]
fn filter_only_comparison_unifies_exactly_its_operands() {
    let mut source: Program = syn::parse_quote! {
        struct Domains;
        relation E(a:u32,b:u32,c:u32) = (edges);
        relation Out(a:u32);
        output Out;
        Out(x) :- E(x,y,z), where x < y;
    };
    source.rules[0].body[1] = BodyItem::Filter(syn::parse_quote!(if x < y));
    let table = DomainComponents::infer(&source).unwrap();
    assert_eq!(table.columns, [vec![0,0,1], vec![0]]);
    assert_eq!(table.count, 2);
}

#[test]
fn or_branches_unify_comparison_operands_without_merging_constants() {
    let source: Program = syn::parse_quote! {
        struct Domains;
        relation E(a:u32,b:u32,c:u32,d:u32) = (edges);
        relation Out(a:u32);
        output Out;
        Out(x) :- E(x,y,z,w), where x == y || y < z || w == 7;
    };
    let table = DomainComponents::infer(&source).unwrap();
    assert_eq!(table.columns, [vec![0,0,0,1], vec![0]]);
    assert_eq!(table.count, 2);
}
