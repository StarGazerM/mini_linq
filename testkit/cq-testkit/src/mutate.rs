//! C1 mutation operators over recorded Physical and Structures facts.
//!
//! Each mutant changes one recorded fact of a compiled program: a key's
//! columns or their order, a carrier width, a constraint, filter or capture,
//! an interval bound, a provider/operand choice, a recorded flag, or the
//! definition order. A checker must reject every mutant, or the mutant must
//! still denote the reference answer (judged by `physical_semantics` for
//! Physical, by an executable route otherwise). Mutants are deterministic,
//! pairwise distinct and never equal to their source. Carrier mutations use
//! typed fixture/model facts, never Rust type syntax; no pass is called.

use std::collections::{BTreeMap, BTreeSet};

use quote::ToTokens;
use syn::parse_quote;

use crate::{physical, structures, values::ColumnType};
use cq_syntax::constraint::{Comparison, Constraint, Term};

/// The C1 operator family a mutant belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Operator {
    /// Permute a recorded key's (or rename's) columns.
    PermuteKey,
    /// Drop one column of a recorded key.
    DropKeyColumn,
    /// Narrow a declared or annotated carrier width.
    NarrowWidth,
    /// Drop a constraint, filter, constant, guard or capture gate.
    DropConstraint,
    /// Negate a constraint or guard, or turn a semijoin into an anti-join.
    FlipConstraint,
    /// Widen or narrow an interval bound (strictness or literal by one).
    ShiftBound,
    /// Swap a provider/operand or change a recorded execution choice.
    SwapProvider,
    /// Drop or toggle a recorded flag (canon, mode, host set, membership).
    DropFlag,
    /// Swap two adjacent definitions or entries.
    ReorderDefinitions,
}

/// One mutated program and the site of its single change.
#[derive(Clone, Debug)]
pub struct Mutant<P> {
    pub operator: Operator,
    pub site: String,
    pub program: P,
}

/// Typed scalar carriers supplied by the fixture or fuzz model, not recovered
/// from Rust type syntax. Names are canonical (without an `r#` prefix).
#[derive(Clone, Debug, Default)]
pub struct CarrierTypes {
    /// Relation name and positional column ordinal.
    pub columns: BTreeMap<(String, usize), ColumnType>,
    /// Scalar binding name.
    pub bindings: BTreeMap<String, ColumnType>,
}

impl CarrierTypes {
    /// Read the generated schema and scalar bindings directly from the model.
    /// Optional aggregate outputs are not scalar narrowing sites.
    pub fn from_case(case: &crate::fuzz::Case) -> Self {
        let mut types = Self::default();
        for relation in &case.query.schema {
            for (column, ty) in relation.types.iter().copied().enumerate() {
                types.columns.insert(
                    (relation.name.trim_start_matches("r#").to_owned(), column),
                    ty,
                );
            }
        }
        for (column, ty) in case.output_types().into_iter().enumerate() {
            if !ty.optional {
                types.columns.insert(("answer".into(), column), ty.column);
            }
        }
        for binding in &case.query.bindings {
            if binding.generator.is_none() {
                types.bindings.insert(
                    case.query
                        .name(binding.variable)
                        .trim_start_matches("r#")
                        .to_owned(),
                    ColumnType::I32,
                );
            }
        }
        types
    }

    /// Record the caller's assertion that every carrier in this authored
    /// fixture is i32. This does not inspect or infer Rust types.
    pub fn all_i32(program: &physical::Program) -> Self {
        let mut types = Self::default();
        for declaration in &program.program.declarations {
            let relation = cq_syntax::symbol_name(&declaration.name);
            for column in 0..declaration.columns.len() {
                types
                    .columns
                    .insert((relation.clone(), column), ColumnType::I32);
            }
        }
        for definition in &program.program.definitions {
            if let physical::Operator::Bind(binding) = &definition.operator {
                if let syn::Pat::Ident(pattern) = &binding.pattern {
                    types
                        .bindings
                        .insert(cq_syntax::symbol_name(&pattern.ident), ColumnType::I32);
                }
            }
        }
        types
    }
}

