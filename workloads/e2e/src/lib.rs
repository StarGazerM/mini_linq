//! Instantiate the authored CQ callback in a real caller with CSV-backed hosts.
//! Inspect the schema to load tuples, then forward the original query tokens.
use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::{format_ident, quote};
use syn::{Ident, LitStr, Token, parse::Parse};

struct Request {
    planner: Ident,
    join: Ident,
    project: Ident,
    intersect: Ident,
    host: Ident,
    distinct: Ident,
    indexes: Ident,
    case: LitStr,
    tokens: Tokens,
}

impl Parse for Request {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let header;
        syn::bracketed!(header in input);
        let request = Self {
            planner: header.parse()?,
            join: header.parse()?,
            project: header.parse()?,
            intersect: header.parse()?,
            host: header.parse()?,
            distinct: header.parse()?,
            indexes: header.parse()?,
            case: {
                header.parse::<Token![;]>()?;
                header.parse()?
            },
            tokens: input.parse()?,
        };
        if !header.is_empty() {
            return Err(header.error("unexpected configuration fields"));
        }
        Ok(request)
    }
}

#[proc_macro]
pub fn instantiate(input: TokenStream) -> TokenStream {
    entry(input, Mode::Test)
}

#[proc_macro]
pub fn compare_push(input: TokenStream) -> TokenStream {
    entry(input, Mode::Push)
}

#[proc_macro]
pub fn compile_query(input: TokenStream) -> TokenStream {
    entry(input, Mode::Compile)
}

#[derive(Clone, Copy)]
enum Mode {
    Test,
    Push,
    Compile,
}

fn entry(input: TokenStream, mode: Mode) -> TokenStream {
    syn::parse(input)
        .and_then(|r| expand(r, mode))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand(request: Request, mode: Mode) -> syn::Result<Tokens> {
    let Request {
        planner,
        join,
        project,
        intersect,
        host,
        distinct,
        indexes,
        case,
        tokens,
    } = request;
    // The policy's authored name for a probe join is `seek`.
    let join = if join == "probe" {
        syn::parse_quote!(seek)
    } else {
        join
    };
    let query: cq_lang_cq::Program = syn::parse2(tokens.clone())?;
    let mut setup = Vec::new();
    for (i, declaration) in query.inputs().enumerate() {
        let storage = format_ident!("input_{i}");
        let name = &declaration.name;
        let arity = declaration.columns.len();
        let fields: Vec<_> = (0..arity).collect();
        let tail: Vec<_> = (1..arity).collect();
        let rows = quote!(dataset.relations[#i].rows.iter());
        let init = match host.to_string().as_str() {
            "slice" => quote!(#rows.map(|row| (#(row[#fields],)*)).collect::<Vec<_>>()),
            "hashmap" | "btreemap" => {
                let map = if host == "hashmap" {
                    quote!(std::collections::HashMap)
                } else {
                    quote!(std::collections::BTreeMap)
                };
                quote!({
                    let mut groups = #map::<_, Vec<_>>::new();
                    for row in #rows {
                        groups.entry((row[0],)).or_default().push((#(row[#tail],)*));
                    }
                    groups
                })
            }
            _ => return Err(syn::Error::new_spanned(host, "unknown test host")),
        };
        let view = if host == "slice" {
            quote!(#storage.as_slice())
        } else {
            quote!(&#storage)
        };
        setup.push(quote!(let #storage = #init; let #name = #view;));
    }
    let provider = match indexes.to_string().as_str() {
        "default" => quote! {
            pub(super) use cq_state_providers::default_indexes as index_provider;
        },
        "custom" => quote! {
            #[allow(unused_macros)]
            macro_rules! custom_indexes {
                (probe) => { crate::support::SortedIndex<_, _> };
                ($($rest:tt)*) => { cq_state_providers::default_indexes!($($rest)*) };
            }
            pub(super) use custom_indexes as index_provider;
        },
        _ => return Err(syn::Error::new_spanned(indexes, "unknown index provider")),
    };
    let fields: Vec<_> = (0..query.query.head.args.len())
        .map(syn::Index::from)
        .collect();
    let compare = matches!(mode, Mode::Push).then(|| quote! {
        let pushed = {
            mod push_wiring {
                pub(super) use super::mini_linq::{choose_index_policy, physical_policy, select_structures, index_provider};
                pub(super) use own_push::{push as structures_to_rust, set_sink as distinct_state};
            }
            use push_wiring as mini_linq;
            ::mini_linq::#planner! { #tokens }.map(|row| vec![#(*row.#fields),*]).collect()
        };
        cq_workloads::execution::check(#case, scenario, pushed);
    });
    let test = (!matches!(mode, Mode::Compile)).then(|| quote!(#[test]));
    Ok(quote! {
        macro_rules! strategy {
            ($($ix:tt)*) => { ::mini_linq::lower_physical! {
                $($ix)* policy { join = #join; project = #project; intersect = #intersect; }
            } };
        }
        use strategy as physical_policy;
        mod mini_linq {
            pub(super) use super::physical_policy;
            pub(super) use ::mini_linq::{index_demands as choose_index_policy, select_structures, structures_to_rust};
            pub(super) use cq_state_providers::#distinct as distinct_state;
            #provider
        }
        #test
        #[allow(non_snake_case)]
        pub fn corpus() {
            for &scenario in cq_workloads::execution::scenarios(#case) {
                let dataset = cq_workloads::execution::load(#case, scenario);
                #(#setup)*
                let answers = ::mini_linq::#planner! { #tokens }
                    .map(|row| vec![#(*row.#fields),*]).collect();
                cq_workloads::execution::check(#case, scenario, answers);
                #compare
            }
        }
    })
}
