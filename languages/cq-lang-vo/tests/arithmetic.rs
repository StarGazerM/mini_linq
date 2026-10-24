use cq_lang_vo::{Program, contract};

#[test]
fn arithmetic_moves_before_unrelated_atoms_and_opaque_steps() {
    let program: Program = syn::parse_quote! {
        struct Query;
        relation R(x: i64, y: i64);
        relation S(z: i64);
        answer(value) :- R(x, y), S(z), if opaque(z),
            let value: i64 = *x * (100 - *y), if *x + *y > 0;
        valuation {
            R@0(x, y);
            let value: i64 = *x * (100 - *y);
            if *x + *y > 0;
            S@1(z);
            if opaque(z);
        }
    };
    contract::check(&program).unwrap();
}

#[test]
fn runtime_division_retains_before_and_arithmetic_does_not_unpin_opaque_guards() {
    let partial: Program = syn::parse_quote! {
        struct Query; relation R(x: i64, y: i64); relation S(z: i64);
        answer(value) :- R(x,y), S(z), let value: i64 = *x / *y;
        valuation { R@0(x,y); let value: i64 = *x / *y; S@1(z); }
    };
    assert!(contract::check(&partial).is_err());
    let reordered: Program = syn::parse_quote! {
        struct Query; relation R(x: i64, y: i64);
        answer(x) :- R(x,y), if *x * *y > 0, if opaque(x);
        valuation { R@0(x,y); if opaque(x); if *x * *y > 0; }
    };
    assert!(contract::check(&reordered).is_err());
}
