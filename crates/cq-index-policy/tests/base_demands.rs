//! Input-column requirements derived from authored RA without another pass.
use cq_ir::{index_requirements as ix, relational_plan as ra};

#[test]
fn declaration_coordinates_survive_reordered_renames_and_repeated_inputs() {
    let source: ra::Module = syn::parse_quote! {
        struct Reordered;
        relation R(a:i32,b:i32,c:i32);
        relation S(a:i32,b:i32,c:i32);
        answer(x,w) :- R(x,y,z), S(z,x,w), S(z,x,q);
        relational {
            seed = rename R {c -> z, a -> x, b -> y};
            first = rename S {c -> w, b -> x, a -> z};
            second = rename S {b -> x, c -> q, a -> z};
            joined = natural_join seed with first;
            repeated = natural_join joined with second;
            result = project repeated keep {x,w};
            output result as answer(x,w);
        }
    };
    let expected: ix::Module = syn::parse_quote! {
        #source indexes { S[0,1]; }
    };
    assert_eq!(
        cq_index_policy::base_demands::compile(&source).unwrap(),
        expected
    );
    assert!(cq_index_policy::base_demands::check(&source, &expected));
}

#[test]
fn a_same_named_result_does_not_supply_a_base_input_requirement() {
    let source: ra::Module = syn::parse_quote! {
        struct Namespaces;
        relation R(a:i32,b:i32);
        relation S(a:i32,b:i32);
        answer(x,z) :- R(x,y), S(y,z);
        relational {
            seed = rename R {a -> x, b -> y};
            S = rename S {a -> y, b -> z};
            joined = natural_join seed with S;
            result = project joined keep {x,z};
            output result as answer(x,z);
        }
    };
    let correct: ix::Module = syn::parse_quote! { #source indexes { S[0]; } };
    let wrong: ix::Module = syn::parse_quote! { #source indexes { @S[0]; } };
    ix::contract::check(&wrong).unwrap();
    assert_eq!(
        cq_index_policy::base_demands::compile(&source).unwrap(),
        correct
    );
    assert!(cq_index_policy::base_demands::check(&source, &correct));
    assert!(!cq_index_policy::base_demands::check(&source, &wrong));
    assert_ne!(
        ix::signature(&correct.indexes[0]),
        ix::signature(&wrong.indexes[0])
    );
}
