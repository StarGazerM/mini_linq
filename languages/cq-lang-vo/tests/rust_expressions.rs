use cq_lang_vo::{Program, contract};

#[test]
fn rust_moves_before_unrelated_atoms_and_expressions() {
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
fn division_and_opaque_guards_have_only_free_variable_dependencies() {
    let division: Program = syn::parse_quote! {
        struct Query; relation R(x: i64, y: i64); relation S(z: i64);
        answer(value) :- R(x,y), S(z), let value: i64 = *x / *y;
        valuation { R@0(x,y); let value: i64 = *x / *y; S@1(z); }
    };
    contract::check(&division).unwrap();
    let reordered: Program = syn::parse_quote! {
        struct Query; relation R(x: i64, y: i64);
        answer(x) :- R(x,y), if *x * *y > 0, if opaque(x);
        valuation { R@0(x,y); if opaque(x); if *x * *y > 0; }
    };
    contract::check(&reordered).unwrap();
}
