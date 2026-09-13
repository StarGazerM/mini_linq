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
    pub inputs: BTreeMap<String, syn::Path>,
    pub materialize: syn::Path,
    pub results: BTreeMap<String, syn::Path>,
    pub semijoin: syn::Path,
    pub semijoins: BTreeMap<String, syn::Path>,
    pub keys: syn::Path,
    pub key_sets: BTreeMap<String, syn::Path>,
    pub join: syn::Path,
    pub joins: BTreeMap<String, syn::Path>,
}
impl PhysicalOptions {
    fn recipe(&self, result: &str) -> &syn::Path {
        self.semijoins.get(result).unwrap_or(&self.semijoin)
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
                &self.semijoins,
                1,
                "semijoin recipes may name only Semijoin results",
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
/// Compatibility entry composing CP-Defer and CP-Realize in their declared order.
pub fn compile_physical(source: &ra::Module, options: &PhysicalOptions) -> syn::Result<syn::File> {
    let indexed = crate::passes::compile_deferred_indexes(source)?;
    crate::passes::compile_composed(&indexed, options)
}

/// Physical implementation body; checked stage orchestration lives in passes.rs.
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
                definition,
                layout.view(&op.left),
                view,
                &resources.entries[resources.uses[&result]],
                &wiring,
            )?,
            ra::Operator::NaturalJoin(op) => lower::join(
                definition,
                layout.view(&op.left),
                layout.view(&op.right),
                &target,
                options.joins.get(&result).unwrap_or(&options.join),
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

/// Convenience facade selecting block storage for every binding. General
/// PhysicalOptions composition does not inherit these convenience defaults.
pub struct BlockOptions {
    pub storage: syn::Path,
    pub access: syn::Path,
    pub keys: syn::Path,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockSemijoin {
    Exact,
    ZoneMap,
}
pub type BlockChoices = BTreeMap<String, BlockSemijoin>;
fn uniform(source: &ra::Module, choice: BlockSemijoin) -> BlockChoices {
    source
        .plan
        .definitions
        .iter()
        .filter(|d| matches!(d.operator, ra::Operator::Semijoin(_)))
        .map(|d| (symbol_name(&d.result), choice))
        .collect()
}
fn block_options(
    source: &ra::Module,
    options: &BlockOptions,
    choices: &BlockChoices,
) -> syn::Result<PhysicalOptions> {
    if uniform(source, BlockSemijoin::Exact)
        .keys()
        .collect::<Vec<_>>()
        != choices.keys().collect::<Vec<_>>()
    {
        return Err(syn::Error::new_spanned(
            source,
            "block choices must name exactly the semijoin results",
        ));
    }
    Ok(PhysicalOptions {
        storage: options.storage.clone(),
        access: options.access.clone(),
        inputs: source
            .program
            .inputs()
            .map(|r| {
                (
                    symbol_name(&r.name),
                    syn::parse_quote!(::mini_linq::block_rows),
                )
            })
            .collect(),
        materialize: syn::parse_quote!(::mini_linq::block_rows),
        results: BTreeMap::new(),
        semijoin: syn::parse_quote!(::mini_linq::exact_semijoin),
        semijoins: choices
            .iter()
            .map(|(name, choice)| {
                (
                    name.clone(),
                    match choice {
                        BlockSemijoin::Exact => syn::parse_quote!(::mini_linq::exact_semijoin),
                        BlockSemijoin::ZoneMap => syn::parse_quote!(::mini_linq::zone_semijoin),
                    },
                )
            })
            .collect(),
        keys: options.keys.clone(),
        key_sets: BTreeMap::new(),
        join: syn::parse_quote!(::mini_linq::hash_join),
        joins: BTreeMap::new(),
    })
}
pub fn compile_blocked(source: &ra::Module, options: &BlockOptions) -> syn::Result<syn::File> {
    compile_blocked_with(source, options, &uniform(source, BlockSemijoin::Exact))
}
pub fn compile_zonemap(source: &ra::Module, options: &BlockOptions) -> syn::Result<syn::File> {
    compile_blocked_with(source, options, &uniform(source, BlockSemijoin::ZoneMap))
}
pub fn compile_blocked_with(
    source: &ra::Module,
    options: &BlockOptions,
    choices: &BlockChoices,
) -> syn::Result<syn::File> {
    compile_physical(source, &block_options(source, options, choices)?)
}
pub fn check_blocked_with(
    source: &ra::Module,
    target: &syn::File,
    options: &BlockOptions,
    choices: &BlockChoices,
) -> syn::Result<()> {
    check_physical(source, target, &block_options(source, options, choices)?)
}
pub fn check_blocked_resources(
    source: &ra::Module,
    target: &syn::File,
    options: &BlockOptions,
) -> syn::Result<()> {
    check_blocked_with(
        source,
        target,
        options,
        &uniform(source, BlockSemijoin::Exact),
    )
}
pub fn check_zonemap_resources(
    source: &ra::Module,
    target: &syn::File,
    options: &BlockOptions,
) -> syn::Result<()> {
    check_blocked_with(
        source,
        target,
        options,
        &uniform(source, BlockSemijoin::ZoneMap),
    )
}
