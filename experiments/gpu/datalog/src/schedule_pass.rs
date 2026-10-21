//! The scheduling pass alone consumes scheduling advice. It refines checked
//! source through strata and incremental drivers into device storage.
use proc_macro2::TokenStream;
use std::collections::BTreeSet;
use syn::parse::Parser;

pub fn compile(
    source: &cq_lang_datalog_source::Program,
    arguments: TokenStream,
) -> syn::Result<cq_lang_datalog_storage::Program> {
    let arguments = syn::punctuated::Punctuated::<syn::Ident, syn::Token![,]>::parse_terminated
        .parse2(arguments)?;
    let mut seen = BTreeSet::new();
    let mut driver = cq_datalog_incremental::DriverPolicy::WrittenOrder;
    let mut hoist = false;
    for argument in &arguments {
        if !seen.insert(argument.to_string()) {
            return Err(syn::Error::new_spanned(
                argument,
                "CQ_GPU_DATALOG_SCHEDULE_ARGUMENT: duplicate schedule advice",
            ));
        }
        match argument.to_string().as_str() {
            "delta_first" => driver = cq_datalog_incremental::DriverPolicy::DeltaFirst,
            "hoist_invariants" => hoist = true,
            _ => {
                return Err(syn::Error::new_spanned(
                    argument,
                    "CQ_GPU_DATALOG_SCHEDULE_ARGUMENT: expected delta_first or hoist_invariants",
                ));
            }
        }
    }
    let mut target = cq_datalog_stratify::compile(source)?;
    if hoist {
        target = cq_datalog_stratify::invariants::compile(&target)?;
    }
    let incremental = cq_datalog_incremental::compile_with_driver(&target, driver)?;
    cq_datalog_storage::compile_with_representation(
        &incremental,
        cq_datalog_storage::Representation::DeviceSet,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    #[test]
    fn driver_advice_refines_incremental_order_into_device_storage() {
        let source: cq_lang_datalog_source::Program = syn::parse_quote! {
            struct Reachability;
            relation Edge(x:i64,y:i64) = (edges);
            relation Reach(x:i64,y:i64);
            output Reach;
            Reach(*x,*y) :- Edge(x,y);
            Reach(*x,*z) :- Edge(y,z), Reach(x,y);
        };
        for (arguments, driver) in [
            (TokenStream::new(), 0),
            (quote!(delta_first), 1),
            (quote!(delta_first, hoist_invariants), 1),
        ] {
            let storage = compile(&source, arguments).unwrap();
            cq_lang_datalog_storage::contract::check(&storage).unwrap();
            let recursive = storage.incremental.variants.iter()
                .find(|variant| variant.rule == 1 && variant.delta == Some(1))
                .unwrap();
            assert_eq!(recursive.driver, Some(driver));
            let round = cq_datalog_fork::compile_with_ratio(&storage, 4).unwrap();
            assert_eq!(
                round.context.representation,
                cq_lang_datalog_round::KeyRepresentation::DeviceSetOwned,
            );
        }
    }
}
