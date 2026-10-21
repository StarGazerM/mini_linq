//! Checked storage refinement of Boolean incremental Datalog.
//! Records concrete Rust providers, canonicalization wrappers, per-layout maintenance,
//! occurrence bindings, and negation-key projections. Applicable provider types and
//! initializers are opaque Rust syntax whose collection laws are premises. Unused
//! relation-kind fields retain checked defaults; delta and next share a move type.
//! See doc/formal/datalog.tex for the storage refinement obligations.
use cq_datalog_support::{columns, ensure, filters, key_columns};
pub use cq_lang_datalog_incremental as incremental;
use cq_syntax::symbol_name;
pub use incremental::strata::source::{BodyItem, Query};
use std::collections::BTreeSet;
use syn::{Expr, Ident, Type};
mod layouts;
pub mod text;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub incremental: incremental::Program,
    pub providers: Vec<Providers>,
    pub layouts: Vec<Layout>,
    pub delta_layouts: Vec<Layout>,
    pub reads: Vec<Read>,
    pub negations: Vec<Negative>,
    pub policy: Policy,
}
/// Concrete providers for one relation, indexed by source declaration order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Providers {
    pub canonical_name: Ident,
    /// Generic inner collection type, with `R` denoting the canonical row.
    pub canonical_set: Type,
    pub all: Type,
    pub seen: Type,
    pub delta: Type,
    pub next: Type,
    pub all_init: Expr,
    pub delta_init: Expr,
    pub next_init: Expr,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub relation: Ident,
    pub columns: Vec<usize>,
    pub key: usize,
    pub provider: Type,
    pub maintenance: Ident,
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
    pub insertion: Ident,
    pub negation_keys: Ident,
}
record!(Program { incremental: incremental::Program, providers: Vec<Providers>, layouts: Vec<Layout>, delta_layouts: Vec<Layout>, reads: Vec<Read>, negations: Vec<Negative>, policy: Policy });
record!(Providers {
    canonical_name: Ident,
    canonical_set: Type,
    all: Type,
    seen: Type,
    delta: Type,
    next: Type,
    all_init: Expr,
    delta_init: Expr,
    next_init: Expr
});
record!(Layout { relation: Ident, columns: Vec<usize>, key: usize, provider: Type, maintenance: Ident });
record!(Read { variant: usize, position: usize, kind: Ident, index: usize, columns: Vec<usize> });
record!(Negative { component: usize, relation: usize, columns: Vec<usize>, filters: Vec<(usize, Expr)> });
record!(Policy {
    insertion: Ident,
    negation_keys: Ident
});

pub use incremental::Analysis;
/// Choose the default written-order occurrence bindings; contracts do not call this.
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
        let order = incremental::variant_order(
            &program.incremental.strata.source.rules[variant.rule], variant.driver,
        )?;
        for (position, item) in variant.query.body.iter().enumerate() {
            match item {
                BodyItem::Positive { atom } => {
                    let source_position = order[position];
                    let is_delta = variant.delta == Some(source_position);
                    let relation = if is_delta {
                        let BodyItem::Positive { atom } =
                            &program.incremental.strata.source.rules[variant.rule].body[source_position]
                        else {
                            unreachable!("checked changing occurrence")
                        };
                        &atom.relation
                    } else {
                        &atom.relation
                    };
                    let (columns, key) = key_columns(atom, &bound);
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
                        columns: columns(atom),
                        filters: filters(atom),
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
    /// Formal rule WF-DL-Storage: admissible bindings, layouts and total maintenance.
    pub fn check(program: &Program) -> syn::Result<Analysis> {
        let analysis = incremental::contract::check(&program.incremental)?;
        ensure(
            program.providers.len() == program.incremental.strata.source.declarations.len(),
            program,
            "storage providers must cover every declaration in declaration order",
        )?;
        for (provider, declaration) in program
            .providers
            .iter()
            .zip(&program.incremental.strata.source.declarations)
        {
            ensure(
                provider.delta == provider.next,
                provider,
                "delta and next providers must have the same type for move publication",
            )?;
            if declaration.value.is_some() {
                let types: Vec<_> = declaration
                    .columns
                    .iter()
                    .map(|column| &column.ty)
                    .collect();
                let rows: Type = syn::parse_quote!(::std::vec::Vec<(#(#types,)*)>);
                let seen: Type = syn::parse_quote!(::cq_storage::HashSet<(#(#types,)*)>);
                let empty: Expr = syn::parse_quote!(::std::vec::Vec::new());
                ensure(
                    provider.all == rows
                        && provider.seen == seen
                        && provider.delta == rows
                        && provider.next == rows
                        && provider.all_init == empty
                        && provider.delta_init == empty
                        && provider.next_init == empty,
                    provider,
                    "EDB providers must retain defaults for unused IDB storage fields",
                )?;
            } else {
                ensure(
                    provider.canonical_name == "Canonical"
                        && provider.canonical_set == syn::parse_quote!(::cq_storage::HashSet<R>),
                    provider,
                    "IDB providers must retain defaults for unused EDB canonicalization fields",
                )?;
            }
        }
        layouts::check(program, &analysis)?;
        let p = &program.policy;
        ensure(
            (p.insertion == "all_columns" && p.negation_keys == "borrowed")
                || (p.insertion == "device_set_all_columns" && p.negation_keys == "device_set_owned"),
            program,
            "unsupported storage insertion or negation semantics",
        )?;
        Ok(analysis)
    }
}
pub fn check_rewrite(source: &Program, target: &Program) -> syn::Result<()> {
    contract::check(source)?;
    contract::check(target)?;
    check_rewrite_admitted(source, target)
}
// The stage runner has already checked both language contracts.
fn check_rewrite_admitted(source: &Program, target: &Program) -> syn::Result<()> {
    ensure(
        source.incremental == target.incremental && source.policy == target.policy,
        target,
        "storage rewrite changed its incremental base program",
    )
}
/// The rewrite callback assumes `cq_stage::run` checked both contracts.
/// Standalone callers must use the public free [`check_rewrite`] function.
impl cq_stage::Language for Program {
    fn check(&self) -> syn::Result<()> {
        contract::check(self).map(|_| ())
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        check_rewrite_admitted(source, target)
    }
}
