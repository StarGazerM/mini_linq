//! Execution facts needed by printers. Owned here so a Structures consumer
//! never needs the producer's Physical or logical syntax crates.
use crate::{CommaList, ModeKind, PhysicalFacts, syntax::ColumnDecl};
use std::collections::{BTreeMap, BTreeSet};
use syn::{Expr, Ident};

#[derive(Clone, Debug)]
pub struct Definition {
    pub name: Ident,
    pub mode: ModeKind,
    pub canon: bool,
    pub operator: Operator,
}
#[derive(Clone, Debug)]
pub enum Operator {
    Unit,
    Rename,
    Select,
    Extend,
    Project,
    Range(Ident),
    Probe(Keyed),
    Semi(Keyed),
    Merge(Keyed),
}
#[derive(Clone, Debug)]
pub struct Keyed {
    pub columns: CommaList<Ident>,
    pub left: Ident,
    pub right: Ident,
}
#[derive(Clone, Debug)]
pub struct Select {
    pub input: Ident,
    pub condition: Expr,
}
#[derive(Clone, Debug)]
pub struct Extend {
    pub input: Ident,
    pub columns: Vec<ColumnDecl>,
    pub value: Expr,
}
#[derive(Clone, Debug, Default)]
pub struct Row {
    pub bindings: BTreeMap<String, Ident>,
    pub computed: BTreeSet<String>,
    pub select: Option<Select>,
    pub extend: Option<Extend>,
    pub project: Option<Ident>,
}

/// Recognize the execution shape used by the optional intersection printer.
pub fn binary_intersection<'a>(
    first: &'a Definition,
    second: &'a Definition,
    analysis: &PhysicalFacts,
) -> Option<(&'a Keyed, &'a Keyed, String)> {
    use crate::symbol_name;
    let Operator::Probe(a) = &first.operator else {
        return None;
    };
    let (Operator::Probe(b) | Operator::Semi(b)) = &second.operator else {
        return None;
    };
    if first.mode != ModeKind::Pipe || first.canon || b.left != first.name {
        return None;
    }
    let left = &analysis.headings[&symbol_name(&a.left)];
    let ah = &analysis.headings[&symbol_name(&a.right)];
    let bh = &analysis.headings[&symbol_name(&b.right)];
    let added: Vec<_> = ah.iter().filter(|c| !left.contains(c)).collect();
    if added.len() != 1 {
        return None;
    }
    let v = added[0];
    let ak: Vec<_> = a.columns.iter().map(symbol_name).collect();
    let bk: Vec<_> = b.columns.iter().map(symbol_name).collect();
    if ak.is_empty()
        || bk.len() < 2
        || !bk.contains(v)
        || ah.len() != ak.len() + 1
        || bh.len() != bk.len()
        || !bh.iter().all(|c| left.contains(c) || c == v)
        || [&a.right, &b.right]
            .iter()
            .any(|r| analysis.modes[&symbol_name(r)] != ModeKind::Let)
    {
        return None;
    }
    Some((a, b, v.clone()))
}
