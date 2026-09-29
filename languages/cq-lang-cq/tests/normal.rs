use cq::normal::Kernel;
use cq_lang_cq as cq;
use quote::{ToTokens, quote};

fn cq(tokens: proc_macro2::TokenStream) -> cq::Program {
    syn::parse2(tokens).unwrap()
}

#[test]
fn normalization_moves_every_non_variable_argument_out_of_atoms() {
    let source = cq(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(x, z) :- R(x, 5), S(_, z), R(x, x), if z > 1;
    });
    let normal = cq::normal::normalize(&source).unwrap();
    let text = normal.query.to_token_stream().to_string();
    // The constant becomes a join key bound before every atom.
    assert!(
        text.starts_with("answer (x , z) : - let __cq_0_1 : i32 = 5 , R (x , __cq_0_1)"),
        "{text}"
    );
    // The wildcard is a distinct witness; the repeat gets an equality guard.
    assert!(
        text.contains("S (__cq_1_0 , z) , R (x , __cq_2_1) , if __cq_2_1 == x , if z > 1"),
        "{text}"
    );
    cq::normal::check(&normal).unwrap();
    assert!(cq::normal::check(&source).is_err());
    // Normalizing again is the identity.
    assert_eq!(cq::normal::normalize(&normal).unwrap(), normal);
}

#[test]
fn kernels_compare_up_to_renaming_of_variables() {
    let one = cq(quote! {
        struct Q; relation R(a: i32, b: i32);
        answer(x) :- R(x, y), if y > 1;
    });
    let two = cq(quote! {
        struct Q; relation R(a: i32, b: i32);
        answer(p) :- R(p, q), if q > 1;
    });
    let three = cq(quote! {
        struct Q; relation R(a: i32, b: i32);
        answer(p) :- R(q, p), if q > 1;
    });
    assert_eq!(Kernel::of(&one), Kernel::of(&two));
    assert_ne!(Kernel::of(&one), Kernel::of(&three));
}
