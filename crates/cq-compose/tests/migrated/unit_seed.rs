use cq_ir::{index_requirements, relational_plan};
use quote::quote;
#[test]
fn unit_seed_uses_result_indexes_and_fused_linear_lowering() {
    let source: relational_plan::Module = syn::parse2(quote! {
        struct GlobalSeed;
        relation R(c0: i32);
        answer(x) :- R(x);
        relational {
            r0 = unit;
            r1 = rename R {c0 -> x};
            r2 = natural_join r0 with r1;
            r3 = project r2 keep {x};
            output r3 as answer(x);
        }
    })
    .unwrap();
    relational_plan::contract::check(&source).unwrap();

    assert!(cq_index_policy::base_demands::check_domain(&source).is_err());
    assert!(cq_index_policy::base_demands::compile(&source).is_err());
    let requirements = cq_index_policy::result_demands::compile(&source).unwrap();
    assert!(cq_index_policy::result_demands::check(
        &source,
        &requirements
    ));

    let indexed: index_requirements::Module = syn::parse2(quote! {
        #source
        indexes {}
    })
    .unwrap();
    index_requirements::contract::check(&indexed).unwrap();
    cq_relation_lowering::prefix_hash::compile(
        &indexed,
        &syn::parse_quote!(::mini_linq::access_to_distinct_iter),
    )
    .unwrap();
}
