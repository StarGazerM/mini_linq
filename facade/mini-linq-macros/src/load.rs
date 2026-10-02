use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{ext::IdentExt, parse::Parse};

struct Declarations(Vec<cq_lang_cq::RelationDecl>);

impl Parse for Declarations {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(cq_lang_cq::kw::relation) {
            let mut declarations = Vec::new();
            while !input.is_empty() {
                declarations.push(input.parse()?);
            }
            Ok(Self(declarations))
        } else {
            let program: cq_lang_cq::Program = input.parse()?;
            Ok(Self(program.inputs().cloned().collect()))
        }
    }
}

pub(crate) fn expand(input: TokenStream) -> TokenStream {
    syn::parse(input)
        .and_then(generate)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn generate(Declarations(declarations): Declarations) -> syn::Result<proc_macro2::TokenStream> {
    cq_lang_cq::check_input_declarations(&declarations)?;
    let mut output = proc_macro2::TokenStream::new();
    let mut selectors = Vec::new();
    for declaration in declarations {
        let relation = &declaration.name;
        let name = declaration.name.unraw().to_string();
        let marker = format_ident!("__MiniLinqLoad{}", name);
        let loader = format_ident!("{}Loader", name);
        selectors.push(quote!((#relation) => { #loader };));
        let types: Vec<_> = declaration
            .columns
            .iter()
            .map(|column| &column.ty)
            .collect();
        let names: Vec<_> = declaration
            .columns
            .iter()
            .map(|column| column.name.unraw().to_string())
            .collect();
        let arity = types.len();
        let indices: Vec<_> = (0..arity).collect();
        let fields: Vec<_> = indices
            .iter()
            .map(|i| format_ident!("column_{i}"))
            .collect();
        let parameters: Vec<_> = indices.iter().map(|i| format_ident!("Column{i}")).collect();
        let generic_parameters = (!parameters.is_empty()).then(|| quote!(<#(#parameters),*>));
        let generic_types = (!types.is_empty()).then(|| quote!(<#(#types),*>));
        let generic_bounds =
            (!parameters.is_empty()).then(|| quote!(<#(#parameters: ::cq_load::Load),*>));
        let decode = if let Some(first) = fields.first() {
            let mut zipped = quote!(#first);
            let mut pattern = quote!(#first);
            for field in &fields[1..] {
                zipped = quote!(#zipped.zip(#field));
                pattern = quote!((#pattern, #field));
            }
            quote!(#zipped.map(|#pattern| (#(#fields,)*)).collect())
        } else {
            quote!(::std::vec![(); batch.len()])
        };
        let groups = (0..=arity).map(|prefix| {
            let keys = &parameters[..prefix];
            let values = &parameters[prefix..];
            let key_fields = &fields[..prefix];
            let value_fields = &fields[prefix..];
            quote! {
                impl #generic_bounds ::cq_load::Group<#prefix> for #marker #generic_parameters {
                    type Key = (#(#keys,)*);
                    type Value = (#(#values,)*);
                    fn split(row: Self::Row) -> (Self::Key, Self::Value) {
                        let (#(#fields,)*) = row;
                        ((#(#key_fields,)*), (#(#value_fields,)*))
                    }
                }
            }
        });
        output.extend(quote! {
                #[allow(non_camel_case_types)]
            pub struct #marker #generic_parameters (::core::marker::PhantomData<fn() -> (#(#parameters,)*)>);
            impl #generic_bounds ::cq_load::Relation for #marker #generic_parameters {
                type Row = (#(#parameters,)*);
                const COLUMNS: &'static [&'static str] = &[#(#names,)*];
                fn decode(batch: &::cq_load::Batch<'_>) -> ::cq_load::Result<::std::vec::Vec<Self::Row>> {
                    #(let #fields = batch.column::<#parameters>(#indices)?.into_iter();)*
                    Ok(#decode)
                }
            }
            #(#groups)*
            #[doc = "Typed CSV and Parquet readers generated from the authored relation."]
            #[allow(non_camel_case_types, private_interfaces)]
            pub type #loader = ::cq_load::Loader<#marker #generic_types>;
        });
    }
    output.extend(quote! {
        #[allow(unused_macros)]
        macro_rules! relation_loader { #(#selectors)* }
    });
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::{Declarations, generate};

    #[test]
    fn malformed_loader_schemas_return_diagnostics() {
        assert!(syn::parse_str::<Declarations>("").is_err());
        for source in [
            "relation R();",
            "relation R(a: i32, a: i64);",
            "relation R(a: i32); relation R(a: i32);",
        ] {
            let declarations = syn::parse_str::<Declarations>(source).unwrap();
            assert!(generate(declarations).is_err(), "{source}");
        }
    }
}
