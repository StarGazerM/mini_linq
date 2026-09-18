//! Plain item and callable hosts introduce no counter or observer convention.
use super::super::{check_program, check_query, layout, statements};
use super::*;

pub fn compile_program(source: &ra::Module, options: &RelationOptions) -> syn::Result<syn::File> {
    ra::contract::check(source)?;
    options.validate(source)?;
    let layout = layout::analyze(source, options)?;
    let wiring = Wiring::new(source, &layout, options, None)?;
    let statements = statements(source, options, &layout, &wiring)?;
    let statements = statements.iter().map(|(statement, _)| statement);
    let parameters = parameters(source, &layout, options);
    let answer = answer(source, &layout, &wiring);
    let row = row(&layout);
    let (visibility, program) = (&source.program.visibility, &source.program.name);
    let target = syn::parse2(quote! {
        #visibility struct #program;
        impl #program {
            pub fn run(#(#parameters),*) -> ::std::vec::Vec<#row> {
                #(#statements)*
                #answer
                __ml_answer.into_iter().collect()
            }
        }
    })?;
    check_program(source, &target, options)?;
    Ok(target)
}

/// Return an anonymous closure over borrowed inputs; bound provider expressions
/// capture caller configuration under ordinary Rust closure rules.
pub fn compile_query(source: &ra::Module, options: &RelationOptions) -> syn::Result<syn::Expr> {
    ra::contract::check(source)?;
    options.validate(source)?;
    let layout = layout::analyze(source, options)?;
    let wiring = Wiring::new(source, &layout, options, None)?;
    let statements = statements(source, options, &layout, &wiring)?;
    let statements = statements.iter().map(|(statement, _)| statement);
    let parameters = parameters(source, &layout, options);
    let answer = answer(source, &layout, &wiring);
    let row = row(&layout);
    let target = syn::parse2(quote! {
        move |#(#parameters),*| -> ::std::vec::Vec<#row> {
            #(#statements)*
            #answer
            __ml_answer.into_iter().collect()
        }
    })?;
    check_query(source, &target, options)?;
    Ok(target)
}
