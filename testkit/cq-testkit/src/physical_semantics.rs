//! Independent interpretation of complete Physical programs under Set semantics.
//!
//! Each definition denotes a finite set of named, typed valuations. The
//! interpreter follows the *recorded* program: a keyed operator matches only
//! its written key columns (a shared non-key column keeps the stream-side
//! value, as a provider read would), a rename reads the written host columns,
//! a select applies its written theta, and a group folds its written keys.
//! Execution choices (`pipe`/`let`/`exists`, `canon`, probe `first`/`smallest`,
//! `merge`, semi `eager`/`lazy`, union witnesses and fusion, host sets) do not
//! change a Set denotation and are ignored. A checker that accepts a changed
//! record is therefore judged by the answer the changed record denotes.
//!
//! Rust payloads are evaluated by a small total-integer/string fragment
//! interpreter: literals, `i32::MAX`-style constants, `+ - * / %`, unary `-`
//! and `!`, comparisons, `&&`/`||`, `as` casts between integer types,
//! `to_owned`/`clone`/`std::convert::identity`/`Some(..).unwrap()`, `min`/`max`/
//! `abs`, blocks with `let`, `if`, bounded ranges, `vec![..]`,
//! `std::iter::empty()` and `.map(closure)`. Integer overflow, division by zero
//! and `None.unwrap()` are reported as [`Error::Panic`]; anything else is
//! [`Error::Unsupported`]. Only language syntax and `values` are used; no
//! pass, planner or language analysis helper is called.

use std::collections::{BTreeMap, BTreeSet};

use crate::physical;
use crate::values::{TypedRows, Value};
use cq_syntax::symbol_name;

/// Declared relation names mapped to typed positional rows.
pub type TypedDatabase = BTreeMap<String, TypedRows>;
/// One complete typed valuation of a heading.
pub type Binding = BTreeMap<String, Value>;

/// A finite named-heading relation of typed values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relation {
    pub heading: BTreeSet<String>,
    pub rows: BTreeSet<Binding>,
}

/// Every interpreted definition and the ordered answer tuples.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evaluation {
    pub definitions: BTreeMap<String, Relation>,
    pub output: TypedRows,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Syntax outside the interpreted fragment; the stage is not judged.
    Unsupported(String),
    /// The written Rust would panic (overflow, division by zero, unwrap).
    Panic(String),
    /// The program reads something it does not define or declare.
    Malformed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(m) => write!(f, "unsupported: {m}"),
            Self::Panic(m) => write!(f, "panic: {m}"),
            Self::Malformed(m) => write!(f, "malformed: {m}"),
        }
    }
}

impl std::error::Error for Error {}

type Result<T, E = Error> = std::result::Result<T, E>;

fn unsupported<T>(what: impl quote::ToTokens) -> Result<T> {
    Err(Error::Unsupported(what.to_token_stream().to_string()))
}

fn malformed<T>(message: impl Into<String>) -> Result<T> {
    Err(Error::Malformed(message.into()))
}

