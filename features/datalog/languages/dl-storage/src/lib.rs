//! Checked storage refinement of Boolean incremental Datalog.
//! Text records layouts, every positive read, frozen negative projections, and
//! provider/admission/maintenance choices. Erasure retains the incremental program.
pub use cq_lang_datalog_incremental as incremental;
use cq_syntax::{rust, symbol_name};
pub use incremental::strata::source::{BodyItem, Query};
use quote::ToTokens;
use std::collections::BTreeSet;
use syn::{Expr, Ident};
mod layouts;
pub mod text;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub incremental: incremental::Program,
    pub layouts: Vec<Layout>,
    pub delta_layouts: Vec<Layout>,
    pub reads: Vec<Read>,
    pub negations: Vec<Negative>,
    pub policy: Policy,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub relation: Ident,
    pub columns: Vec<usize>,
    pub key: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    pub variant: usize,
    pub position: usize,
    pub kind: Ident,
    pub index: usize,
    pub columns: Vec<usize>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Negative {
    pub component: usize,
    pub relation: usize,
    pub columns: Vec<usize>,
    pub filters: Vec<(usize, Expr)>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    pub edb: Ident,
    pub all: Ident,
    pub seen: Ident,
    pub delta: Ident,
    pub next: Ident,
    pub layout: Ident,
    pub insertion: Ident,
    pub negation_keys: Ident,
    pub full_maintenance: Ident,
    pub delta_maintenance: Ident,
}
record!(Program { incremental: incremental::Program, layouts: Vec<Layout>, delta_layouts: Vec<Layout>, reads: Vec<Read>, negations: Vec<Negative>, policy: Policy });
record!(Layout { relation: Ident, columns: Vec<usize>, key: usize });
record!(Read { variant: usize, position: usize, kind: Ident, index: usize, columns: Vec<usize> });
record!(Negative { component: usize, relation: usize, columns: Vec<usize>, filters: Vec<(usize, Expr)> });
record!(Policy {
    edb: Ident,
    all: Ident,
    seen: Ident,
    delta: Ident,
    next: Ident,
    layout: Ident,
    insertion: Ident,
    negation_keys: Ident,
    full_maintenance: Ident,
    delta_maintenance: Ident
});

pub use incremental::Analysis;
pub fn ensure(ok: bool, at: impl ToTokens, message: &str) -> syn::Result<()> {
    if ok {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(at, message))
    }
}
/// Infer occurrence obligations from checked variants, without choosing layouts.
pub fn read_plan(
    program: &Program,
    analysis: &Analysis,
) -> syn::Result<(Vec<Read>, Vec<Negative>)> {
    let mut reads = Vec::new();
    let mut negations = Vec::new();
    for (variant_index, variant) in program.incremental.variants.iter().enumerate() {
        let mut bound = BTreeSet::new();
        let head = analysis.positions[&symbol_name(&variant.query.head.relation)];
        let component = analysis.component[head].expect("checked IDB component");
        for (position, item) in variant.query.body.iter().enumerate() {
            match item {
                BodyItem::Positive { atom } => {
                    let is_delta = variant.delta == Some(position);
                    let relation = if is_delta {
                        let BodyItem::Positive { atom } =
                            &program.incremental.strata.source.rules[variant.rule].body[position]
                        else {
                            unreachable!("checked changing occurrence")
                        };
                        &atom.relation
                    } else {
                        &atom.relation
                    };
                    let mut columns: Vec<_> = atom
                        .args
                        .iter()
                        .enumerate()
                        .filter_map(|(i, arg)| {
                            match rust::variable(arg) {
                                Some(id) => bound.contains(&symbol_name(id)),
                                None => !matches!(rust::unparen(arg), Expr::Infer(_)),
                            }
                            .then_some(i)
                        })
                        .collect();
                    let key = columns.len();
                    let payload: Vec<_> = (0..atom.args.len())
                        .filter(|i| !columns.contains(i))
                        .collect();
                    columns.extend(payload);
                    let (kind, index) = if is_delta && position == 0 && key == 0 {
                        (
                            syn::parse_quote!(direct),
                            analysis.positions[&symbol_name(relation)],
                        )
                    } else {
                        let choices = if is_delta {
                            &program.delta_layouts
                        } else {
                            &program.layouts
                        };
                        let index = choices
                            .iter()
                            .position(|l| {
                                l.relation == *relation && l.columns == columns && l.key == key
                            })
                            .ok_or_else(|| {
                                syn::Error::new_spanned(atom, "storage omitted a read layout")
                            })?;
                        (
                            if is_delta {
                                syn::parse_quote!(delta)
                            } else {
                                syn::parse_quote!(full)
                            },
                            index,
                        )
                    };
                    reads.push(Read {
                        variant: variant_index,
                        position,
                        kind,
                        index,
                        columns,
                    });
                    bound.extend(atom.variables().map(symbol_name));
                }
                BodyItem::Negation(negative) => {
                    let atom = &negative.atom;
                    let entry = Negative {
                        component,
                        relation: analysis.positions[&symbol_name(&atom.relation)],
                        columns: atom
                            .args
                            .iter()
                            .enumerate()
                            .filter_map(|(i, arg)| rust::variable(arg).map(|_| i))
                            .collect(),
                        filters: atom
                            .args
                            .iter()
                            .enumerate()
                            .filter(|(_, arg)| {
                                rust::variable(arg).is_none()
                                    && !matches!(rust::unparen(arg), Expr::Infer(_))
                            })
                            .map(|(i, arg)| (i, arg.clone()))
                            .collect(),
                    };
                    if !negations.contains(&entry) {
                        negations.push(entry);
                    }
                }
                _ => {}
            }
        }
    }
    Ok((reads, negations))
}
pub mod contract {
    use super::*;
    /// Formal rule WF-DL-Storage: exact layouts, reads, providers and maintenance.
    pub fn check(program: &Program) -> syn::Result<Analysis> {
        let analysis = incremental::contract::check(&program.incremental)?;
        layouts::check(program, &analysis)?;
        let p = &program.policy;
        ensure(
            p.edb == "canonical_borrowed_hash_set"
                && p.all == "vec"
                && p.seen == "hash_set"
                && p.delta == "vec"
                && p.next == "vec"
                && p.layout == "hash_map_vec_hash_state"
                && p.insertion == "hash_set_all_columns"
                && p.negation_keys == "hash_set_borrowed"
                && p.full_maintenance == "admitted_rows"
                && p.delta_maintenance == "replace_each_run",
            program,
            "unsupported storage provider or maintenance policy",
        )?;
        let (reads, negations) = read_plan(program, &analysis)?;
        ensure(
            program.reads == reads && program.negations == negations,
            program,
            "storage read or negative projection coverage changed",
        )?;
        Ok(analysis)
    }
}
pub fn check_rewrite(source: &Program, target: &Program) -> syn::Result<()> {
    contract::check(source)?;
    contract::check(target)?;
    ensure(
        source == target,
        target,
        "storage rewrites must preserve all recorded choices",
    )
}
impl cq_stage::Language for Program {
    fn check(&self) -> syn::Result<()> {
        contract::check(self).map(|_| ())
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        check_rewrite(source, target)
    }
}
