use cq_lang_vo::contract;

#[test]
fn rust_moves_before_unrelated_atoms_and_expressions() {
    let (cq, program) = parse(quote::quote! {
        struct Query;
        relation R(x: i64, y: i64);
        relation S(z: i64);
        answer(value) :- R(x, y), S(z), if opaque(z),
            let value: i64 = *x * (100 - *y), if *x + *y > 0;
        valuation {
            R@0(x, y);
            if *x + *y > 0;
            let value: i64 = *x * (100 - *y);
            S@1(z);
            if opaque(z);
        }
    }).unwrap();
    contract::check(&cq, &program).unwrap();
}

#[test]
fn division_and_opaque_guards_have_only_free_variable_dependencies() {
    let (cq, division) = parse(quote::quote! {
        struct Query; relation R(x: i64, y: i64); relation S(z: i64);
        answer(value) :- R(x,y), S(z), let value: i64 = *x / *y;
        valuation { R@0(x,y); let value: i64 = *x / *y; S@1(z); }
    }).unwrap();
    contract::check(&cq, &division).unwrap();
    let (cq, reordered) = parse(quote::quote! {
        struct Query; relation R(x: i64, y: i64);
        answer(x) :- R(x,y), if *x * *y > 0, if opaque(x);
        valuation { R@0(x,y); if opaque(x); if *x * *y > 0; }
    }).unwrap();
    contract::check(&cq, &reordered).unwrap();
}

fn parse(tokens: proc_macro2::TokenStream) -> syn::Result<(cq_lang_vo::cq::Program, cq_lang_vo::Program)> {
    syn::parse::Parser::parse2(|input: syn::parse::ParseStream<'_>| {
        let cq = input.parse()?;
        let vo = cq_lang_vo::Program::parse(input, &cq)?;
        Ok((cq, vo))
    }, tokens)
}
