macro_rules! fixture {
    (physical { $($physical:tt)* } structures { $($structures:tt)* } $($sequence:tt)*) => {{
        let physical: cq_lang_sequence::structures::physical::Program =
            syn::parse2(quote::quote!(physical { $($physical)* })).unwrap();
        let structures = syn::parse::Parser::parse2(
            |input: syn::parse::ParseStream<'_>| cq_lang_sequence::structures::Program::parse(input, &physical),
            quote::quote!(structures { $($structures)* }),
        ).unwrap();
        let program: cq_lang_sequence::Program =
            syn::parse2(quote::quote!($($sequence)*)).unwrap();
        (physical, structures, program)
    }};
}

macro_rules! structure_fixture {
    (physical { $($physical:tt)* } structures { $($structures:tt)* }) => {{
        let physical: cq_lang_sequence::structures::physical::Program =
            syn::parse2(quote::quote!(physical { $($physical)* })).unwrap();
        let structures = syn::parse::Parser::parse2(
            |input: syn::parse::ParseStream<'_>| cq_lang_sequence::structures::Program::parse(input, &physical),
            quote::quote!(structures { $($structures)* }),
        ).unwrap();
        (physical, structures)
    }};
}
