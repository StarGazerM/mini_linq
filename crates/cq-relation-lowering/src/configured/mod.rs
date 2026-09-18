//! Explicit physical composition. Implementation paths are open; generated Rust
//! checks the operations and types of the selected implementations.
use cq_ir::{relational_plan as ra, symbol_name};
use quote::quote;
use std::collections::{BTreeMap, BTreeSet};
mod check;
mod expressions;
mod hosts;
mod layout;
mod lower;
mod resources;
#[cfg(test)]
mod tests;
mod wiring;
pub use check::{check_observed_program, check_observed_query, check_program, check_query};

/// Per-binding storage and per-operation lowering choices, with scoped defaults.
#[derive(Clone, Default)]
pub struct RelationOptions {
    pub access: Option<syn::Path>,
    pub inputs: BTreeMap<String, syn::Path>,
    pub result_rows: Option<syn::Path>,
    pub result_bindings: BTreeMap<String, syn::Path>,
    pub candidates: Option<syn::Path>,
    pub candidate_bindings: BTreeMap<String, syn::Path>,
    pub keys: Option<syn::Path>,
    pub key_sets: BTreeMap<String, syn::Path>,
    pub pairs: Option<syn::Path>,
    pub pair_bindings: BTreeMap<String, syn::Path>,
}
impl RelationOptions {
    fn resolve<'a>(
        &'a self,
        result: &syn::Ident,
        role: &str,
        overrides: &'a BTreeMap<String, syn::Path>,
        default: &'a Option<syn::Path>,
    ) -> syn::Result<&'a syn::Path> {
        overrides
            .get(&symbol_name(result))
            .or(default.as_ref())
            .ok_or_else(|| {
                syn::Error::new_spanned(
                    result,
                    format!("relation `{result}` requires a `{role}` implementation"),
                )
            })
    }
    fn access_for(&self, result: &syn::Ident) -> syn::Result<&syn::Path> {
        self.access.as_ref().ok_or_else(|| {
            syn::Error::new_spanned(
                result,
                format!("relation `{result}` requires an `access` implementation"),
            )
        })
    }
    fn result_rows_for(&self, result: &syn::Ident) -> syn::Result<&syn::Path> {
        self.resolve(
            result,
            "result_rows",
            &self.result_bindings,
            &self.result_rows,
        )
    }
    fn candidates_for(&self, result: &syn::Ident) -> syn::Result<&syn::Path> {
        self.resolve(
            result,
            "candidates",
            &self.candidate_bindings,
            &self.candidates,
        )
    }
    fn keys_for(&self, result: &syn::Ident) -> syn::Result<&syn::Path> {
        self.resolve(result, "keys", &self.key_sets, &self.keys)
    }
    fn pairs_for(&self, result: &syn::Ident) -> syn::Result<&syn::Path> {
        self.resolve(result, "pairs", &self.pair_bindings, &self.pairs)
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
                &self.result_bindings,
                0,
                "result providers may name only Unit, Project, Extend or NaturalJoin results; Rename, Select and Semijoin inherit their source provider",
            ),
            (
                &self.candidate_bindings,
                1,
                "candidate scans may name only Semijoin results",
            ),
            (
                &self.key_sets,
                1,
                "key providers may name only Semijoin results",
            ),
            (
                &self.pair_bindings,
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
                                    | ra::Operator::Extend(_)
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
        for definition in &source.plan.definitions {
            let result = &definition.result;
            match definition.operator {
                ra::Operator::Rename(_) => {}
                ra::Operator::Unit(_) => {
                    self.result_rows_for(result)?;
                }
                ra::Operator::Project(_) | ra::Operator::Extend(_) => {
                    self.access_for(result)?;
                    self.result_rows_for(result)?;
                }
                ra::Operator::Select(_) => {
                    self.access_for(result)?;
                }
                ra::Operator::Semijoin(_) => {
                    self.access_for(result)?;
                    self.candidates_for(result)?;
                    self.keys_for(result)?;
                }
                ra::Operator::NaturalJoin(_) => {
                    self.result_rows_for(result)?;
                    self.pairs_for(result)?;
                }
            }
        }
        Ok(())
    }
}
/// Counter storage belongs only to the observed host, independently of row storage.
#[derive(Clone)]
pub struct ObservedOptions {
    pub relations: RelationOptions,
    pub counter_storage: syn::Path,
}

/// Shared definition lowering; hosts own parameter, output and observation scope.
fn statements(
    source: &ra::Module,
    options: &RelationOptions,
    layout: &layout::Layout,
    wiring: &wiring::Wiring,
) -> syn::Result<Vec<(proc_macro2::TokenStream, proc_macro2::TokenStream)>> {
    let resources = resources::analyze(source, layout, options)?;
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
            ra::Operator::Project(p) => lower::project(
                layout.view(&p.input),
                &target,
                options.access_for(&definition.result)?,
                wiring,
            )?,
            ra::Operator::Select(op) => lower::select(
                layout.view(&op.input),
                view,
                &op.condition,
                options.access_for(&definition.result)?,
                wiring,
            )?,
            ra::Operator::Extend(op) => lower::extend(
                layout.view(&op.input),
                &target,
                op,
                options.access_for(&definition.result)?,
                wiring,
            )?,
            ra::Operator::Semijoin(op) => lower::semijoin(
                layout.view(&op.left),
                view,
                &resources.entries[resources.uses[&result]],
                options.candidates_for(&definition.result)?,
                options.access_for(&definition.result)?,
                wiring,
            )?,
            ra::Operator::NaturalJoin(op) => lower::join(
                layout.view(&op.left),
                layout.view(&op.right),
                &target,
                options.pairs_for(&definition.result)?,
                wiring,
            )?,
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

pub use hosts::observed::{compile_observed_program, compile_observed_query};
pub use hosts::plain::{compile_program, compile_query};
