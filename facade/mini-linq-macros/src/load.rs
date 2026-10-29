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

pub(crate) fn expand(input: TokenStream, interned: bool) -> TokenStream {
    syn::parse(input)
        .and_then(|declarations| generate(declarations, interned))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn generate(
    Declarations(declarations): Declarations,
    interned: bool,
) -> syn::Result<proc_macro2::TokenStream> {
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
        let interned_impl = interned.then(|| {
            let interned_bounds = (!parameters.is_empty())
                .then(|| quote!(<#(#parameters: ::cq_load::InternedField),*>));
            let source_types = (!parameters.is_empty())
                .then(|| quote!(<#(<#parameters as ::cq_load::InternedField>::Source),*>));
            let tuple_indices: Vec<_> = indices.iter().map(|&i| syn::Index::from(i)).collect();
            quote! {
                impl #interned_bounds ::cq_load::InternedRelation for #marker #generic_parameters {
                    type Row = (#(#parameters,)*);
                    type Source = #marker #source_types;

                    fn intern(
                        rows: ::std::vec::Vec<<Self::Source as ::cq_load::Relation>::Row>,
                        dictionaries: &mut ::cq_load::Dictionaries,
                    ) -> ::std::vec::Vec<Self::Row> {
                        #(
                            <#parameters as ::cq_load::InternedField>::prepare(
                                #names,
                                rows.iter().map(|row| &row.#tuple_indices),
                                dictionaries,
                            );
                        )*
                        #(let #fields = dictionaries.get(#names);)*
                        rows.into_iter().map(|row| (
                            #(
                                <#parameters as ::cq_load::InternedField>::intern(
                                    row.#tuple_indices, #fields,
                                ),
                            )*
                        )).collect()
                    }
                }
            }
        });
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
            #interned_impl
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
            for interned in [false, true] {
                let declarations = syn::parse_str::<Declarations>(source).unwrap();
                assert!(generate(declarations, interned).is_err(), "{source}");
            }
        }
    }

    #[test]
    fn interned_schema_implementation_requires_explicit_opt_in() {
        let source = "relation R(id: i64, text: String);";
        let plain = generate(syn::parse_str::<Declarations>(source).unwrap(), false).unwrap();
        let interned = generate(syn::parse_str::<Declarations>(source).unwrap(), true).unwrap();
        let plain: syn::File = syn::parse2(plain).unwrap();
        let mut interned: syn::File = syn::parse2(interned).unwrap();
        let before = interned.items.len();
        interned.items.retain(|item| {
            !matches!(
                item,
                syn::Item::Impl(implementation)
                    if implementation.trait_.as_ref().is_some_and(|(_, path, _)|
                        path.segments.last().is_some_and(|segment|
                            segment.ident == "InternedRelation"))
            )
        });
        assert_eq!(before, interned.items.len() + 1);
        assert_eq!(
            quote::quote!(#plain).to_string(),
            quote::quote!(#interned).to_string()
        );
    }
}