/// Interpret `program` over `database`. `parameters` supplies external host
/// values named by closed relational operands (`@(name)`) and Rust payloads.
pub fn evaluate(
    program: &physical::Program,
    database: &TypedDatabase,
    parameters: &BTreeMap<String, Value>,
) -> Result<Evaluation> {
    let body = &program.program;
    let declarations: BTreeMap<String, &cq_syntax::RelationDecl> = body
        .declarations
        .iter()
        .map(|d| (symbol_name(&d.name), d))
        .collect();
    let mut env: Scope = parameters
        .iter()
        .map(|(k, v)| (k.clone(), Scalar::from_value(v)))
        .collect();
    let mut gate = true;
    for capture in &body.captures.items {
        match capture {
            cq_syntax::capture::Capture::Let(binding) => {
                let value = eval(&binding.expression, &env)?;
                let annotation = binding.annotation.as_ref().map(|a| &a.ty);
                bind_pattern(&binding.pattern, value, annotation, &mut env)?;
            }
            cq_syntax::capture::Capture::Guard(filter) => {
                gate &= eval(&filter.condition, &env)?.truth()?;
            }
            // Dictionary domains come from runtime storage, which this
            // evaluator does not model.
            cq_syntax::capture::Capture::Domain(_) => {
                return Err(Error::Unsupported("domain capture".to_owned()));
            }
        }
    }
    let mut definitions = BTreeMap::<String, Relation>::new();
    for definition in &body.definitions {
        let name = symbol_name(&definition.name);
        let get = |id: &syn::Ident| -> Result<&Relation> {
            definitions
                .get(&symbol_name(id))
                .map_or_else(|| malformed(format!("`{id}` read before definition")), Ok)
        };
        let relation = match &definition.operator {
            physical::Operator::Unit(_) => Relation {
                heading: BTreeSet::new(),
                rows: BTreeSet::from([Binding::new()]),
            },
            physical::Operator::Rename(rename) => {
                let host = symbol_name(&rename.relation);
                let Some(declaration) = declarations.get(&host) else {
                    return malformed(format!("undeclared host `{host}`"));
                };
                let columns: Vec<String> = declaration
                    .columns
                    .iter()
                    .map(|c| symbol_name(&c.name))
                    .collect();
                let rows = database
                    .get(&host)
                    .map_or_else(|| malformed(format!("missing table `{host}`")), Ok)?;
                let mut out = BTreeSet::new();
                for row in rows {
                    if row.len() != columns.len() {
                        return malformed(format!("table `{host}` has a row of the wrong width"));
                    }
                    let source: BTreeMap<&str, &Value> =
                        columns.iter().map(String::as_str).zip(row).collect();
                    if let Some(constants) = &rename.constants {
                        let mut keep = true;
                        for equality in &constants.equalities {
                            let column = symbol_name(&equality.left);
                            let Some(value) = source.get(column.as_str()) else {
                                return malformed(format!("`{host}` has no column `{column}`"));
                            };
                            let right = match &equality.right {
                                cq_syntax::EqualityValue::Variable(v) => {
                                    match source.get(symbol_name(v).as_str()) {
                                        Some(value) => Scalar::from_value(value),
                                        None => return malformed(format!("unknown column `{v}`")),
                                    }
                                }
                                cq_syntax::EqualityValue::Literal(l) => eval(l, &env)?,
                            };
                            keep &= Scalar::from_value(value).equals(&right)?;
                        }
                        if !keep {
                            continue;
                        }
                    }
                    let mut binding = Binding::new();
                    for mapping in &rename.mapping {
                        let column = symbol_name(&mapping.source);
                        let Some(value) = source.get(column.as_str()) else {
                            return malformed(format!("`{host}` has no column `{column}`"));
                        };
                        binding.insert(symbol_name(&mapping.target), (*value).clone());
                    }
                    out.insert(binding);
                }
                Relation {
                    heading: rename
                        .mapping
                        .iter()
                        .map(|m| symbol_name(&m.target))
                        .collect(),
                    rows: out,
                }
            }
            physical::Operator::Select(s)
            | physical::Operator::Point(_, s)
            | physical::Operator::Range(_, s) => {
                let input = get(&s.input)?;
                let mut rows = BTreeSet::new();
                for row in &input.rows {
                    if theta(&s.condition, row, &env)? {
                        rows.insert(row.clone());
                    }
                }
                Relation {
                    heading: input.heading.clone(),
                    rows,
                }
            }
            physical::Operator::Guard(g) => {
                let input = get(&g.input)?;
                let mut rows = BTreeSet::new();
                for row in &input.rows {
                    if eval(&g.condition, &row_scope(&env, row))?.truth()? {
                        rows.insert(row.clone());
                    }
                }
                Relation {
                    heading: input.heading.clone(),
                    rows,
                }
            }
            physical::Operator::Bind(b) => {
                let input = get(&b.input)?;
                let annotation = b.annotation.as_ref().map(|a| &a.ty);
                let binders: Vec<String> = cq_syntax::rust::bindings(&b.pattern)
                    .iter()
                    .map(symbol_name)
                    .collect();
                let mut heading = input.heading.clone();
                for binder in &binders {
                    if !heading.insert(binder.clone()) {
                        return malformed(format!("bind repeats `{binder}`"));
                    }
                }
                let mut rows = BTreeSet::new();
                for row in &input.rows {
                    let scope = row_scope(&env, row);
                    let value = eval(&b.expression, &scope)?;
                    let values = if b.for_token.is_some() {
                        value.items()?
                    } else {
                        vec![value]
                    };
                    for value in values {
                        let mut local = Scope::new();
                        bind_pattern(&b.pattern, value, annotation, &mut local)?;
                        let mut out = row.clone();
                        for binder in &binders {
                            out.insert(binder.clone(), local[binder].to_value(None)?);
                        }
                        rows.insert(out);
                    }
                }
                Relation { heading, rows }
            }
            physical::Operator::Project(p) => {
                let input = get(&p.input)?;
                let heading: BTreeSet<String> = p.attributes.iter().map(symbol_name).collect();
                if !heading.is_subset(&input.heading) {
                    return malformed(format!("project `{name}` keeps unavailable columns"));
                }
                project(input, &heading)
            }
            physical::Operator::Group(group) => {
                let input = get(&group.input)?;
                let keys: Vec<String> = group
                    .keys
                    .iter()
                    .map(symbol_name)
                    .filter(|k| !env.contains_key(k))
                    .collect();
                if !keys.iter().all(|k| input.heading.contains(k)) {
                    return malformed(format!("group `{name}` keys unavailable columns"));
                }
                let mut groups: BTreeMap<Vec<Value>, Vec<&Binding>> = BTreeMap::new();
                for row in &input.rows {
                    groups
                        .entry(keys.iter().map(|k| row[k].clone()).collect())
                        .or_default()
                        .push(row);
                }
                if keys.is_empty() && groups.is_empty() {
                    groups.insert(Vec::new(), Vec::new());
                }
                let mut heading: BTreeSet<String> = keys.iter().cloned().collect();
                heading.extend(group.aggregates.iter().map(|a| symbol_name(&a.name)));
                let mut rows = BTreeSet::new();
                for (key, members) in groups {
                    let mut out: Binding = keys.iter().cloned().zip(key).collect();
                    let mut absent = false;
                    for aggregate in &group.aggregates {
                        match fold(aggregate, &members, &env)? {
                            Some(value) => {
                                out.insert(symbol_name(&aggregate.name), value);
                            }
                            None => absent = true,
                        }
                    }
                    if absent {
                        return Err(Error::Unsupported(format!(
                            "group `{name}`: empty extremum (None) has no Value encoding"
                        )));
                    }
                    rows.insert(out);
                }
                Relation { heading, rows }
            }
            physical::Operator::Probe(_, k) | physical::Operator::Merge(_, k) => {
                let left = get(&k.left)?;
                let right = get(&k.right)?;
                let columns = key_columns(k, left, right)?;
                let index = key_index(right, &columns);
                let mut rows = BTreeSet::new();
                for l in &left.rows {
                    let key: Vec<&Value> = columns.iter().map(|c| &l[c]).collect();
                    for r in index.get(&key).into_iter().flatten() {
                        let mut out = (*r).clone();
                        out.extend(l.iter().map(|(c, v)| (c.clone(), v.clone())));
                        rows.insert(out);
                    }
                }
                Relation {
                    heading: left.heading.union(&right.heading).cloned().collect(),
                    rows,
                }
            }
            physical::Operator::Semi(_, k) | physical::Operator::Anti(_, k) => {
                let left = get(&k.left)?;
                let right = get(&k.right)?;
                let columns = key_columns(k, left, right)?;
                let keep = matches!(definition.operator, physical::Operator::Semi(..));
                let index = key_index(right, &columns);
                Relation {
                    heading: left.heading.clone(),
                    rows: left
                        .rows
                        .iter()
                        .filter(|l| {
                            let key: Vec<&Value> = columns.iter().map(|c| &l[c]).collect();
                            index.contains_key(&key) == keep
                        })
                        .cloned()
                        .collect(),
                }
            }
            physical::Operator::Union(union) => {
                let first = get(&union.operands[0])?;
                let heading = first.heading.clone();
                let mut rows = BTreeSet::new();
                for operand in &union.operands {
                    let relation = get(operand)?;
                    if relation.heading != heading {
                        return malformed(format!("union `{name}` mixes headings"));
                    }
                    rows.extend(relation.rows.iter().cloned());
                }
                Relation { heading, rows }
            }
        };
        if definitions.insert(name.clone(), relation).is_some() {
            return malformed(format!("`{name}` defined twice"));
        }
    }
    let returned = symbol_name(&body.returned);
    let Some(result) = definitions.get(&returned) else {
        return malformed(format!("return `{returned}` names no result"));
    };
    let output_types: Vec<Option<syn::Type>> = declarations
        .get(&symbol_name(&body.head.relation))
        .map(|d| d.columns.iter().map(|c| Some(c.ty.clone())).collect())
        .unwrap_or_else(|| vec![None; body.head.args.len()]);
    let mut output = TypedRows::new();
    if gate {
        for row in &result.rows {
            let scope = row_scope(&env, row);
            let mut tuple = Vec::with_capacity(body.head.args.len());
            for (position, argument) in body.head.args.iter().enumerate() {
                let ty = output_types.get(position).cloned().flatten();
                tuple.push(eval(argument, &scope)?.to_value(ty.as_ref())?);
            }
            output.insert(tuple);
        }
    }
    Ok(Evaluation {
        definitions,
        output,
    })
}

