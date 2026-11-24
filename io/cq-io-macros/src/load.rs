use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::ext::IdentExt;

use crate::schema::{Declarations, check_declarations};

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
    check_declarations(&declarations, false)?;
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
                    type State = (#(<#parameters as ::cq_load::InternedField>::State,)*);
                    const KEY_COLUMNS: &'static [(&'static str, Option<::cq_load::set_manifest::KeyEncoding>)] =
                        &[#((#names, <#parameters as ::cq_load::InternedField>::KEY_ENCODING),)*];

                    fn state() -> Self::State {
                        (#(<#parameters as ::cq_load::InternedField>::State::default(),)*)
                    }

                    fn intern(
                        rows: ::std::vec::Vec<<Self::Source as ::cq_load::Relation>::Row>,
                        dictionaries: &mut ::cq_load::Dictionaries,
                    ) -> ::std::vec::Vec<Self::Row> {
                        let mut state = Self::state();
                        let mut rows: ::std::vec::Vec<_> = rows.into_iter().map(|row| (
                            #(
                                <#parameters as ::cq_load::InternedField>::encode(
                                    row.#tuple_indices, &mut state.#tuple_indices,
                                ),
                            )*
                        )).collect();
                        Self::finish(&mut rows, state, dictionaries);
                        rows
                    }

                    fn decode(
                        batch: &::cq_load::Batch<'_>,
                        state: &mut Self::State,
                    ) -> ::cq_load::Result<::std::vec::Vec<Self::Row>> {
                        #(
                            let #fields = <#parameters as ::cq_load::InternedField>::column(
                                batch, #indices, &mut state.#tuple_indices,
                            )?.into_iter();
                        )*
                        Ok(#decode)
                    }

                    fn finish(
                        rows: &mut [Self::Row],
                        mut state: Self::State,
                        dictionaries: &mut ::cq_load::Dictionaries,
                    ) {
                        #(
                            <#parameters as ::cq_load::InternedField>::finish(
                                #names, &mut state.#tuple_indices, dictionaries,
                            );
                        )*
                        for row in rows {
                            #(
                                <#parameters as ::cq_load::InternedField>::remap(
                                    &mut row.#tuple_indices, &state.#tuple_indices,
                                );
                            )*
                        }
                    }
                }
            }
        });
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
                const KEY_COLUMNS: &'static [(&'static str, Option<::cq_load::set_manifest::KeyEncoding>)] =
                    &[#((#names, <#parameters as ::cq_load::Load>::KEY_ENCODING),)*];
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
            "relation R(a: i32 @ plain);",
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

    #[test]
    fn verified_key_metadata_uses_field_traits_and_authored_column_order() {
        let source = "relation R(r#type: UserAlias, value: crate::Custom);";
        let output = generate(syn::parse_str(source).unwrap(), true).unwrap();
        let file: syn::File = syn::parse2(output).unwrap();
        for (relation_trait, field_trait) in [
            ("Relation", quote::quote!(::cq_load::Load)),
            ("InternedRelation", quote::quote!(::cq_load::InternedField)),
        ] {
            let implementation = file.items.iter().find_map(|item| match item {
                syn::Item::Impl(implementation)
                    if implementation.trait_.as_ref().is_some_and(|(_, path, _)|
                        path.segments.last().is_some_and(|segment|
                            segment.ident == relation_trait)) => Some(implementation),
                _ => None,
            }).unwrap();
            let metadata = implementation.items.iter().find_map(|item| match item {
                syn::ImplItem::Const(item) if item.ident == "KEY_COLUMNS" => Some(&item.expr),
                _ => None,
            }).unwrap();
            assert_eq!(
                quote::quote!(#metadata).to_string(),
                quote::quote!(&[
                    ("type", <Column0 as #field_trait>::KEY_ENCODING),
                    ("value", <Column1 as #field_trait>::KEY_ENCODING),
                ]).to_string(),
            );
        }
    }
}
