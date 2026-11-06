//! Move a captured body into a separately partitioned runtime monomorphization.
use crate::RoundContext;
use proc_macro2::{Group, Ident, Span, TokenStream, TokenTree};
use quote::{format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};

/// Expand only the runtime calibration selector, leaving every other macro opaque.
/// This also lets syntax fixtures inspect exactly the feature-off realization.
pub fn calibration_branch(tokens: TokenStream, enabled: bool) -> TokenStream {
    use syn::parse::{ParseStream, Parser};
    fn select(input: ParseStream<'_>, enabled: bool) -> syn::Result<TokenStream> {
        let mut output = TokenStream::new();
        while !input.is_empty() {
            let candidate = input.fork().parse::<syn::Macro>().ok().filter(|mac| {
                mac.path.segments.len() == 2
                    && mac.path.segments[0].ident == "cq_storage"
                    && mac.path.segments[1].ident == "calibration_select"
            });
            if candidate.is_some() {
                let mac: syn::Macro = input.parse()?;
                let selected = (|input: ParseStream<'_>| {
                    let plain: Ident = input.parse()?;
                    if plain != "plain" { return Err(input.error("expected plain branch")); }
                    let body;
                    syn::braced!(body in input);
                    let plain: TokenStream = body.parse()?;
                    let calibration: Ident = input.parse()?;
                    if calibration != "calibration" { return Err(input.error("expected calibration branch")); }
                    let body;
                    syn::braced!(body in input);
                    let calibration: TokenStream = body.parse()?;
                    Ok(if enabled { calibration } else { plain })
                }).parse2(mac.tokens)?;
                output.extend(calibration_branch(selected, enabled));
            } else {
                let token: TokenTree = input.parse()?;
                match token {
                    TokenTree::Group(group) => {
                        let mut replacement = Group::new(group.delimiter(), calibration_branch(group.stream(), enabled));
                        replacement.set_span(group.span());
                        output.extend([TokenTree::Group(replacement)]);
                    }
                    token => output.extend([token]),
                }
            }
        }
        Ok(output)
    }
    (|input: ParseStream<'_>| select(input, enabled)).parse2(tokens)
        .expect("printer emits complete calibration selectors")
}

fn identifiers(tokens: TokenStream, names: &mut BTreeSet<String>) {
    for token in tokens {
        match token {
            TokenTree::Ident(name) => {
                names.insert(name.to_string());
            }
            TokenTree::Group(group) => identifiers(group.stream(), names),
            _ => {}
        }
    }
}

// Storage hosts are compiler-owned identifiers, distinct from CQ relation names,
// types, and binders. Substitution also traverses the still-unexpanded CQ macro:
// its host expressions are opaque to syn's Rust expression visitor.
fn substitute(tokens: TokenStream, names: &BTreeMap<String, Ident>) -> TokenStream {
    tokens
        .into_iter()
        .flat_map(|token| match token {
            TokenTree::Ident(name) if names.contains_key(&name.to_string()) => {
                let parameter = &names[&name.to_string()];
                quote!((*#parameter))
            }
            TokenTree::Group(group) => {
                let mut replacement =
                    Group::new(group.delimiter(), substitute(group.stream(), names));
                replacement.set_span(group.span());
                TokenStream::from(TokenTree::Group(replacement))
            }
            token => TokenStream::from(token),
        })
        .collect()
}

/// Replay recorded compiler-owned host bindings with the printer's exact
/// substitution rule. Each side must be a distinct identifier, never Rust code.
pub fn substitute_host_parameters(
    tokens: TokenStream,
    pairs: &[(&str, &str)],
) -> syn::Result<TokenStream> {
    let mut names = BTreeMap::new();
    let mut parameters = BTreeSet::new();
    for &(host, parameter) in pairs {
        let host: Ident = syn::parse_str(host)?;
        let parameter: Ident = syn::parse_str(parameter)?;
        if names.contains_key(&host.to_string()) {
            return Err(syn::Error::new_spanned(host, "duplicate host binding"));
        }
        if !parameters.insert(parameter.to_string()) {
            return Err(syn::Error::new_spanned(parameter, "duplicate host parameter"));
        }
        names.insert(host.to_string(), parameter);
    }
    Ok(substitute(tokens, &names))
}

pub(crate) fn split(
    body: TokenStream,
    context: &RoundContext,
    slot: usize,
    mutable: &BTreeSet<String>,
    locals: &BTreeSet<String>,
) -> TokenStream {
    // Capture discovery must not retain hosts referenced only by erased metadata.
    let plain = split_branch(calibration_branch(body.clone(), false), context, slot, mutable, locals, false);
    let calibrated = split_branch(calibration_branch(body, true), context, slot, mutable, locals, true);
    quote!(::cq_storage::calibration_select! {
        plain { #plain }
        calibration { #calibrated }
    })
}

fn split_branch(
    body: TokenStream,
    context: &RoundContext,
    slot: usize,
    mutable: &BTreeSet<String>,
    locals: &BTreeSet<String>,
    calibrated: bool,
) -> TokenStream {
    let mut used = BTreeSet::new();
    identifiers(body.clone(), &mut used);
    let hosts = context
        .relations
        .iter()
        .flat_map(|relation| {
            [&relation.next_canonical, &relation.next]
                .into_iter()
                .chain(relation.delta_host.iter())
        })
        .chain(
            context
                .layouts
                .iter()
                .chain(&context.delta_layouts)
                .map(|layout| &layout.host),
        );
    let mut names = BTreeMap::new();
    let mut arguments = Vec::new();
    let mut parameters = Vec::new();
    for host in hosts {
        let name = host.to_string();
        if !used.contains(&name) || locals.contains(&name) {
            continue;
        }
        let parameter = format_ident!(
            "__cq_codegen_{}_{}",
            slot,
            parameters.len(),
            span = Span::mixed_site()
        );
        arguments.push(if mutable.contains(&name) {
            quote!(&mut #host)
        } else {
            quote!(&#host)
        });
        parameters.push(parameter.clone());
        names.insert(name, parameter);
    }
    let body = substitute(body, &names);
    let module = format_ident!("partition_{}", slot % 16);
    let aliases = calibrated.then(|| {
        let aliases = names.iter().map(|(host, parameter)| {
            let replacement = parameter.to_string();
            quote!((#host, #replacement))
        });
        let guard = format_ident!("__cq_calibration_host_aliases_{}", slot, span = Span::mixed_site());
        quote!(let #guard = ::cq_storage::counters::calibration::host_alias_scope(&[#(#aliases),*]);)
    });
    quote!({ ::cq_datalog::__codegen::#module::run(
        (#(#arguments,)*),
        #[inline(always)] |(#(#parameters,)*)| { #aliases #body },
    ) })
}