fn key_index<'a>(
    relation: &'a Relation,
    columns: &[String],
) -> BTreeMap<Vec<&'a Value>, Vec<&'a Binding>> {
    let mut index: BTreeMap<Vec<&Value>, Vec<&Binding>> = BTreeMap::new();
    for row in &relation.rows {
        index
            .entry(columns.iter().map(|c| &row[c]).collect())
            .or_default()
            .push(row);
    }
    index
}

fn key_columns(k: &physical::Keyed, left: &Relation, right: &Relation) -> Result<Vec<String>> {
    let columns: Vec<String> = k.columns.iter().map(symbol_name).collect();
    for c in &columns {
        if !left.heading.contains(c) || !right.heading.contains(c) {
            return malformed(format!("key column `{c}` is not in both operands"));
        }
    }
    Ok(columns)
}

fn project(input: &Relation, heading: &BTreeSet<String>) -> Relation {
    Relation {
        rows: input
            .rows
            .iter()
            .map(|row| {
                heading
                    .iter()
                    .map(|c| (c.clone(), row[c].clone()))
                    .collect()
            })
            .collect(),
        heading: heading.clone(),
    }
}

fn fold(
    aggregate: &cq_syntax::algebra::AggregateBinding,
    members: &[&Binding],
    env: &Scope,
) -> Result<Option<Value>> {
    use cq_syntax::algebra::AggregateKind;
    let ty = &aggregate.ty;
    if aggregate.kind == AggregateKind::Count {
        return Ok(Some(
            Scalar::Int(members.len() as i128, Some(IntTy::Usize)).to_value(Some(ty))?,
        ));
    }
    let Some(expression) = &aggregate.expression else {
        return unsupported(aggregate);
    };
    let mut values = Vec::with_capacity(members.len());
    for row in members {
        values.push(eval(expression, &row_scope(env, row))?);
    }
    match aggregate.kind {
        AggregateKind::Sum => {
            let mut total = Scalar::Int(0, int_type(ty));
            for value in values {
                total = arithmetic(&syn::BinOp::Add(Default::default()), total, value)?;
            }
            total.to_value(Some(ty)).map(Some)
        }
        AggregateKind::Min | AggregateKind::Max => {
            let inner = option_inner(ty);
            let mut best: Option<Scalar> = None;
            for value in values {
                best = Some(match best {
                    None => value,
                    Some(current) => {
                        let less = value.compare(&current)? == std::cmp::Ordering::Less;
                        if less == (aggregate.kind == AggregateKind::Min) {
                            value
                        } else {
                            current
                        }
                    }
                });
            }
            best.map(|v| v.to_value(inner.as_ref())).transpose()
        }
        AggregateKind::Count => unreachable!(),
    }
}

