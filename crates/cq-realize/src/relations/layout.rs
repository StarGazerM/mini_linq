//! Admitted source domain and physical field bindings, independent of emission.
use std::collections::{BTreeMap, BTreeSet};

use cq_ir::{relational_plan as ra, symbol_name};
use proc_macro2::Span;
use quote::format_ident;

pub(super) struct View {
    pub name: syn::Ident,
    pub provider: syn::Path,
    // Physical field positions are distinct from sorted logical headings.
    pub layout: Vec<String>,
    pub info: ra::RelationInfo,
}

pub(super) struct Layout {
    pub inputs: BTreeMap<String, syn::Ident>,
    pub views: BTreeMap<String, View>,
    pub output_types: Vec<syn::Type>,
}

impl Layout {
    pub fn view(&self, name: &syn::Ident) -> &View {
        &self.views[&symbol_name(name)]
    }
}

pub(super) fn analyze(
    source: &ra::Module,
    options: &super::RelationOptions,
) -> syn::Result<Layout> {
    ra::contract::check(source)?;
    let info = ra::check_in(source.program.inputs(), &source.plan)?;
    let inputs = source
        .program
        .inputs()
        .enumerate()
        .map(|(i, input)| (symbol_name(&input.name), local("input", i)))
        .collect();
    let mut views: BTreeMap<String, View> = BTreeMap::new();
    for (i, definition) in source.plan.definitions.iter().enumerate() {
        let result = symbol_name(&definition.result);
        let layout = match &definition.operator {
            ra::Operator::Rename(rename) => {
                let schema = source
                    .program
                    .inputs()
                    .find(|d| symbol_name(&d.name) == symbol_name(&rename.relation))
                    .unwrap();
                schema
                    .columns
                    .iter()
                    .map(|column| {
                        symbol_name(
                            &rename
                                .mapping
                                .iter()
                                .find(|m| symbol_name(&m.source) == symbol_name(&column.name))
                                .unwrap()
                                .target,
                        )
                    })
                    .collect()
            }
            ra::Operator::Unit(_) => Vec::new(),
            ra::Operator::Semijoin(op) => views[&symbol_name(&op.left)].layout.clone(),
            ra::Operator::Select(op) => views[&symbol_name(&op.input)].layout.clone(),
            ra::Operator::Project(_) | ra::Operator::NaturalJoin(_) | ra::Operator::Extend(_) => {
                info[&result].heading.iter().cloned().collect()
            }
        };
        if layout.len() != info[&result].heading.len()
            || layout.iter().cloned().collect::<BTreeSet<_>>() != info[&result].heading
        {
            return Err(syn::Error::new_spanned(
                definition,
                "physical view layout disagrees with its logical heading",
            ));
        }
        views.insert(
            result.clone(),
            View {
                name: local("view", i),
                provider: match &definition.operator {
                    ra::Operator::Rename(op) => options.inputs[&symbol_name(&op.relation)].clone(),
                    ra::Operator::Semijoin(op) => views[&symbol_name(&op.left)].provider.clone(),
                    ra::Operator::Select(op) => views[&symbol_name(&op.input)].provider.clone(),
                    _ => options.materialize_for(&definition.result)?.clone(),
                },
                layout,
                info: info[&result].clone(),
            },
        );
    }
    let output_types = ra::output_types(
        &source.program,
        &info[&symbol_name(&source.plan.output.input)],
    )?
    .iter()
    .map(crate::rust_bindings::qualify_type)
    .collect();
    Ok(Layout {
        inputs,
        views,
        output_types,
    })
}

pub(super) fn local(prefix: &str, index: usize) -> syn::Ident {
    format_ident!("__ml_{}{}", prefix, index, span = Span::mixed_site())
}

pub(super) fn positions(layout: &[String], names: &[String]) -> Vec<usize> {
    names
        .iter()
        .map(|n| layout.iter().position(|v| v == n).unwrap())
        .collect()
}

// Layouts have already been checked against logical headings. Sorting the whole
// intersection fixes matching field roles on both sides, including empty keys.
pub(super) fn separator(left: &View, right: &View) -> Vec<String> {
    left.layout
        .iter()
        .filter(|v| right.layout.contains(v))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
