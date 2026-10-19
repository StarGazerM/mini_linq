//! The scheduling pass alone consumes scheduling advice. Its result is the
//! ordinary independently checked Datalog schedule, including driver identities.
use proc_macro2::TokenStream;
use std::collections::BTreeSet;
use syn::parse::Parser;

pub fn compile(
    source: &cq_lang_datalog::Program,
    arguments: TokenStream,
) -> syn::Result<cq_lang_datalog::Program> {
    let arguments = syn::punctuated::Punctuated::<syn::Ident, syn::Token![,]>::parse_terminated
        .parse2(arguments)?;
    let mut seen = BTreeSet::new();
    let mut driver = cq_datalog_schedule::seminaive::DriverPolicy::WrittenOrder;
    let mut hoist = false;
    for argument in &arguments {
        if !seen.insert(argument.to_string()) {
            return Err(syn::Error::new_spanned(
                argument,
                "CQ_GPU_DATALOG_SCHEDULE_ARGUMENT: duplicate schedule advice",
            ));
        }
        match argument.to_string().as_str() {
            "delta_first" => driver = cq_datalog_schedule::seminaive::DriverPolicy::DeltaFirst,
            "hoist_invariants" => hoist = true,
            _ => {
                return Err(syn::Error::new_spanned(
                    argument,
                    "CQ_GPU_DATALOG_SCHEDULE_ARGUMENT: expected delta_first or hoist_invariants",
                ));
            }
        }
    }
    cq_lang_datalog::contract::check(source)?;
    if source.variants.is_some() && !arguments.is_empty() {
        return Err(syn::Error::new_spanned(
            source,
            "CQ_GPU_DATALOG_SCHEDULE_ARGUMENT: cannot reselect an already recorded schedule",
        ));
    }
    let mut target = source.clone();
    if target.strata.is_none() {
        target = cq_datalog_schedule::stratify::compile(&target)?;
    }
    if hoist {
        target = cq_datalog_schedule::invariants::compile(&target)?;
    }
    if target.variants.is_none() {
        target = cq_datalog_schedule::seminaive::compile_with_driver(
            &target,
            cq_datalog_schedule::seminaive::Representation::DeviceSet,
            driver,
        )?;
    }
    Ok(target)
}