fn option_inner(ty: &syn::Type) -> Option<syn::Type> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    let last = path.path.segments.last()?;
    if last.ident != "Option" {
        return Some(ty.clone());
    }
    let syn::PathArguments::AngleBracketed(arguments) = &last.arguments else {
        return None;
    };
    match arguments.args.first()? {
        syn::GenericArgument::Type(inner) => Some(inner.clone()),
        _ => None,
    }
}

fn theta(condition: &cq_syntax::algebra::Theta, row: &Binding, env: &Scope) -> Result<bool> {
    for equality in &condition.equalities {
        let left = column(row, &symbol_name(&equality.left))?;
        let right = match &equality.right {
            cq_syntax::EqualityValue::Variable(v) => column(row, &symbol_name(v))?,
            cq_syntax::EqualityValue::Literal(l) => eval(l, env)?,
        };
        if !left.equals(&right)? {
            return Ok(false);
        }
    }
    for constraint in &condition.constraints {
        if !relational(constraint, row, env)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn column(row: &Binding, name: &str) -> Result<Scalar> {
    row.get(name)
        .map(Scalar::from_value)
        .map_or_else(|| malformed(format!("unbound column `{name}`")), Ok)
}

fn relational(c: &cq_syntax::Constraint, row: &Binding, env: &Scope) -> Result<bool> {
    use cq_syntax::constraint::{Comparison, Constraint, Term};
    let value = |t: &Term| -> Result<Scalar> {
        match t {
            Term::Variable(v) => column(row, &symbol_name(v)),
            Term::Closed(e) => eval(e, env),
            Term::Literal(l) => eval(l, env),
        }
    };
    Ok(match c {
        Constraint::Compare {
            left,
            comparison,
            right,
        } => {
            let ordering = value(left)?.compare(&value(right)?)?;
            use std::cmp::Ordering::*;
            match comparison {
                Comparison::Eq(_) => ordering == Equal,
                Comparison::Ne(_) => ordering != Equal,
                Comparison::Lt(_) => ordering == Less,
                Comparison::Le(_) => ordering != Greater,
                Comparison::Gt(_) => ordering == Greater,
                Comparison::Ge(_) => ordering != Less,
            }
        }
        Constraint::Between { term, low, high } => {
            let t = value(term)?;
            value(low)?.compare(&t)? != std::cmp::Ordering::Greater
                && t.compare(&value(high)?)? != std::cmp::Ordering::Greater
        }
        Constraint::In { term, literals } => {
            let t = value(term)?;
            let mut found = false;
            for literal in literals {
                found |= t.equals(&eval(literal, env)?)?;
            }
            found
        }
        Constraint::Or(branches) => {
            let mut any = false;
            for branch in branches {
                let mut all = true;
                for c in branch {
                    all &= relational(c, row, env)?;
                }
                any |= all;
            }
            any
        }
    })
}

// ---------------------------------------------------------------------------
// Rust payload fragment.

/// Rust integer carrier types modeled by the payload interpreter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntTy {
    I8,
    I16,
    I32,
    I64,
    I128,
    Isize,
    U8,
    U16,
    U32,
    U64,
    Usize,
}

impl IntTy {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "i8" => Self::I8,
            "i16" => Self::I16,
            "i32" => Self::I32,
            "i64" => Self::I64,
            "i128" => Self::I128,
            "isize" => Self::Isize,
            "u8" => Self::U8,
            "u16" => Self::U16,
            "u32" => Self::U32,
            "u64" => Self::U64,
            "usize" => Self::Usize,
            _ => return None,
        })
    }

    fn bounds(self) -> (i128, i128) {
        match self {
            Self::I8 => (i8::MIN as i128, i8::MAX as i128),
            Self::I16 => (i16::MIN as i128, i16::MAX as i128),
            Self::I32 => (i32::MIN as i128, i32::MAX as i128),
            Self::I64 | Self::Isize => (i64::MIN as i128, i64::MAX as i128),
            Self::I128 => (i128::MIN, i128::MAX),
            Self::U8 => (0, u8::MAX as i128),
            Self::U16 => (0, u16::MAX as i128),
            Self::U32 => (0, u32::MAX as i128),
            Self::U64 | Self::Usize => (0, u64::MAX as i128),
        }
    }

    /// `as` conversion: two's complement truncation to this width.
    fn wrap(self, value: i128) -> i128 {
        let bits = match self {
            Self::I8 | Self::U8 => 8,
            Self::I16 | Self::U16 => 16,
            Self::I32 | Self::U32 => 32,
            Self::I64 | Self::U64 | Self::Isize | Self::Usize => 64,
            Self::I128 => return value,
        };
        let mask = (1i128 << bits) - 1;
        let low = value & mask;
        let signed = matches!(
            self,
            Self::I8 | Self::I16 | Self::I32 | Self::I64 | Self::Isize
        );
        if signed && low >> (bits - 1) == 1 {
            low - (1i128 << bits)
        } else {
            low
        }
    }
}

