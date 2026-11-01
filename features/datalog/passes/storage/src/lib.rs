//! Choose stable storage layouts in the independently checked occurrence order.
//! Each relation's canonical layout is the first full layout keyed by every
//! column (a read binding all columns); otherwise the admission anti-join read
//! adds the identity all-column-keyed layout. It is an ordinary layout. Every
//! layout keyed by all columns is a payload-free hash set; the others are
//! grouped hash maps.
use cq_datalog_support::{ensure, key_columns};
use cq_lang_datalog_incremental as incremental;
use cq_lang_datalog_storage::{
    BodyItem, Hasher, HostCanonical, Layout, Policy, Program, Provider, Providers, provider,
};
use cq_syntax::symbol_name;
use std::collections::BTreeSet;

pub fn compile(source: &incremental::Program) -> syn::Result<Program> {
    let analysis = incremental::contract::check(source)?;
    let (layouts, delta_layouts, canonical) = choose_layouts(source, &analysis);
    let mut target = Program {
        incremental: source.clone(),
        device: None,
        // Base tokens; `hash_set` also types Round's negation key sets.
        tokens: vec![
            provider::hash_set(),
            provider::hash_map(),
            provider::rows_vec(),
        ],
        providers: source
            .strata
            .source
            .declarations
            .iter()
            .zip(canonical)
            .map(|(declaration, canonical)| {
                let rows = rows(declaration);
                Providers {
                    canonical,
                    delta: rows.clone(),
                    next: rows,
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
        policy: Policy {
            insertion: syn::parse_quote!(all_columns),
            negation_keys: syn::parse_quote!(borrowed),
        },
    };
    (target.reads, target.negations) = cq_lang_datalog_storage::read_plan(&target, &analysis)?;
    for (index, declaration) in source.strata.source.declarations.iter().enumerate() {
        target.certificates.push(HostCanonical {
            kind: syn::parse_quote!(canonical),
            index,
            source: syn::parse_quote!(admission),
            columns: target.layouts[target.providers[index].canonical]
                .columns
                .clone(),
        });
        if analysis
            .deltas
            .contains_key(&symbol_name(&declaration.name))
        {
            target.certificates.push(HostCanonical {
                kind: syn::parse_quote!(delta),
                index,
                source: syn::parse_quote!(admission),
                columns: (0..declaration.columns.len()).collect(),
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
                    syn::parse_quote!(canonical)
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
/// The `rows_vec` buffer of a relation's full rows.
fn rows(declaration: &cq_syntax::RelationDecl) -> Provider {
    let token = provider::rows_vec();
    let row = cq_lang_datalog_storage::row_type(declaration);
    let ty = provider::spell(&token, &row, &syn::parse_quote!(()), Hasher::None)
        .expect("rows_vec spells unhashed rows");
    Provider {
        token: token.token,
        ty,
    }
}
/// Default providers (DL-LayoutProtocol): a layout keyed by every column has no
/// payload, so it is a `hash_set`; other layouts are `hash_map`s grouping their
/// payload rows by key. Both record the default Fx hasher.
fn layout(
    declaration: &cq_syntax::RelationDecl,
    columns: Vec<usize>,
    key: usize,
    delta: bool,
) -> Layout {
    let token = if key == columns.len() {
        provider::hash_set()
    } else {
        provider::hash_map()
    };
    let (key_row, payload) = cq_lang_datalog_storage::layout_schema(declaration, &columns, key);
    let ty = provider::spell(&token, &key_row, &payload, Hasher::Fx)
        .expect("base tokens spell the Fx hasher");
    Layout {
        relation: declaration.name.clone(),
        columns,
        key,
        protocol: token
            .protocol
            .expect("base layout tokens declare a protocol"),
        provider: Provider {
            token: token.token,
            ty,
        },
        hasher: Hasher::Fx,
        maintenance: if delta {
            syn::parse_quote!(replace_each_run)
        } else {
            syn::parse_quote!(admitted_rows)
        },
    }
}
/// Formal rule DL-Canonical: read layouts first, then one canonical member per relation.
fn choose_layouts(
    source: &incremental::Program,
    analysis: &incremental::Analysis,
) -> (Vec<Layout>, Vec<Layout>, Vec<usize>) {
    let mut layouts = Vec::new();
    let mut delta_layouts = Vec::new();
    let mut seen = BTreeSet::new();
    let mut ordered: Vec<_> = source.variants.iter().collect();
    ordered.sort_by_key(|variant| (variant.rule, variant.delta));
    for variant in ordered {
        let mut bound = BTreeSet::new();
        let order =
            incremental::variant_order(&source.strata.source.rules[variant.rule], &variant.order)
                .expect("checked occurrence order");
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
                let mut layout = layout(declaration, keys, key, is_delta);
                layout.relation = relation.clone();
                selected.push(layout);
            }
            bound.extend(atom.variables().map(symbol_name));
        }
    }
    // The admission anti-join (and EDB loading) binds every column: reuse a
    // full layout already keyed by all columns, else complete it in identity order.
    let canonical = source
        .strata
        .source
        .declarations
        .iter()
        .map(|declaration| {
            let name = symbol_name(&declaration.name);
            layouts
                .iter()
                .position(|layout| {
                    symbol_name(&layout.relation) == name && layout.key == declaration.arity()
                })
                .unwrap_or_else(|| {
                    let arity = declaration.arity();
                    layouts.push(layout(declaration, (0..arity).collect(), arity, false));
                    layouts.len() - 1
                })
        })
        .collect();
    (layouts, delta_layouts, canonical)
}
