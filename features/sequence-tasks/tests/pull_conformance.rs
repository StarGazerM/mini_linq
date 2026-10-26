use cq_lang_sequence::Program;
use proc_macro2::TokenStream;
use quote::quote;

fn simple(body: TokenStream) -> Program {
    let physical = syn::parse_quote! {
        physical {
            relation R(a:i32);
            r = pipe canon rename R@0 {a -> a};
            return r as answer(a);
        }
    };
    Program {
        structures: cq_structure_selection::compile(&physical).unwrap(),
        sequence: syn::parse2(quote!(sequence { #body })).unwrap(),
        graded: None,
        evidence: Default::default(),
    }
}

#[test]
fn sequential_and_tasks_share_the_sequence_answer_contract() {
    use cq_sequence_conformance::{Target, assert_conformant};
    let program = simple(quote! {
        input = store rows r;
        out = store tile 2 canon input;
    });
    assert_conformant(&program, &[Target::Sequential, Target::Tasks]);
}
