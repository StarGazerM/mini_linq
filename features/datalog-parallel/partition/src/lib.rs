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