struct Collector<P> {
    source: String,
    seen: BTreeSet<String>,
    out: Vec<Mutant<P>>,
}

impl<P: ToTokens> Collector<P> {
    fn new(source: &P) -> Self {
        let source = source.to_token_stream().to_string();
        Self {
            seen: BTreeSet::from([source.clone()]),
            source,
            out: Vec::new(),
        }
    }

    fn push(&mut self, operator: Operator, site: impl Into<String>, program: P) {
        let text = program.to_token_stream().to_string();
        if text != self.source && self.seen.insert(text) {
            self.out.push(Mutant {
                operator,
                site: site.into(),
                program,
            });
        }
    }
}

fn narrower(ty: ColumnType) -> Option<syn::Type> {
    Some(match ty {
        ColumnType::I64 | ColumnType::Usize => parse_quote!(u32),
        ColumnType::I32 => parse_quote!(i16),
        ColumnType::U32 => parse_quote!(u16),
        ColumnType::Str => return None,
    })
}

fn negate(c: Comparison) -> Comparison {
    match c {
        Comparison::Eq(_) => parse_quote!(!=),
        Comparison::Ne(_) => parse_quote!(=),
        Comparison::Lt(_) => parse_quote!(>=),
        Comparison::Le(_) => parse_quote!(>),
        Comparison::Gt(_) => parse_quote!(<=),
        Comparison::Ge(_) => parse_quote!(<),
    }
}

fn strictness(c: Comparison) -> Option<Comparison> {
    Some(match c {
        Comparison::Lt(_) => parse_quote!(<=),
        Comparison::Le(_) => parse_quote!(<),
        Comparison::Gt(_) => parse_quote!(>=),
        Comparison::Ge(_) => parse_quote!(>),
        _ => return None,
    })
}

