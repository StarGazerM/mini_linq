//! Slice-one execution is an erasure-preserving Round rewrite. Exactly one
//! leading immutable scan per eligible rule is replaced by a fresh flat input.
//! All other occurrences, opaque Rust steps, queues and publication stay fixed.
use cq_datalog_support::Fresh;
use cq_lang_datalog_round::{self as round, Execution, Program};
use proc_macro2::TokenStream;

pub fn compile(source: &Program) -> syn::Result<Program> {
    compile_with(source, TokenStream::new())
}

pub fn compile_with(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    if !arguments.is_empty() {
        return Err(syn::Error::new_spanned(
            arguments,
            "partition takes no arguments",
        ));
    }
    round::contract::check(source)?;
    if !source.execution.is_empty() {
        return Err(syn::Error::new_spanned(
            source,
            "partition requires an unannotated Round program",
        ));
    }
    let mut target = source.clone();
    let mut fresh = Fresh::new(source);
    for stratum in &source.context.strata {
        for rule in stratum.seeds.iter().chain(&stratum.steps) {
            let Some(scan) = &rule.scan else { continue };
            let binding = fresh.id("parallel_chunk");
            let hole = &mut target.holes[rule.hole].cq;
            let declaration = hole
                .declarations
                .iter_mut()
                .find(|declaration| declaration.name == scan.binding)
                .expect("checked scan declaration");
            declaration.name = binding.clone();
            declaration.value = Some(syn::parse_quote!(= (#binding)));
            let round::cq::BodyItem::Positive { atom } = &mut hole.query.body[0] else {
                unreachable!("checked leading scan")
            };
            atom.relation = binding.clone();
            target.execution.push(Execution {
                hole: rule.hole,
                occurrence: 0,
                binding,
                strategy: syn::parse_quote!(rayon_slice1),
            });
        }
    }
    check(source, &target)?;
    Ok(target)
}

/// Formal rule CK-DL-Partition: Certify exact scan coverage and reversible single-occurrence substitution.
pub fn check(source: &Program, target: &Program) -> syn::Result<()> {
    round::contract::check(source)?;
    round::contract::check(target)?;
    if !source.execution.is_empty() || round::erase(target)? != *source {
        return Err(syn::Error::new_spanned(
            target,
            "partition changed the erased Round program or its CQ pipelines",
        ));
    }
    let expected = source
        .context
        .strata
        .iter()
        .flat_map(|stratum| stratum.seeds.iter().chain(&stratum.steps))
        .filter(|rule| rule.scan.is_some());
    let mut annotations = target.execution.iter();
    for rule in expected {
        let scan = rule.scan.as_ref().expect("filtered scan");
        let slice_backed = match scan.source {
            round::InputSource::Delta(index) => {
                vector_element(&source.context.relations[index].providers.delta).is_some()
            }
            round::InputSource::Layout(index) => {
                let provider = &source.context.layouts[index].provider;
                map_payload(provider).and_then(vector_element).is_some()
            }
        };
        if !slice_backed {
            return Err(syn::Error::new_spanned(
                target,
                "slice partition requires a ::std::vec::Vec delta or layout bucket provider",
            ));
        }
        let Some(annotation) = annotations.next() else {
            return Err(syn::Error::new_spanned(
                target,
                "partition omitted an eligible scan",
            ));
        };
        if annotation.hole != rule.hole
            || annotation.occurrence != 0
            || annotation.strategy != "rayon_slice1"
        {
            return Err(syn::Error::new_spanned(
                target,
                "partition changed scan coverage, occurrence or execution strategy",
            ));
        }
    }
    if annotations.next().is_some() {
        return Err(syn::Error::new_spanned(
            target,
            "partition added an ineligible scan",
        ));
    }
    Ok(())
}

fn vector_element(ty: &syn::Type) -> Option<&syn::Type> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    if path.qself.is_some() || path.path.leading_colon.is_none() || path.path.segments.len() != 3 {
        return None;
    }
    let mut segments = path.path.segments.iter();
    for name in ["std", "vec"] {
        let segment = segments.next()?;
        if segment.ident != name || !matches!(segment.arguments, syn::PathArguments::None) {
            return None;
        }
    }
    let segment = segments.next()?;
    if segment.ident != "Vec" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    if arguments.args.len() != 1 {
        return None;
    }
    match arguments.args.first()? {
        syn::GenericArgument::Type(element) => Some(element),
        _ => None,
    }
}

fn map_payload(ty: &syn::Type) -> Option<&syn::Type> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    if path.qself.is_some() {
        return None;
    }
    let syn::PathArguments::AngleBracketed(arguments) = &path.path.segments.last()?.arguments
    else {
        return None;
    };
    match arguments.args.iter().nth(1)? {
        syn::GenericArgument::Type(payload) => Some(payload),
        _ => None,
    }
}
