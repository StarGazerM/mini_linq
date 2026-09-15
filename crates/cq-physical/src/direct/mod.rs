//! Explicit physical composition. Implementation paths are open; generated Rust
//! checks the operations and types of the selected implementations.
use cq_ir::{relational_plan as ra, symbol_name};
use quote::quote;
use std::collections::{BTreeMap, BTreeSet};
mod check;
mod hosts;
mod layout;
mod lower;
mod resources;
#[cfg(test)]
mod tests;
mod wiring;
pub use check::{
    check_observed_physical, check_observed_physical_query, check_physical, check_physical_query,
};

/// Per-binding storage and per-operation lowering choices, with scoped defaults.
#[derive(Clone)]
pub struct PhysicalOptions {
    pub access: syn::Path,
    pub inputs: BTreeMap<String, syn::Path>,
    pub materialize: syn::Path,
    pub results: BTreeMap<String, syn::Path>,
    pub scan: syn::Path,
    pub scans: BTreeMap<String, syn::Path>,
    pub keys: syn::Path,
    pub key_sets: BTreeMap<String, syn::Path>,
    pub join: syn::Path,
    pub joins: BTreeMap<String, syn::Path>,
}
impl PhysicalOptions {
    fn keys(&self, result: &str) -> &syn::Path {
        self.key_sets.get(result).unwrap_or(&self.keys)
    }
    fn validate(&self, source: &ra::Module) -> syn::Result<()> {
        let inputs: BTreeSet<_> = source
            .program
            .inputs()
            .map(|r| symbol_name(&r.name))
            .collect();
        if inputs != self.inputs.keys().cloned().collect() {
            return Err(syn::Error::new_spanned(
                source,
                "physical inputs must name exactly the input relations",
            ));
        }
        for (map, admits, message) in [
            (
                &self.results,
                0,
                "result providers may name only Unit, Project or NaturalJoin results; Rename and Semijoin inherit their source provider",
            ),
            (
                &self.scans,
                1,
                "candidate scans may name only Semijoin results",
            ),
            (
                &self.key_sets,
                1,
                "key providers may name only Semijoin results",
            ),
            (
                &self.joins,
                2,
                "join recipes may name only NaturalJoin results",
            ),
        ] {
            for (name, path) in map {
                let valid = source.plan.definitions.iter().any(|d| {
                    symbol_name(&d.result) == *name
                        && match admits {
                            0 => matches!(
                                d.operator,
                                ra::Operator::Unit(_)
                                    | ra::Operator::Project(_)
                                    | ra::Operator::NaturalJoin(_)
                            ),
                            1 => matches!(d.operator, ra::Operator::Semijoin(_)),
                            _ => matches!(d.operator, ra::Operator::NaturalJoin(_)),
                        }
                });
                if !valid {
                    return Err(syn::Error::new_spanned(path, message));
                }
            }
        }
        Ok(())
    }
}
/// Counter storage belongs only to the observed host, independently of row storage.
#[derive(Clone)]
pub struct ObservedOptions {
    pub physical: PhysicalOptions,
    pub storage: syn::Path,
}

/// Shared definition lowering; hosts own parameter, output and observation scope.
fn statements(
    source: &ra::Module,
    options: &PhysicalOptions,
    layout: &layout::Layout,
    wiring: &wiring::Wiring,
) -> syn::Result<Vec<(proc_macro2::TokenStream, proc_macro2::TokenStream)>> {
    let resources = resources::analyze(source, layout, options);
    let mut statements = Vec::new();
    for (i, definition) in source.plan.definitions.iter().enumerate() {
        let result = symbol_name(&definition.result);
        let view = &layout.views[&result];
        let provider = wiring.provider(&view.provider);
        let name = &view.name;
        for resource in resources.entries.iter().filter(|r| r.first_use == i) {
            statements.push((resources::emit(resource, source, layout, wiring)?, quote!()));
        }
        let target = lower::Materialized {
            view,
            owner: layout::local("owner", i),
        };
        let statement = match &definition.operator {
            ra::Operator::Rename(rename) => {
                let input = &layout.inputs[&symbol_name(&rename.relation)];
                quote!(let #name = #provider! { input (#input) };)
            }
            ra::Operator::Unit(_) => lower::unit(&target, wiring),
            ra::Operator::Project(p) => {
                lower::project(layout.view(&p.input), &target, options, wiring)?
            }
            ra::Operator::Semijoin(op) => lower::semijoin(
                layout.view(&op.left),
                view,
                &resources.entries[resources.uses[&result]],
                options.scans.get(&result).unwrap_or(&options.scan),
                options,
                wiring,
            )?,
            ra::Operator::NaturalJoin(op) => lower::join(
                layout.view(&op.left),
                layout.view(&op.right),
                &target,
                options.joins.get(&result).unwrap_or(&options.join),
                options,
                wiring,
            )?,
            _ => unreachable!("layout checks the admitted source domain"),
        };
        statements.push((
            statement,
            quote! {
                __ml_stats.steps.push((#result, #provider! { len (#name) }, __ml_work.take()));
            },
        ));
    }
    Ok(statements)
}

pub use hosts::observed::{compile_observed_physical, compile_observed_physical_query};
pub use hosts::plain::{compile_physical, compile_physical_query};
