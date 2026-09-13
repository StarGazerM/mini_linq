//! Native hash and ordered row-set expansions.
use crate::physical::options::ProviderOptions;
use cq_ir::physical_provider::Operation;
use proc_macro2::TokenStream;
use quote::quote;

#[derive(Clone, Copy)]
enum NativeSet {
    Hash,
    Sorted,
}

impl NativeSet {
    fn path(self) -> TokenStream {
        match self {
            Self::Hash => quote!(::std::collections::HashSet),
            Self::Sorted => quote!(::std::collections::BTreeSet),
        }
    }
}

/// Expand a typed operation over an ordinary native hash set of integer tuples.
///
/// The result is a Rust type for `Type`, statements for `Materialize`, and an
/// expression otherwise. The owning physical context supplies argument meanings.
pub fn compile_hash_provider(
    input: &Operation,
    options: &ProviderOptions,
) -> syn::Result<TokenStream> {
    native_provider(input, options, NativeSet::Hash)
}

/// Expand a typed operation over an ordinary native ordered set of integer tuples.
/// The result follows the same Rust-fragment contract as the hash provider.
pub fn compile_sorted_provider(
    input: &Operation,
    options: &ProviderOptions,
) -> syn::Result<TokenStream> {
    native_provider(input, options, NativeSet::Sorted)
}

fn native_provider(
    operation: &Operation,
    options: &ProviderOptions,
    family: NativeSet,
) -> syn::Result<TokenStream> {
    let set = family.path();
    let output = match operation {
        Operation::Type { arity, .. } => {
            quote!(#set<[::core::primitive::i64; #arity]>)
        }
        Operation::Input { value, .. } | Operation::Row { value, .. } => {
            quote!((#value))
        }
        Operation::Len { value, .. } => {
            quote!((#value).len())
        }
        Operation::Scan { value, .. } | Operation::Rows { value, .. } => {
            if let Some(work) = &options.work {
                quote!({
                    let __cq_source = (#value);
                    let __cq_work = (#work);
                    __cq_source.iter().inspect(move |_| {
                        __cq_work.add(|w| w.rows_visited += 1);
                    })
                })
            } else {
                quote!((#value).iter())
            }
        }
        Operation::Select { value, rows, .. } => {
            quote!({
                let __cq_source = (#value);
                let __cq_selected = (#rows);
                let mut __cq_result = #set::new();
                for __cq_row in __cq_selected {
                    assert!(__cq_source.contains(__cq_row),
                        "selected row is absent from the current input");
                    __cq_result.insert(*__cq_row);
                }
                __cq_result
            })
        }
        Operation::Materialize {
            owner, value, rows, ..
        } => {
            // Native storage does not evaluate the irrelevant block-size expression.
            let construction = match family {
                NativeSet::Hash => quote!((#rows)),
                NativeSet::Sorted => quote!((#rows).into_iter().collect()),
            };
            quote! {
                let #owner: #set<_> = #construction;
                let #value = &#owner;
            }
        }
        Operation::Blocks { .. } | Operation::Bounds { .. } | Operation::BlockRows { .. } => {
            return Err(syn::Error::new_spanned(
                operation,
                "this native row-set provider has no block descriptor lowering; select a block provider for this binding or another semijoin implementation",
            ));
        }
    };
    Ok(output)
}
