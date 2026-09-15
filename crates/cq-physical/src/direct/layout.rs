//! Admitted source domain and physical field bindings, independent of emission.
use std::collections::{BTreeMap, BTreeSet};

use cq_ir::{cq, relational_plan as ra, symbol_name};
use proc_macro2::Span;
use quote::format_ident;

pub(super) struct View {
    pub name: syn::Ident,
    pub provider: syn::Path,
    // Physical array positions are distinct from sorted logical headings.
    pub layout: Vec<String>,
}

pub(super) struct Layout {
    pub inputs: BTreeMap<String, syn::Ident>,
    pub views: BTreeMap<String, View>,
    pub output_columns: Vec<usize>,
}

impl Layout {
    pub fn view(&self, name: &syn::Ident) -> &View {
        &self.views[&symbol_name(name)]
    }
}

pub(super) fn analyze(
    source: &ra::Module,
    options: &super::PhysicalOptions,
) -> syn::Result<Layout> {
    ra::contract::check(source)?;
    let info = ra::check_in(source.program.inputs(), &source.plan)?;
    for input in &source.program.declarations {
        for column in &input.columns {
            if column.ty != syn::parse_quote!(i64) {
                return Err(syn::Error::new_spanned(
                    &column.ty,
                    "array physical providers currently require i64 columns",
                ));
            }
        }
    }
    if source.plan.output.head.args.len() > 12 {
        return Err(syn::Error::new_spanned(
            &source.plan.output,
            "block output supports at most 12 tuple fields",
        ));
    }
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
            ra::Operator::Project(_) | ra::Operator::NaturalJoin(_) => {
                info[&result].heading.iter().cloned().collect()
            }
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "array physical composition supports Unit, Rename, Project, Semijoin and NaturalJoin; choose another physical continuation for host expressions",
                ));
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
                    _ => options
                        .results
                        .get(&result)
                        .unwrap_or(&options.materialize)
                        .clone(),
                },
                layout,
            },
        );
    }
    let output = &views[&symbol_name(&source.plan.output.input)];
    let output_columns = source
        .plan
        .output
        .head
        .args
        .iter()
        .map(|expr| {
            let ident = cq::rust::variable(expr).ok_or_else(|| {
                syn::Error::new_spanned(expr, "block backend requires variable-only output")
            })?;
            output
                .layout
                .iter()
                .position(|v| *v == symbol_name(ident))
                .ok_or_else(|| {
                    syn::Error::new_spanned(
                        expr,
                        "block output must name an attribute of its input",
                    )
                })
        })
        .collect::<syn::Result<_>>()?;
    Ok(Layout {
        inputs,
        views,
        output_columns,
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
