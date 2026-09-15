//! Explicit physical composition. Implementation paths are open; generated Rust
//! checks the operations and types of the selected implementations.
use cq_ir::{relational_plan as ra, symbol_name};
use quote::quote;
use std::collections::{BTreeMap, BTreeSet};
mod api;
mod check;
mod layout;
mod lower;
mod resources;
#[cfg(test)]
mod tests;
mod wiring;
pub use check::check_physical;

/// Per-binding storage and per-operation lowering choices, with scoped defaults.
pub struct PhysicalOptions {
    pub storage: syn::Path,
    pub access: syn::Path,
    pub iterator: syn::Path,
    pub inputs: BTreeMap<String, syn::Path>,
    pub materialize: syn::Path,
    pub results: BTreeMap<String, syn::Path>,
    pub scan: syn::Path,
    pub scans: BTreeMap<String, syn::Path>,
    pub key_provider: syn::Path,
    pub key_providers: BTreeMap<String, syn::Path>,
    pub keys: syn::Path,
    pub key_sets: BTreeMap<String, syn::Path>,
    pub join: syn::Path,
    pub joins: BTreeMap<String, syn::Path>,
}
impl PhysicalOptions {
    fn key_provider(&self, result: &str) -> &syn::Path {
        self.key_providers.get(result).unwrap_or(&self.key_provider)
    }
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
                &self.key_providers,
                1,
                "key providers may name only Semijoin results",
            ),
            (
                &self.key_sets,
                1,
                "key collections may name only Semijoin results",
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
/// RA realization using compiler-local layouts and native Access operations.
pub(crate) fn compile(source: &ra::Module, options: &PhysicalOptions) -> syn::Result<syn::File> {
    options.validate(source)?;
    let layout = layout::analyze(source, options)?;
    let resources = resources::analyze(source, &layout, options);
    let wiring = wiring::Wiring::new(source, &layout, options);
    let mut statements = Vec::new();
    for (i, definition) in source.plan.definitions.iter().enumerate() {
        let result = symbol_name(&definition.result);
        let view = &layout.views[&result];
        let provider = wiring.provider(&view.provider);
        let name = &view.name;
        for resource in resources.entries.iter().filter(|r| r.first_use == i) {
            statements.push((resources::emit(resource, source, &layout, &wiring)?, None));
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
            ra::Operator::Unit(_) => lower::unit(&target, &wiring),
            ra::Operator::Project(p) => {
                lower::project(layout.view(&p.input), &target, options, &wiring)?
            }
            ra::Operator::Semijoin(op) => lower::semijoin(
                layout.view(&op.left),
                view,
                &resources.entries[resources.uses[&result]],
                options.scans.get(&result).unwrap_or(&options.scan),
                options,
                &wiring,
            )?,
            ra::Operator::NaturalJoin(op) => lower::join(
                layout.view(&op.left),
                layout.view(&op.right),
                &target,
                options.joins.get(&result).unwrap_or(&options.join),
                options,
                &wiring,
            )?,
            _ => unreachable!("layout checks the admitted source domain"),
        };
        statements.push((
            statement,
            Some(quote! {
                __ml_stats.steps.push((#result, #provider! { len (#name) }, __ml_work.take()));
            }),
        ));
    }
    api::emit(source, &layout, &statements, options, &wiring)
}

/// Configured RA -> Rust[Access]. Layouts and resource reuse remain local to
/// this compiler edge; there is no empty index overlay or contextual language.
pub fn compile_physical(source: &ra::Module, options: &PhysicalOptions) -> syn::Result<syn::File> {
    ra::contract::check(source)?;
    let target = compile(source, options)?;
    check_physical(source, &target, options)?;
    Ok(target)
}