/// A Rust value in the interpreted fragment. Unsuffixed integer literals carry
/// no type until an operand, annotation or declared column supplies one.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Scalar {
    Int(i128, Option<IntTy>),
    Bool(bool),
    Str(String),
    Option(Option<Box<Scalar>>),
    Items(Vec<Scalar>),
}

type Scope = BTreeMap<String, Scalar>;

fn row_scope(env: &Scope, row: &Binding) -> Scope {
    let mut scope = env.clone();
    scope.extend(row.iter().map(|(k, v)| (k.clone(), Scalar::from_value(v))));
    scope
}

fn int_type(ty: &syn::Type) -> Option<IntTy> {
    match ty {
        syn::Type::Path(p) => p
            .path
            .get_ident()
            .and_then(|i| IntTy::parse(&i.to_string())),
        syn::Type::Paren(p) => int_type(&p.elem),
        _ => None,
    }
}

fn is_str(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Reference(r) => is_str(&r.elem),
        syn::Type::Path(p) => p.path.is_ident("str") || p.path.is_ident("String"),
        _ => false,
    }
}

impl Scalar {
    fn from_value(value: &Value) -> Self {
        match value {
            Value::I32(v) => Self::Int(*v as i128, Some(IntTy::I32)),
            Value::I64(v) => Self::Int(*v as i128, Some(IntTy::I64)),
            Value::U32(v) => Self::Int(*v as i128, Some(IntTy::U32)),
            Value::Usize(v) => Self::Int(*v as i128, Some(IntTy::Usize)),
            Value::Str(s) => Self::Str(s.clone()),
        }
    }

    /// Store as a row value; `ty` (declared column or annotation) wins.
    fn to_value(&self, ty: Option<&syn::Type>) -> Result<Value> {
        if let Some(ty) = ty {
            if let Some(inner) = option_inner(ty).filter(|inner| inner != ty) {
                return match self {
                    Self::Option(Some(v)) => v.to_value(Some(&inner)),
                    Self::Option(None) => Err(Error::Unsupported(
                        "None output has no Value encoding".into(),
                    )),
                    other => other.to_value(Some(&inner)),
                };
            }
        }
        match self {
            Self::Int(v, carried) => {
                let declared = ty.and_then(int_type);
                let target = declared.or(*carried).unwrap_or(IntTy::I32);
                if let (Some(declared), Some(carried)) = (declared, carried) {
                    if declared != *carried {
                        return Err(Error::Unsupported(format!(
                            "{carried:?} value stored as {declared:?}"
                        )));
                    }
                }
                let (lo, hi) = target.bounds();
                if *v < lo || *v > hi {
                    return Err(Error::Panic(format!("{v} out of {target:?} range")));
                }
                Ok(match target {
                    IntTy::I32 => Value::I32(*v as i32),
                    IntTy::I64 => Value::I64(*v as i64),
                    IntTy::U32 => Value::U32(*v as u32),
                    IntTy::Usize | IntTy::U64 => Value::Usize(*v as u64),
                    other => return Err(Error::Unsupported(format!("{other:?} column"))),
                })
            }
            Self::Str(s) if ty.is_none_or(is_str) => Ok(Value::Str(s.clone())),
            other => Err(Error::Unsupported(format!("store {other:?}"))),
        }
    }

    fn truth(&self) -> Result<bool> {
        match self {
            Self::Bool(b) => Ok(*b),
            other => Err(Error::Unsupported(format!("non-bool condition {other:?}"))),
        }
    }

    fn items(self) -> Result<Vec<Scalar>> {
        match self {
            Self::Items(items) => Ok(items),
            Self::Option(o) => Ok(o.map(|v| *v).into_iter().collect()),
            other => Err(Error::Unsupported(format!("iterate {other:?}"))),
        }
    }

    fn compare(&self, other: &Self) -> Result<std::cmp::Ordering> {
        match (self, other) {
            (Self::Int(a, ta), Self::Int(b, tb)) => {
                if let (Some(ta), Some(tb)) = (ta, tb) {
                    if ta != tb {
                        return Err(Error::Unsupported(format!("compare {ta:?} with {tb:?}")));
                    }
                }
                Ok(a.cmp(b))
            }
            (Self::Bool(a), Self::Bool(b)) => Ok(a.cmp(b)),
            (Self::Str(a), Self::Str(b)) => Ok(a.cmp(b)),
            (Self::Option(a), Self::Option(b)) => match (a, b) {
                (None, None) => Ok(std::cmp::Ordering::Equal),
                (None, Some(_)) => Ok(std::cmp::Ordering::Less),
                (Some(_), None) => Ok(std::cmp::Ordering::Greater),
                (Some(a), Some(b)) => a.compare(b),
            },
            (a, b) => Err(Error::Unsupported(format!("compare {a:?} with {b:?}"))),
        }
    }

    fn equals(&self, other: &Self) -> Result<bool> {
        Ok(self.compare(other)? == std::cmp::Ordering::Equal)
    }
}

fn unify(a: Option<IntTy>, b: Option<IntTy>) -> Result<Option<IntTy>> {
    match (a, b) {
        (Some(a), Some(b)) if a != b => Err(Error::Unsupported(format!("mix {a:?} and {b:?}"))),
        (a, b) => Ok(a.or(b)),
    }
}

