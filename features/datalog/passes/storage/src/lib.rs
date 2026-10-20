//! Choose the existing stable written-order storage refinement.
use cq_datalog_support::{ensure, key_columns};
use cq_lang_datalog_incremental as incremental;
use cq_lang_datalog_storage::{BodyItem, Layout, Policy, Program};
use cq_syntax::symbol_name;
use incremental::Variant;
use std::collections::BTreeSet;

pub fn compile(source: &incremental::Program) -> syn::Result<Program> {
    let analysis = incremental::contract::check(source)?;
    let (layouts, delta_layouts) = choose_layouts(&source.variants, &source.strata.source.rules);
    let mut target = Program {
        incremental: source.clone(),
        layouts,
        delta_layouts,
        reads: Vec::new(),
        negations: Vec::new(),
        policy: Policy {
            edb: syn::parse_quote!(canonical_borrowed_hash_set),
            all: syn::parse_quote!(vec),
            seen: syn::parse_quote!(hash_set),
            delta: syn::parse_quote!(vec),
            next: syn::parse_quote!(vec),
            layout: syn::parse_quote!(hash_map_vec_hash_state),
            insertion: syn::parse_quote!(hash_set_all_columns),
            negation_keys: syn::parse_quote!(hash_set_borrowed),
            full_maintenance: syn::parse_quote!(admitted_rows),
            delta_maintenance: syn::parse_quote!(replace_each_run),
        },
    };
    (target.reads, target.negations) = cq_lang_datalog_storage::read_plan(&target, &analysis)?;
    check(source, &target)?;
    Ok(target)
}
/// Formal rule CK-DL-Storage: independent data-refinement correspondence.
pub fn check(source: &incremental::Program, target: &Program) -> syn::Result<()> {
    incremental::contract::check(source)?;
    cq_lang_datalog_storage::contract::check(target)?;
    ensure(
        source == &target.incremental,
        target,
        "storage changed its incremental source",
    )
}
fn choose_layouts(
    variants: &[Variant],
    rules: &[incremental::strata::source::Query],
) -> (Vec<Layout>, Vec<Layout>) {
    let mut layouts = Vec::new();
    let mut delta_layouts = Vec::new();
    let mut seen = BTreeSet::new();
    let mut ordered: Vec<_> = variants.iter().collect();
    ordered.sort_by_key(|variant| (variant.rule, variant.delta));
    for variant in ordered {
        let mut bound = BTreeSet::new();
        for (position, item) in variant.query.body.iter().enumerate() {
            let BodyItem::Positive { atom } = item else {
                continue;
            };
            let (keys, key) = key_columns(atom, &bound);
            let is_delta = Some(position) == variant.delta;
            let relation = if is_delta {
                let BodyItem::Positive { atom: base } = &rules[variant.rule].body[position] else {
                    unreachable!("changing source position is a positive atom")
                };
                &base.relation
            } else {
                &atom.relation
            };
            if (!is_delta || key != 0 || position != 0)
                && seen.insert((is_delta, symbol_name(relation), keys.clone(), key))
            {
                let selected = if is_delta {
                    &mut delta_layouts
                } else {
                    &mut layouts
                };
                selected.push(Layout {
                    relation: relation.clone(),
                    columns: keys,
                    key,
                });
            }
            bound.extend(atom.variables().map(symbol_name));
        }
    }
    (layouts, delta_layouts)
}
