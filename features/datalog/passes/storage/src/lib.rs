//! Choose stable storage layouts in the independently checked variant-driver order.
use cq_datalog_support::{ensure, key_columns};
use cq_lang_datalog_incremental as incremental;
use cq_lang_datalog_storage::{BodyItem, HostCanonical, Layout, Policy, Program, Providers};
use cq_syntax::symbol_name;
use std::collections::BTreeSet;

/// Admission and negative-key representation, independent of incremental coverage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Representation {
    #[default]
    CpuHashSet,
    DeviceSet,
}
pub fn compile(source: &incremental::Program) -> syn::Result<Program> {
    compile_with_representation(source, Representation::CpuHashSet)
}

pub fn compile_with_representation(
    source: &incremental::Program,
    representation: Representation,
) -> syn::Result<Program> {
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
        certificates: Vec::new(),
        policy: match representation {
            Representation::CpuHashSet => Policy {
                insertion: syn::parse_quote!(all_columns),
                negation_keys: syn::parse_quote!(borrowed),
            },
            Representation::DeviceSet => Policy {
                insertion: syn::parse_quote!(device_set_all_columns),
                negation_keys: syn::parse_quote!(device_set_owned),
            },
        },
    };
    (target.reads, target.negations) = cq_lang_datalog_storage::read_plan(&target, &analysis)?;
    for (index, declaration) in source.strata.source.declarations.iter().enumerate() {
        let columns: Vec<_> = (0..declaration.columns.len()).collect();
        target.certificates.push(HostCanonical {
            kind: syn::parse_quote!(all),
            index,
            source: if declaration.value.is_some() {
                syn::parse_quote!(provider)
            } else {
                syn::parse_quote!(admission)
            },
            columns: columns.clone(),
        });
        if analysis
            .deltas
            .contains_key(&symbol_name(&declaration.name))
        {
            target.certificates.push(HostCanonical {
                kind: syn::parse_quote!(delta),
                index,
                source: syn::parse_quote!(admission),
                columns,
            });
        }
    }
    for (delta, layouts) in [(false, &target.layouts), (true, &target.delta_layouts)] {
        for (index, layout) in layouts.iter().enumerate() {
            target.certificates.push(HostCanonical {
                kind: if delta {
                    syn::parse_quote!(delta_layout)
                } else {
                    syn::parse_quote!(layout)
                },
                index,
                source: if delta {
                    syn::parse_quote!(delta)
                } else {
                    syn::parse_quote!(all)
                },
                columns: layout.columns.clone(),
            });
        }
    }
    for (index, negative) in target.negations.iter().enumerate() {
        target.certificates.push(HostCanonical {
            kind: syn::parse_quote!(negation),
            index,
            source: syn::parse_quote!(projection),
            columns: negative.columns.clone(),
        });
    }
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
        let order =
            incremental::variant_order(&source.strata.source.rules[variant.rule], variant.driver)
                .expect("checked driver order");
        for (position, item) in variant.query.body.iter().enumerate() {
            let BodyItem::Positive { atom } = item else {
                continue;
            };
            let (keys, key) = key_columns(atom, &bound);
            let source_position = order[position];
            let is_delta = Some(source_position) == variant.delta;
            let relation = if is_delta {
                let BodyItem::Positive { atom: base } =
                    &source.strata.source.rules[variant.rule].body[source_position]
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