fn checked(value: i128, ty: Option<IntTy>) -> Result<Scalar> {
    let (lo, hi) = ty.unwrap_or(IntTy::I32).bounds();
    if value < lo || value > hi {
        return Err(Error::Panic(format!("arithmetic overflow ({value})")));
    }
    Ok(Scalar::Int(value, ty))
}

fn arithmetic(op: &syn::BinOp, left: Scalar, right: Scalar) -> Result<Scalar> {
    let (Scalar::Int(a, ta), Scalar::Int(b, tb)) = (&left, &right) else {
        return Err(Error::Unsupported(format!(
            "arithmetic on {left:?}, {right:?}"
        )));
    };
    let ty = unify(*ta, *tb)?;
    let value = match op {
        syn::BinOp::Add(_) => a + b,
        syn::BinOp::Sub(_) => a - b,
        syn::BinOp::Mul(_) => a
            .checked_mul(*b)
            .ok_or_else(|| Error::Panic("overflow".into()))?,
        syn::BinOp::Div(_) | syn::BinOp::Rem(_) => {
            if *b == 0 {
                return Err(Error::Panic("division by zero".into()));
            }
            // MIN / -1 overflows; the range check below reports it.
            if matches!(op, syn::BinOp::Div(_)) {
                a / b
            } else {
                let (lo, _) = ty.unwrap_or(IntTy::I32).bounds();
                if *a == lo && *b == -1 {
                    return Err(Error::Panic("remainder overflow".into()));
                }
                a % b
            }
        }
        other => return unsupported(other),
    };
    checked(value, ty)
}

fn eval(expr: &syn::Expr, scope: &Scope) -> Result<Scalar> {
    use syn::Expr;
    match expr {
        Expr::Paren(p) => eval(&p.expr, scope),
        Expr::Group(g) => eval(&g.expr, scope),
        Expr::Lit(l) => literal(&l.lit),
        Expr::Path(p) => path_value(&p.path, scope),
        Expr::Reference(r) => eval(&r.expr, scope),
        Expr::Unary(u) => {
            let value = eval(&u.expr, scope)?;
            match (&u.op, value) {
                (syn::UnOp::Deref(_), v) => Ok(v),
                (syn::UnOp::Not(_), Scalar::Bool(b)) => Ok(Scalar::Bool(!b)),
                (syn::UnOp::Neg(_), Scalar::Int(v, ty)) => {
                    if ty.is_some_and(|t| t.bounds().0 == 0) {
                        return unsupported(u);
                    }
                    checked(-v, ty)
                }
                _ => unsupported(u),
            }
        }
        Expr::Binary(b) => {
            if matches!(b.op, syn::BinOp::And(_) | syn::BinOp::Or(_)) {
                let left = eval(&b.left, scope)?.truth()?;
                let short = matches!(b.op, syn::BinOp::Or(_));
                if left == short {
                    return Ok(Scalar::Bool(left));
                }
                return Ok(Scalar::Bool(eval(&b.right, scope)?.truth()?));
            }
            let left = eval(&b.left, scope)?;
            let right = eval(&b.right, scope)?;
            use std::cmp::Ordering::*;
            let compare = |l: &Scalar, r: &Scalar| l.compare(r);
            Ok(match b.op {
                syn::BinOp::Eq(_) => Scalar::Bool(compare(&left, &right)? == Equal),
                syn::BinOp::Ne(_) => Scalar::Bool(compare(&left, &right)? != Equal),
                syn::BinOp::Lt(_) => Scalar::Bool(compare(&left, &right)? == Less),
                syn::BinOp::Le(_) => Scalar::Bool(compare(&left, &right)? != Greater),
                syn::BinOp::Gt(_) => Scalar::Bool(compare(&left, &right)? == Greater),
                syn::BinOp::Ge(_) => Scalar::Bool(compare(&left, &right)? != Less),
                _ => arithmetic(&b.op, left, right)?,
            })
        }
        Expr::Cast(c) => {
            let Some(target) = int_type(&c.ty) else {
                return unsupported(c);
            };
            match eval(&c.expr, scope)? {
                Scalar::Int(v, _) => Ok(Scalar::Int(target.wrap(v), Some(target))),
                Scalar::Bool(b) => Ok(Scalar::Int(b as i128, Some(target))),
                _ => unsupported(c),
            }
        }
        Expr::MethodCall(m) => {
            let receiver = eval(&m.receiver, scope)?;
            let method = m.method.to_string();
            let args = m
                .args
                .iter()
                .map(|a| eval(a, scope))
                .collect::<Result<Vec<_>>>();
            match (method.as_str(), receiver) {
                ("to_owned" | "clone" | "to_string" | "copied" | "cloned", v)
                    if m.args.is_empty() =>
                {
                    Ok(v)
                }
                ("unwrap", Scalar::Option(o)) => o
                    .map(|v| *v)
                    .ok_or_else(|| Error::Panic("unwrap on None".into())),
                ("unwrap_or", Scalar::Option(o)) => {
                    let mut args = args?;
                    Ok(o.map(|v| *v).unwrap_or_else(|| args.remove(0)))
                }
                ("is_some", Scalar::Option(o)) => Ok(Scalar::Bool(o.is_some())),
                ("is_none", Scalar::Option(o)) => Ok(Scalar::Bool(o.is_none())),
                ("abs", Scalar::Int(v, ty)) => checked(v.abs(), ty),
                ("min" | "max", Scalar::Int(v, ty)) => {
                    let args = args?;
                    let [Scalar::Int(w, tw)] = args.as_slice() else {
                        return unsupported(m);
                    };
                    let ty = unify(ty, *tw)?;
                    Ok(Scalar::Int(
                        if method == "min" {
                            v.min(*w)
                        } else {
                            v.max(*w)
                        },
                        ty,
                    ))
                }
                ("map", Scalar::Items(items)) => {
                    let [Expr::Closure(closure)] = m.args.iter().collect::<Vec<_>>()[..] else {
                        return unsupported(m);
                    };
                    let [pattern] = closure.inputs.iter().collect::<Vec<_>>()[..] else {
                        return unsupported(m);
                    };
                    let mut out = Vec::with_capacity(items.len());
                    for item in items {
                        let mut local = scope.clone();
                        bind_pattern(pattern, item, None, &mut local)?;
                        out.push(eval(&closure.body, &local)?);
                    }
                    Ok(Scalar::Items(out))
                }
                ("into_iter" | "iter", v @ Scalar::Items(_)) => Ok(v),
                _ => unsupported(m),
            }
        }
        Expr::Call(call) => {
            let Expr::Path(function) = &*call.func else {
                return unsupported(call);
            };
            let segments: Vec<String> = function
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            let last = segments.last().map(String::as_str);
            let args = call
                .args
                .iter()
                .map(|a| eval(a, scope))
                .collect::<Result<Vec<_>>>()?;
            match (last, args.len()) {
                (Some("identity" | "clone"), 1) => Ok(args.into_iter().next().unwrap()),
                (Some("Some"), 1) => Ok(Scalar::Option(Some(Box::new(
                    args.into_iter().next().unwrap(),
                )))),
                (Some("empty"), 0) => Ok(Scalar::Items(Vec::new())),
                _ => unsupported(call),
            }
        }
        Expr::Block(block) => block_value(&block.block, scope),
        Expr::If(e) => {
            if eval(&e.cond, scope)?.truth()? {
                block_value(&e.then_branch, scope)
            } else if let Some((_, otherwise)) = &e.else_branch {
                eval(otherwise, scope)
            } else {
                unsupported(e)
            }
        }
        Expr::Range(r) => {
            let (Some(start), Some(end)) = (&r.start, &r.end) else {
                return unsupported(r);
            };
            let (Scalar::Int(a, ta), Scalar::Int(b, tb)) = (eval(start, scope)?, eval(end, scope)?)
            else {
                return unsupported(r);
            };
            let ty = unify(ta, tb)?;
            let end = if matches!(r.limits, syn::RangeLimits::Closed(_)) {
                b + 1
            } else {
                b
            };
            if end - a > 100_000 {
                return Err(Error::Unsupported("range too long to enumerate".into()));
            }
            Ok(Scalar::Items(
                (a..end).map(|v| Scalar::Int(v, ty)).collect(),
            ))
        }
        Expr::Macro(m) if m.mac.path.is_ident("vec") => {
            let items = m
                .mac
                .parse_body_with(
                    syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated,
                )
                .map_err(|e| Error::Unsupported(e.to_string()))?;
            let mut out = Vec::new();
            for item in &items {
                out.push(eval(item, scope)?);
            }
            Ok(Scalar::Items(out))
        }
        other => unsupported(other),
    }
}