fn bump(term: &Term) -> Option<Term> {
    match term {
        Term::Literal(l) => Some(Term::Literal(Box::new(parse_quote!((#l) + 1)))),
        Term::Closed(e) => Some(Term::Closed(parse_quote!((#e) + 1))),
        Term::Variable(_) => None,
    }
}

/// One-step variants of a constraint (operator, description, replacement).
fn constraint_variants(c: &Constraint) -> Vec<(Operator, &'static str, Constraint)> {
    let mut out = Vec::new();
    match c {
        Constraint::Compare {
            left,
            comparison,
            right,
        } => {
            out.push((
                Operator::FlipConstraint,
                "negate comparison",
                Constraint::Compare {
                    left: left.clone(),
                    comparison: negate(*comparison),
                    right: right.clone(),
                },
            ));
            if let Some(comparison) = strictness(*comparison) {
                out.push((
                    Operator::ShiftBound,
                    "toggle strictness",
                    Constraint::Compare {
                        left: left.clone(),
                        comparison,
                        right: right.clone(),
                    },
                ));
            }
            if let Some(right) = bump(right) {
                out.push((
                    Operator::ShiftBound,
                    "bound + 1",
                    Constraint::Compare {
                        left: left.clone(),
                        comparison: *comparison,
                        right,
                    },
                ));
            }
        }
        Constraint::Between { term, low, high } => {
            if let Some(low) = bump(low) {
                out.push((
                    Operator::ShiftBound,
                    "low + 1",
                    Constraint::Between {
                        term: term.clone(),
                        low,
                        high: high.clone(),
                    },
                ));
            }
            if let Some(high) = bump(high) {
                out.push((
                    Operator::ShiftBound,
                    "high + 1",
                    Constraint::Between {
                        term: term.clone(),
                        low: low.clone(),
                        high,
                    },
                ));
            }
        }
        Constraint::In { term, literals } if literals.len() > 1 => {
            out.push((
                Operator::DropConstraint,
                "drop a member",
                Constraint::In {
                    term: term.clone(),
                    literals: literals.iter().skip(1).cloned().collect(),
                },
            ));
        }
        Constraint::In { .. } => {}
        Constraint::Or(branches) => {
            if branches.len() > 1 {
                out.push((
                    Operator::DropConstraint,
                    "drop a disjunct",
                    Constraint::Or(branches[1..].to_vec()),
                ));
            }
            for (b, branch) in branches.iter().enumerate() {
                for (i, inner) in branch.iter().enumerate() {
                    for (operator, what, replacement) in constraint_variants(inner) {
                        let mut branches = branches.clone();
                        branches[b][i] = replacement;
                        out.push((operator, what, Constraint::Or(branches)));
                    }
                }
            }
        }
    }
    out
}

fn theta_variants(
    theta: &cq_syntax::algebra::Theta,
) -> Vec<(Operator, String, cq_syntax::algebra::Theta)> {
    let mut out = Vec::new();
    for i in 0..theta.equalities.len() {
        let mut t = theta.clone();
        t.equalities = theta
            .equalities
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, e)| e.clone())
            .collect();
        out.push((Operator::DropConstraint, format!("drop equality {i}"), t));
        if let cq_syntax::EqualityValue::Literal(l) = &theta.equalities[i].right {
            let mut t = theta.clone();
            t.equalities[i].right = cq_syntax::EqualityValue::Literal(parse_quote!((#l) + 1));
            out.push((Operator::ShiftBound, format!("equality {i} + 1"), t));
        }
    }
    for i in 0..theta.constraints.len() {
        let mut t = theta.clone();
        t.constraints = theta
            .constraints
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, c)| c.clone())
            .collect();
        out.push((Operator::DropConstraint, format!("drop constraint {i}"), t));
        for (operator, what, replacement) in constraint_variants(&theta.constraints[i]) {
            let mut t = theta.clone();
            t.constraints[i] = replacement;
            out.push((operator, format!("constraint {i}: {what}"), t));
        }
    }
    out
}

fn without<T: Clone, P: Default>(
    list: &syn::punctuated::Punctuated<T, P>,
    index: usize,
) -> syn::punctuated::Punctuated<T, P> {
    list.iter()
        .enumerate()
        .filter(|(j, _)| *j != index)
        .map(|(_, x)| x.clone())
        .collect()
}

fn keyed_variants(k: &physical::Keyed) -> Vec<(Operator, String, physical::Keyed)> {
    use physical::Choice;
    let mut out = Vec::new();
    if k.columns.len() > 1 {
        let mut m = k.clone();
        m.columns = k.columns.iter().rev().cloned().collect();
        out.push((Operator::PermuteKey, "reverse key".into(), m));
    }
    for i in 0..k.columns.len() {
        let mut m = k.clone();
        m.columns = without(&k.columns, i);
        out.push((Operator::DropKeyColumn, format!("drop key column {i}"), m));
    }
    let mut m = k.clone();
    std::mem::swap(&mut m.left, &mut m.right);
    out.push((Operator::SwapProvider, "swap operands".into(), m));
    let choice: Option<Choice> = match &k.choice {
        Some(Choice::First(_)) => Some(parse_quote!(smallest)),
        Some(Choice::Smallest(_)) => Some(parse_quote!(first)),
        Some(Choice::Eager(_)) => Some(parse_quote!(lazy)),
        Some(Choice::Lazy(_)) => Some(parse_quote!(eager)),
        None => None,
    };
    if let Some(choice) = choice {
        let mut m = k.clone();
        m.choice = Some(choice);
        out.push((Operator::SwapProvider, "change choice".into(), m));
    }
    out
}

/// Every one-fact mutant of a Physical program. Width mutations require a
/// corresponding typed carrier fact; unrecorded carriers are left alone.
pub fn physical(
    program: &physical::Program,
    types: &CarrierTypes,
) -> Vec<Mutant<physical::Program>> {
    use physical::{Mode, Operator as Op};
    let mut c = Collector::new(program);
    let body = &program.program;
    for (d, declaration) in body.declarations.iter().enumerate() {
        for (i, column) in declaration.columns.iter().enumerate() {
            if let Some(ty) = types
                .columns
                .get(&(cq_syntax::symbol_name(&declaration.name), i))
                .copied()
                .and_then(narrower)
            {
                let mut m = program.clone();
                m.program.declarations[d].columns[i].ty = ty;
                c.push(
                    Operator::NarrowWidth,
                    format!("{}.{}", declaration.name, column.name),
                    m,
                );
            }
        }
    }
    for (i, item) in body.captures.items.iter().enumerate() {
        if let cq_syntax::capture::Capture::Guard(_) = item {
            let mut m = program.clone();
            m.program.captures.items.remove(i);
            c.push(Operator::DropConstraint, format!("capture gate {i}"), m);
        }
    }
    if body.consumer_canonical.is_some() {
        let mut m = program.clone();
        m.program.consumer_canonical = None;
        c.push(Operator::DropFlag, "consumer canonical", m);
    }
    if body.return_canon.is_some() {
        let mut m = program.clone();
        m.program.return_canon = None;
        c.push(Operator::DropFlag, "return canon", m);
    }
    for (d, definition) in body.definitions.iter().enumerate() {
        let site = |what: &str| format!("{}: {what}", definition.name);
        let mut with = |operator: Operator, what: &str, op: Op| {
            let mut m = program.clone();
            m.program.definitions[d].operator = op;
            c.push(operator, site(what), m);
        };
        match &definition.operator {
            Op::Probe(t, k) => {
                for (operator, what, k) in keyed_variants(k) {
                    with(operator, &what, Op::Probe(*t, k));
                }
                if !k.columns.is_empty() {
                    let mut k = k.clone();
                    k.choice = None;
                    with(
                        Operator::SwapProvider,
                        "probe as merge",
                        Op::Merge(Default::default(), k),
                    );
                }
            }
            Op::Merge(t, k) => {
                for (operator, what, k) in keyed_variants(k) {
                    with(operator, &what, Op::Merge(*t, k));
                }
                let mut k = k.clone();
                k.choice = Some(parse_quote!(first));
                with(
                    Operator::SwapProvider,
                    "merge as probe",
                    Op::Probe(Default::default(), k),
                );
            }
            Op::Semi(t, k) => {
                for (operator, what, k) in keyed_variants(k) {
                    with(operator, &what, Op::Semi(*t, k));
                }
                with(
                    Operator::FlipConstraint,
                    "semi as anti",
                    Op::Anti(Default::default(), k.clone()),
                );
            }
            Op::Anti(t, k) => {
                for (operator, what, k) in keyed_variants(k) {
                    with(operator, &what, Op::Anti(*t, k));
                }
                with(
                    Operator::FlipConstraint,
                    "anti as semi",
                    Op::Semi(Default::default(), k.clone()),
                );
            }
            Op::Select(s) | Op::Point(_, s) | Op::Range(_, s) => {
                for (operator, what, theta) in theta_variants(&s.condition) {
                    let mut s = s.clone();
                    s.condition = theta;
                    let op = match &definition.operator {
                        Op::Point(t, _) => Op::Point(*t, s),
                        Op::Range(t, _) => Op::Range(*t, s),
                        _ => Op::Select(s),
                    };
                    with(operator, &what, op);
                }
            }
            Op::Rename(r) => {
                if r.mapping.len() > 1 {
                    let mut m = r.clone();
                    let first = m.mapping[0].target.clone();
                    let last = m.mapping.len() - 1;
                    m.mapping[0].target = m.mapping[last].target.clone();
                    m.mapping[last].target = first;
                    with(Operator::PermuteKey, "swap rename targets", Op::Rename(m));
                }
                if let Some(constants) = &r.constants {
                    for i in 0..constants.equalities.len() {
                        let mut m = r.clone();
                        let kept = without(&constants.equalities, i);
                        if kept.is_empty() {
                            m.constants = None;
                        } else {
                            m.constants.as_mut().unwrap().equalities = kept;
                        }
                        with(
                            Operator::DropConstraint,
                            &format!("drop constant {i}"),
                            Op::Rename(m),
                        );
                        if let cq_syntax::EqualityValue::Literal(l) = &constants.equalities[i].right
                        {
                            let mut m = r.clone();
                            m.constants.as_mut().unwrap().equalities[i].right =
                                cq_syntax::EqualityValue::Literal(parse_quote!((#l) + 1));
                            with(
                                Operator::ShiftBound,
                                &format!("constant {i} + 1"),
                                Op::Rename(m),
                            );
                        }
                    }
                }
            }
            Op::Guard(g) => {
                let mut m = g.clone();
                m.capture = None;
                m.condition = parse_quote!(true);
                with(Operator::DropConstraint, "drop guard", Op::Guard(m));
                let mut m = g.clone();
                let condition = &g.condition;
                m.condition = parse_quote!(!(#condition));
                with(Operator::FlipConstraint, "negate guard", Op::Guard(m));
            }
            Op::Bind(b) => {
                let ty = match &b.pattern {
                    syn::Pat::Ident(pattern) if b.annotation.is_some() => types
                        .bindings
                        .get(&cq_syntax::symbol_name(&pattern.ident))
                        .copied()
                        .and_then(narrower),
                    _ => None,
                };
                if let Some(ty) = ty {
                    let mut m = b.clone();
                    m.annotation.as_mut().unwrap().ty = ty;
                    with(Operator::NarrowWidth, "narrow binding", Op::Bind(m));
                }
            }
            Op::Group(g) => {
                for i in 0..g.keys.len() {
                    let mut m = g.clone();
                    m.keys = without(&g.keys, i);
                    with(
                        Operator::DropKeyColumn,
                        &format!("drop group key {i}"),
                        Op::Group(m),
                    );
                }
            }
            Op::Union(u) => {
                if u.operands.len() > 1 {
                    let mut m = u.clone();
                    m.operands = without(&u.operands, 0);
                    with(Operator::DropConstraint, "drop first arm", Op::Union(m));
                }
            }
            Op::Project(_) | Op::Unit(_) => {}
        }
        if definition.canon.is_some() {
            let mut m = program.clone();
            m.program.definitions[d].canon = None;
            c.push(Operator::DropFlag, site("drop canon"), m);
        }
        let mut m = program.clone();
        m.program.definitions[d].mode = match definition.mode {
            Mode::Pipe(_) => Mode::Let(Default::default()),
            Mode::Let(_) | Mode::Exists(_) => Mode::Pipe(Default::default()),
        };
        c.push(Operator::DropFlag, site("toggle mode"), m);
        if d + 1 < body.definitions.len() {
            let mut m = program.clone();
            m.program.definitions.swap(d, d + 1);
            c.push(Operator::ReorderDefinitions, site("swap with next"), m);
        }
    }
    c.out
}

fn structure_keyed(k: &structures::Keyed) -> Vec<(Operator, &'static str, structures::Keyed)> {
    let mut out = Vec::new();
    for (l, level) in k.levels.iter().enumerate() {
        if level.columns.len() > 1 {
            let mut m = k.clone();
            m.levels[l].columns = level.columns.iter().rev().cloned().collect();
            out.push((Operator::PermuteKey, "reverse level columns", m));
            let mut m = k.clone();
            m.levels[l].columns = without(&level.columns, level.columns.len() - 1);
            out.push((Operator::DropKeyColumn, "drop level column", m));
        }
    }
    if k.levels.len() > 1 {
        let mut m = k.clone();
        m.levels.swap(0, 1);
        out.push((Operator::PermuteKey, "swap levels", m));
        let mut m = k.clone();
        let merged: Vec<syn::Ident> = k
            .levels
            .iter()
            .flat_map(|l| l.columns.iter().cloned())
            .collect();
        m.levels = vec![parse_quote!([#(#merged),*])];
        out.push((Operator::PermuteKey, "merge levels", m));
    }
    if k.membership.is_some() {
        let mut m = k.clone();
        m.membership = None;
        out.push((Operator::DropFlag, "drop membership", m));
    }
    if let Some(filtered) = &k.filtered {
        for i in 0..filtered.constants.len() {
            let mut m = k.clone();
            m.filtered.as_mut().unwrap().constants = without(&filtered.constants, i);
            out.push((Operator::DropConstraint, "drop filtered constant", m));
        }
    }
    out
}

fn keyed_of(structure: &mut structures::Structure) -> Option<&mut structures::Keyed> {
    match structure {
        structures::Structure::Built { keyed, .. } | structures::Structure::Host { keyed, .. } => {
            Some(keyed)
        }
        _ => None,
    }
}

/// Every one-fact mutant of the distinct Physical and Structures stack entries.
pub fn structures(
    physical: &physical::Program,
    program: &structures::Program,
    types: &CarrierTypes,
) -> Vec<Mutant<(physical::Program, structures::Program)>> {
    let mut out: Vec<_> = self::physical(physical, types)
        .into_iter()
        .map(|mutant| Mutant {
            operator: mutant.operator,
            site: format!("physical {}", mutant.site),
            program: (mutant.program, program.clone()),
        })
        .collect();
    let mut c = Collector::new(program);
    let entries = &program.structures.entries;
    for (e, entry) in entries.iter().enumerate() {
        let site = |what: &str| format!("{}: {what}", entry.name);
        let mut probe = entry.structure.clone();
        if let Some(keyed) = keyed_of(&mut probe) {
            for (operator, what, k) in structure_keyed(keyed) {
                let mut m = program.clone();
                *keyed_of(&mut m.structures.entries[e].structure).unwrap() = k;
                c.push(operator, site(what), m);
            }
            // Swap providers with the next keyed entry of a different token.
            let token = keyed.provider.token.clone();
            if let Some(f) = (e + 1..entries.len()).find(|f| {
                let mut other = entries[*f].structure.clone();
                keyed_of(&mut other).is_some_and(|k| k.provider.token != token)
            }) {
                let mut m = program.clone();
                let mut other = m.structures.entries[f].structure.clone();
                let theirs = keyed_of(&mut other).unwrap().provider.clone();
                let mine = std::mem::replace(
                    &mut keyed_of(&mut m.structures.entries[e].structure)
                        .unwrap()
                        .provider,
                    theirs,
                );
                keyed_of(&mut m.structures.entries[f].structure)
                    .unwrap()
                    .provider = mine;
                c.push(Operator::SwapProvider, site("swap provider"), m);
            }
        }
        if let structures::Structure::Dedup { index, .. } = &entry.structure {
            let replacement: structures::CanonIndex = match index {
                structures::CanonIndex::Rows(_) => parse_quote!(host),
                _ => parse_quote!(rows),
            };
            let mut m = program.clone();
            if let structures::Structure::Dedup { index, .. } =
                &mut m.structures.entries[e].structure
            {
                *index = replacement;
            }
            c.push(Operator::SwapProvider, site("change canon index"), m);
        }
        let mut m = program.clone();
        m.structures.entries.remove(e);
        c.push(Operator::DropFlag, site("drop entry"), m);
    }
    out.extend(c.out.into_iter().map(|mutant| Mutant {
        operator: mutant.operator,
        site: mutant.site,
        program: (physical.clone(), mutant.program),
    }));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrowing_pairs_come_from_typed_carriers() {
        for (source, target) in [
            (ColumnType::I32, parse_quote!(i16)),
            (ColumnType::I64, parse_quote!(u32)),
            (ColumnType::U32, parse_quote!(u16)),
            (ColumnType::Usize, parse_quote!(u32)),
        ] {
            assert_eq!(narrower(source), Some(target));
        }
        assert_eq!(narrower(ColumnType::Str), None);
    }

    #[test]
    fn scalar_aliases_and_qualified_types_do_not_select_mutations() {
        let program: physical::Program = parse_quote!(physical {
            relation R(x: Carrier);
            relation answer(y: ::std::primitive::i64);
            r = pipe rename R { x -> x };
            b = pipe bind r with y: OtherCarrier = (x.to_owned());
            p = let canon project b keep { y };
            return p as answer(y);
        });
        let types = CarrierTypes {
            columns: BTreeMap::from([
                (("R".into(), 0), ColumnType::I64),
                (("answer".into(), 0), ColumnType::I64),
            ]),
            bindings: BTreeMap::from([("y".into(), ColumnType::I64)]),
        };
        let mutants = physical(&program, &types)
            .into_iter()
            .filter(|mutant| mutant.operator == Operator::NarrowWidth)
            .collect::<Vec<_>>();
        assert_eq!(mutants.len(), 3);
        assert_eq!(
            mutants[0].program.program.declarations[0].columns[0].ty,
            parse_quote!(u32)
        );
        assert_eq!(
            mutants[1].program.program.declarations[1].columns[0].ty,
            parse_quote!(u32)
        );
        let physical::Operator::Bind(binding) = &mutants[2].program.program.definitions[1].operator
        else {
            panic!("expected the binding mutant");
        };
        assert_eq!(binding.annotation.as_ref().unwrap().ty, parse_quote!(u32));
        assert!(
            physical(&program, &CarrierTypes::default())
                .iter()
                .all(|mutant| mutant.operator != Operator::NarrowWidth)
        );
    }

    #[test]
    fn model_carriers_preserve_raw_names_and_optional_outputs() {
        use crate::fuzz::{
            Atom, Binding, Case, Expr, Fold, FoldArg, FoldKind, Query, Relation, Term,
        };

        let case = Case {
            seed: 0,
            query: Query {
                schema: vec![Relation {
                    name: "r#match".into(),
                    types: vec![ColumnType::I64, ColumnType::U32],
                    key: Vec::new(),
                }],
                atoms: vec![Atom {
                    relation: 0,
                    terms: vec![Term::Var(0), Term::Var(2)],
                }],
                bindings: vec![Binding {
                    variable: 1,
                    expression: Expr::Constant(7),
                    generator: None,
                }],
                head: vec![Expr::Var(0)],
                folds: vec![
                    Fold {
                        kind: FoldKind::Count,
                        argument: None,
                    },
                    Fold {
                        kind: FoldKind::Min,
                        argument: Some(FoldArg::Var(2)),
                    },
                ],
                raw_names: true,
                negations: Vec::new(),
                guards: Vec::new(),
                constraints: Vec::new(),
                blocks: Vec::new(),
                order: Vec::new(),
            },
            data: vec![Vec::new()],
        };
        let types = CarrierTypes::from_case(&case);
        assert_eq!(types.columns[&("match".into(), 0)], ColumnType::I64);
        assert_eq!(types.columns[&("match".into(), 1)], ColumnType::U32);
        assert_eq!(types.columns[&("answer".into(), 0)], ColumnType::I64);
        assert_eq!(types.columns[&("answer".into(), 1)], ColumnType::Usize);
        assert!(!types.columns.contains_key(&("answer".into(), 2)));
        assert_eq!(types.bindings["type"], ColumnType::I32);
    }
}
