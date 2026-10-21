//! Choose the existing stable written-order storage refinement.
use cq_datalog_support::{ensure, key_columns};
use cq_lang_datalog_incremental as incremental;
use cq_lang_datalog_storage::{BodyItem, Layout, Policy, Program, Providers};
use cq_syntax::symbol_name;
use std::collections::BTreeSet;

pub fn compile(source: &incremental::Program) -> syn::Result<Program> {
    let analysis = incremental::contract::check(source)?;
    let (layouts, delta_layouts) = choose_layouts(source, &analysis);
    let mut target = Program {
        incremental: source.clone(),
        providers: source
            .strata
            .source
            .declarations
            .iter()
            .map(|declaration| {
                let types: Vec<_> = declaration
                    .columns
                    .iter()
                    .map(|column| &column.ty)
                    .collect();
                Providers {
                    canonical_name: syn::parse_quote!(Canonical),
                    canonical_set: if declaration.value.is_some() {
                        syn::parse_quote!(::std::collections::HashSet<R, ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>>)
                    } else {
                        syn::parse_quote!(::cq_storage::HashSet<R>)
                    },
                    all: syn::parse_quote!(::std::vec::Vec<(#(#types,)*)>),
                    seen: if declaration.value.is_some() {
                        syn::parse_quote!(::cq_storage::HashSet<(#(#types,)*)>)
                    } else {
                        syn::parse_quote!(::std::collections::HashSet<(#(#types,)*), ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>>)
                    },
                    delta: syn::parse_quote!(::std::vec::Vec<(#(#types,)*)>),
                    next: syn::parse_quote!(::std::vec::Vec<(#(#types,)*)>),
                    all_init: syn::parse_quote!(::std::vec::Vec::new()),
                    delta_init: syn::parse_quote!(::std::vec::Vec::new()),
                    next_init: syn::parse_quote!(::std::vec::Vec::new()),
                }
            })
            .collect(),
        layouts,
        delta_layouts,
        reads: Vec::new(),
        negations: Vec::new(),
        policy: Policy {
            insertion: syn::parse_quote!(all_columns),
            negation_keys: syn::parse_quote!(borrowed),
        },
    };
    (target.reads, target.negations) = cq_lang_datalog_storage::read_plan(&target, &analysis)?;
    check(source, &target)?;
    Ok(target)
}
/// Formal rule CK-DL-Storage: independent data-refinement correspondence.
/// Target admission checks its embedded Incremental source; equality admits the input too.
pub fn check(source: &incremental::Program, target: &Program) -> syn::Result<()> {
    cq_lang_datalog_storage::contract::check(target)?;
    ensure(
        source == &target.incremental,
        target,
        "storage changed its incremental source",
    )
}
fn choose_layouts(
    source: &incremental::Program,
    analysis: &incremental::Analysis,
) -> (Vec<Layout>, Vec<Layout>) {
    let mut layouts = Vec::new();
    let mut delta_layouts = Vec::new();
    let mut seen = BTreeSet::new();
    let mut ordered: Vec<_> = source.variants.iter().collect();
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
                let BodyItem::Positive { atom: base } =
                    &source.strata.source.rules[variant.rule].body[position]
                else {
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
                let declaration =
                    &source.strata.source.declarations[analysis.positions[&symbol_name(relation)]];
                let types: Vec<_> = keys
                    .iter()
                    .map(|&column| &declaration.columns[column].ty)
                    .collect();
                let (key_types, value_types) = types.split_at(key);
                selected.push(Layout {
                    relation: relation.clone(),
                    columns: keys,
                    key,
                    provider: syn::parse_quote!(
                        ::std::collections::HashMap<
                            (#(#key_types,)*),
                            ::std::vec::Vec<(#(#value_types,)*)>,
                            ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>
                        >
                    ),
                    maintenance: if is_delta {
                        syn::parse_quote!(replace_each_run)
                    } else {
                        syn::parse_quote!(admitted_rows)
                    },
                });
            }
            bound.extend(atom.variables().map(symbol_name));
        }
    }
    (layouts, delta_layouts)
}