fn block_value(block: &syn::Block, scope: &Scope) -> Result<Scalar> {
    let mut local = scope.clone();
    let mut last = None;
    for (position, statement) in block.stmts.iter().enumerate() {
        match statement {
            syn::Stmt::Local(binding) => {
                let Some(init) = &binding.init else {
                    return unsupported(binding);
                };
                let value = eval(&init.expr, &local)?;
                let (pattern, ty) = match &binding.pat {
                    syn::Pat::Type(typed) => (&*typed.pat, Some(&*typed.ty)),
                    pattern => (pattern, None),
                };
                bind_pattern(pattern, value, ty, &mut local)?;
            }
            syn::Stmt::Expr(e, None) if position + 1 == block.stmts.len() => {
                last = Some(eval(e, &local)?);
            }
            other => return unsupported(other),
        }
    }
    last.map_or_else(|| unsupported(block), Ok)
}

fn retype(value: Scalar, ty: Option<&syn::Type>) -> Result<Scalar> {
    match (value, ty.and_then(int_type)) {
        (Scalar::Int(v, carried), Some(declared)) => {
            unify(carried, Some(declared))?;
            checked(v, Some(declared))
        }
        (value, _) => Ok(value),
    }
}

fn bind_pattern(
    pattern: &syn::Pat,
    value: Scalar,
    ty: Option<&syn::Type>,
    scope: &mut Scope,
) -> Result<()> {
    match pattern {
        syn::Pat::Ident(p) if p.subpat.is_none() => {
            scope.insert(symbol_name(&p.ident), retype(value, ty)?);
            Ok(())
        }
        syn::Pat::Wild(_) => Ok(()),
        syn::Pat::Type(t) => bind_pattern(&t.pat, value, Some(&t.ty), scope),
        syn::Pat::Paren(p) => bind_pattern(&p.pat, value, ty, scope),
        other => unsupported(other),
    }
}

