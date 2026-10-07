//! Instantiate the authored CQ callback in a real caller with CSV-backed hosts.
//! Inspect the schema to load tuples, then forward the original query tokens.
//! Compiled block regression lanes exercise native hosts, fallback per-prefix selection and preservation through the workload rewrite pipeline.
//!
//! Core blocks denote `bottom + Set`: `{ B1 } | { B2 }` unions projected
//! answers; `{ B1 } else { B2 }` selects the first nonempty arm. F is the free
//! heading and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Rust keeps a branch ancestry tree: union siblings unordered, fallback ordered;
//! each arm retains prefix Before facts and fallback anti steps. Rust `||` keeps
//! its existing recognition. Only differing literals on one interface column
//! prove union arms disjoint; no data keys or constraints are assumed.
//!
use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::{format_ident, quote};
use syn::{Ident, LitStr, Token, parse::Parse};
mod extended;

/// Execute an authored extension callback against all three tiny SQL goldens.
#[proc_macro]
pub fn instantiate_extended(input: TokenStream) -> TokenStream {
    extended::expand(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

struct Request {
    planner: Ident,
    join: Ident,
    project: Ident,
    intersect: Ident,
    host: Ident,
    distinct: Ident,
    indexes: Ident,
    lazy: Ident,
    printer: Ident,
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
            lazy: header.parse()?,
            printer: header.parse()?,
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
        lazy,
        printer,
        case,
        tokens,
    } = request;
    let query: cq_lang_cq::Program = syn::parse2(tokens.clone())?;
    let mut setup = Vec::new();
    let mut validation = Vec::new();
    for (i, declaration) in query.inputs().enumerate() {
        let storage = format_ident!("input_{i}");
        let name = &declaration.name;
        let loader = format_ident!("{}Loader", cq_lang_cq::symbol_name(name));
        let arity = declaration.columns.len();
        let fields: Vec<_> = (0..arity).map(syn::Index::from).collect();
        let tail: Vec<_> = (0..arity.saturating_sub(1)).map(syn::Index::from).collect();
        let method = match host.to_string().as_str() {
            "slice" => quote!(csv),
            "hashmap" => quote!(csv_hash::<1>),
            "btreemap" => quote!(csv_btree::<1>),
            _ => return Err(syn::Error::new_spanned(host, "unknown test host")),
        };
        let init = quote!(loaders::#loader::#method(
            directory.join(::cq_workloads::dataset::input_file_name(#i)),
            ::cq_load::CsvOptions::default(),
        ).unwrap());
        let rows = if host == "slice" {
            quote!(#storage.iter().map(|row| vec![#(row.#fields,)*]).collect())
        } else {
            quote!(#storage.iter().flat_map(|(key, values)| values.iter().map(move |value| vec![key.0, #(value.#tail,)*])).collect())
        };
        let relation_name = cq_lang_cq::symbol_name(name);
        validation.push(quote!(::cq_workloads::dataset::RelationData::new(#relation_name, #arity, #rows).unwrap()));
        let view = if host == "slice" {
            quote!(#storage.as_slice())
        } else {
            quote!(&#storage)
        };
        setup.push(quote!(let #storage = #init; let #name = #view;));
    }
    let plan = cq_workloads::pipelines::Planner::ALL
        .iter()
        .position(|p| planner == p.name())
        .ok_or_else(|| syn::Error::new_spanned(planner, "unknown planner pass"))?;
    let selected = cq_workloads::pipelines::Pipeline {
        plan,
        merge: join == "merge",
        canon: project == "canon",
        smallest: intersect == "smallest",
        ordered_dedup: distinct == "ordered_dedup",
        sorted_payload: indexes == "sorted",
        lazy: lazy == "lazy",
        push: printer == "push",
    };
    let configuration = selected.configuration(&query)?;
    let fields: Vec<_> = (0..query.query.head.args.len())
        .map(syn::Index::from)
        .collect();
    let compare = if matches!(mode, Mode::Push) {
        let pushed_configuration = cq_workloads::pipelines::Pipeline {
            push: true,
            ..selected
        }
        .configuration(&query)?;
        Some(quote! {
            let pushed = ::mini_linq::compile! { #pushed_configuration #tokens }
                .map(|row| vec![#(*row.#fields),*]).collect();
            cq_workloads::execution::check(#case, scenario, pushed);
        })
    } else {
        None
    };
    let test = (!matches!(mode, Mode::Compile)).then(|| quote!(#[test]));
    Ok(quote! {
        mod loaders { ::mini_linq::relation_loaders! { #tokens } }
        #test
        #[allow(non_snake_case)]
        pub fn corpus() {
            for &scenario in cq_workloads::execution::scenarios(#case) {
                let directory = cq_workloads::execution::directory(#case, scenario);
                #(#setup)*
                let schema = ::cq_workloads::case(#case).unwrap().module().unwrap();
                ::cq_workloads::dataset::Dataset::from_loaded_schema(
                    &schema.inputs().cloned().collect::<Vec<_>>(),
                    &directory,
                    vec![#(#validation,)*],
                ).unwrap();
                let answers = ::mini_linq::compile! { #configuration #tokens }
                    .map(|row| vec![#(*row.#fields),*]).collect();
                cq_workloads::execution::check(#case, scenario, answers);
                #compare
            }
        }
    })
}