fn path_value(path: &syn::Path, scope: &Scope) -> Result<Scalar> {
    if let Some(ident) = path.get_ident() {
        let name = symbol_name(ident);
        return match scope.get(&name) {
            Some(v) => Ok(v.clone()),
            None if name == "None" => Ok(Scalar::Option(None)),
            None => Err(Error::Unsupported(format!("unbound name `{name}`"))),
        };
    }
    let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    if let [ty, constant] = segments.as_slice() {
        if let Some(ty) = IntTy::parse(ty) {
            let (lo, hi) = ty.bounds();
            return match constant.as_str() {
                "MIN" => Ok(Scalar::Int(lo, Some(ty))),
                "MAX" => Ok(Scalar::Int(hi, Some(ty))),
                _ => unsupported(path),
            };
        }
    }
    unsupported(path)
}

fn literal(lit: &syn::Lit) -> Result<Scalar> {
    match lit {
        syn::Lit::Int(i) => {
            let value: i128 = i
                .base10_parse()
                .map_err(|e| Error::Unsupported(e.to_string()))?;
            let ty = match i.suffix() {
                "" => None,
                suffix => {
                    Some(IntTy::parse(suffix).ok_or_else(|| Error::Unsupported(suffix.into()))?)
                }
            };
            Ok(Scalar::Int(value, ty))
        }
        syn::Lit::Bool(b) => Ok(Scalar::Bool(b.value)),
        syn::Lit::Str(s) => Ok(Scalar::Str(s.value())),
        other => unsupported(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database(tables: &[(&str, &[&[i32]])]) -> TypedDatabase {
        tables
            .iter()
            .map(|(name, rows)| {
                (
                    (*name).to_owned(),
                    rows.iter()
                        .map(|row| row.iter().map(|v| Value::I32(*v)).collect())
                        .collect(),
                )
            })
            .collect()
    }

    fn rows(rows: &[&[i32]]) -> TypedRows {
        rows.iter()
            .map(|row| row.iter().map(|v| Value::I32(*v)).collect())
            .collect()
    }

    fn run(program: physical::Program, db: &TypedDatabase) -> Result<TypedRows> {
        evaluate(&program, db, &BTreeMap::new()).map(|e| e.output)
    }

    #[test]
    fn probe_matches_only_its_recorded_key() {
        let db = database(&[("R", &[&[1, 2]]), ("S", &[&[1, 3], &[9, 2]])]);
        let full: physical::Program = syn::parse_quote!(physical {
            relation R(a: i32, b: i32); relation S(a: i32, b: i32); relation answer(a: i32, b: i32);
            r = pipe rename R@0 { a -> a, b -> b }; s = let rename S@1 { a -> a, b -> b };
            j = pipe probe first [a, b] r, s; return j as answer(a, b);
        });
        assert_eq!(run(full, &db).unwrap(), TypedRows::new());
        // Dropping key column `b` keeps the stream-side `b`: (1, 2) via S(1, 3).
        let dropped: physical::Program = syn::parse_quote!(physical {
            relation R(a: i32, b: i32); relation S(a: i32, b: i32); relation answer(a: i32, b: i32);
            r = pipe rename R@0 { a -> a, b -> b }; s = let rename S@1 { a -> a, b -> b };
            j = pipe probe first [a] r, s; return j as answer(a, b);
        });
        assert_eq!(run(dropped, &db).unwrap(), rows(&[&[1, 2]]));
    }

    #[test]
    fn contradictory_interval_and_typed_payloads() {
        let db = database(&[("R", &[&[1], &[3], &[i32::MAX]])]);
        let contradictory: physical::Program = syn::parse_quote!(physical {
            relation R(x: i32); relation answer(x: i32);
            r = pipe rename R@0 { x -> x }; s = pipe select r if (x < 2, x = 3);
            return s as answer(x);
        });
        assert_eq!(run(contradictory, &db).unwrap(), TypedRows::new());
        let overflow: physical::Program = syn::parse_quote!(physical {
            relation R(x: i32); relation answer(y: i32);
            r = pipe rename R@0 { x -> x }; b = pipe bind r with y: i32 = (x.to_owned() + 1);
            p = let canon project b keep { y }; return p as answer(y);
        });
        assert!(matches!(run(overflow, &db), Err(Error::Panic(_))));
        let bounded: physical::Program = syn::parse_quote!(physical {
            relation R(x: i32); relation answer(y: i32);
            r = pipe rename R@0 { x -> x }; g = pipe guard r if (*x < 3);
            b = pipe bind g with y: i32 = ((10i32 / (((x.to_owned()) % 3) + 4)));
            p = let canon project b keep { y }; return p as answer(y);
        });
        assert_eq!(run(bounded, &db).unwrap(), rows(&[&[2]]));
    }

    #[test]
    fn group_folds_distinct_valuations() {
        let db = database(&[("R", &[&[1, 5], &[1, 7], &[2, 5]])]);
        let program: physical::Program = syn::parse_quote!(physical {
            relation R(k: i32, v: i32); relation answer(k: i32, s: i32, n: usize);
            r = let canon rename R@0 { k -> k, v -> v };
            g = let group r by [k] { s: i32 = sum(*v); n: usize = count(); };
            return g as answer(k, s, n);
        });
        let out = run(program, &db).unwrap();
        assert_eq!(
            out,
            TypedRows::from([
                vec![Value::I32(1), Value::I32(12), Value::Usize(2)],
                vec![Value::I32(2), Value::I32(5), Value::Usize(1)],
            ])
        );
    }
}
