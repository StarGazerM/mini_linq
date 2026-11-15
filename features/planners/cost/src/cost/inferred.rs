//! Structural upper bounds for derived Datalog relations: size-bound
//! adornments (Fattebert, Jiang, Koch, Pichler, Wang; PODS 2026) in a factored
//! form. A bound holds on every database with the cited input facts. It is a
//! worst case, not an estimate.
use std::collections::{BTreeMap, BTreeSet};

use cq_evidence::DatasetEvidence;
use cq_lang_cq::BodyItem;
use cq_lang_datalog_round as round;
use cq_syntax::{declared_evidence::EvidenceView, equality::EqualityValue, rust, symbol_name};
use num_bigint::BigInt;
use num_traits::{One, Zero};
use quote::ToTokens;
use round::incremental::source as dl;
use serde::{Deserialize, Serialize};

use super::bounds;
use crate::export::{self, CheckedIr, ExportError, PlannerInput};

pub const SCHEMA: &str = "cq.inferred-bounds.v1";

fn error(message: impl Into<String>) -> ExportError { ExportError(message.into()) }

/// Where the values of a block come from.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// Columns of an input relation, aligned with the block's columns in
    /// increasing order. `key` are its columns aligned with the block's key:
    /// the block's value occurs among the rows that carry the key's value.
    Input { relation: usize, columns: Vec<usize>, key: Vec<usize> },
    /// One closed value.
    Constant,
}

/// A block of a term: its columns and the columns of earlier blocks that key
/// it, each in increasing order.
pub type Block = (Vec<usize>, Vec<usize>);

/// The cover form of a column set of a relation: every tuple of the
/// projection satisfies some term, or nothing is known.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Form {
    /// Per term, its blocks in order of their first column and the sources
    /// of each block.
    pub terms: BTreeMap<Vec<Block>, Vec<BTreeSet<Source>>>,
    pub unknown: Option<String>,
}

/// One step of a rule's cover: the atom, the head variables assigned to it,
/// and the earlier assigned head variables it also holds.
struct Step<'a> { atom: usize, taken: Vec<&'a str>, key: Vec<&'a str> }
type Term = Vec<(Block, BTreeSet<Source>)>;

enum Argument { Variable(String), Constant, Computed }
struct Atom { relation: usize, columns: Vec<Option<String>> }
struct Rule { head: usize, arguments: Vec<Argument>, atoms: Vec<Atom>, name: String }

/// The rules of a program as the transfer reads them, and the forms computed
/// so far. A form is final once computed: it depends on the program only.
pub struct Inferred {
    names: Vec<String>,
    columns: Vec<Vec<String>>,
    input: Vec<bool>,
    rules: Vec<Rule>,
    heads: Vec<Vec<usize>>,
    forms: BTreeMap<(usize, Vec<usize>), Form>,
}

fn identifiers(tokens: proc_macro2::TokenStream, found: &mut BTreeSet<String>) {
    for token in tokens {
        match token {
            proc_macro2::TokenTree::Ident(name) => { found.insert(symbol_name(&name)); }
            proc_macro2::TokenTree::Group(group) => identifiers(group.stream(), found),
            _ => {}
        }
    }
}

/// A head argument that copies one query variable: `x` or `*x`.
fn copied(expression: &syn::Expr) -> Option<&syn::Ident> {
    match expression {
        syn::Expr::Unary(syn::ExprUnary { op: syn::UnOp::Deref(_), expr, .. }) => copied(expr),
        syn::Expr::Paren(inner) => copied(&inner.expr),
        other => rust::variable(other),
    }
}

impl Inferred {
    pub fn new(program: &dl::Program) -> export::Result<Self> {
        let names: Vec<String> = program.declarations.iter().map(|declaration| symbol_name(&declaration.name)).collect();
        let index = |name: &syn::Ident| names.iter().position(|known| *known == symbol_name(name))
            .ok_or_else(|| error(format!("rule reads undeclared relation {name}")));
        let mut rules = Vec::new();
        let mut heads = vec![Vec::new(); names.len()];
        for (at, query) in program.rules.iter().enumerate() {
            let captures = query.captures.names();
            let mut atoms = Vec::new();
            let mut bound = BTreeSet::new();
            for item in query.body.iter() {
                let BodyItem::Positive { atom } = item else { continue; };
                let columns: Vec<Option<String>> = atom.args.iter().map(|argument| rust::variable(argument)
                    .map(symbol_name).filter(|name| !captures.contains(name))).collect();
                bound.extend(columns.iter().flatten().cloned());
                atoms.push(Atom { relation: index(&atom.relation)?, columns });
            }
            // A variable equated to a bound variable copies it; one equated to
            // a literal is closed.
            let mut alias = BTreeMap::new();
            let mut closed = BTreeSet::new();
            for item in query.body.iter() {
                let BodyItem::Equality(equality) = item else { continue; };
                let left = symbol_name(&equality.left);
                match &equality.right {
                    EqualityValue::Variable(right) => {
                        let right = symbol_name(right);
                        if bound.contains(&right) && !bound.contains(&left) { alias.insert(left, right); }
                        else if bound.contains(&left) && !bound.contains(&right) { alias.insert(right, left); }
                    }
                    EqualityValue::Literal(_) => { if !bound.contains(&left) { closed.insert(left); } }
                }
            }
            let arguments = query.head.args.iter().map(|argument| {
                if let Some(name) = copied(argument).map(symbol_name).filter(|name| !captures.contains(name)) {
                    let name = alias.get(&name).cloned().unwrap_or(name);
                    if closed.contains(&name) { return Argument::Constant; }
                    return Argument::Variable(name);
                }
                let mut mentioned = BTreeSet::new();
                identifiers(argument.to_token_stream(), &mut mentioned);
                // Any query variable in an expression makes its value open.
                if mentioned.iter().any(|name| bound.contains(name) || alias.contains_key(name)) { Argument::Computed }
                else { Argument::Constant }
            }).collect();
            let head = index(&query.head.relation)?;
            heads[head].push(rules.len());
            let name = program.labels.get(at).and_then(Option::as_ref).map_or_else(|| format!("rule {at}"), symbol_name);
            rules.push(Rule { head, arguments, atoms, name });
        }
        Ok(Self {
            columns: program.declarations.iter().map(|declaration| declaration.columns.iter()
                .map(|column| symbol_name(&column.name)).collect()).collect(),
            input: program.declarations.iter().map(|declaration| declaration.value.is_some()).collect(),
            names, rules, heads, forms: BTreeMap::new(),
        })
    }

    pub fn relations(&self) -> usize { self.names.len() }
    pub fn name(&self, relation: usize) -> &str { &self.names[relation] }
    pub fn column_names(&self, relation: usize) -> &[String] { &self.columns[relation] }
    /// Whether rules derive the relation.
    pub fn derived(&self, relation: usize) -> bool { !self.heads[relation].is_empty() }
    /// The relations the relation reads through positive atoms, transitively.
    fn reaches(&self, relation: usize) -> BTreeSet<usize> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![relation];
        while let Some(next) = stack.pop() {
            for &rule in &self.heads[next] {
                for atom in &self.rules[rule].atoms {
                    if seen.insert(atom.relation) { stack.push(atom.relation); }
                }
            }
        }
        seen
    }
    /// Whether the relation depends on itself through positive atoms.
    pub fn recursive(&self, relation: usize) -> bool { self.reaches(relation).contains(&relation) }
    /// The relations mutually recursive with the relation, itself included.
    pub fn component(&self, relation: usize) -> BTreeSet<usize> {
        let mut component: BTreeSet<usize> = self.reaches(relation).into_iter()
            .filter(|&other| self.reaches(other).contains(&relation)).collect();
        component.insert(relation);
        component
    }
    pub fn index(&self, name: &str) -> Option<usize> { self.names.iter().position(|known| known == name) }

    /// A relation with loaded rows only: its degrees are certified facts.
    fn loaded(&self, relation: usize) -> bool { self.input[relation] && !self.derived(relation) }

    /// The rule's cover of the head variables at `columns`: each variable is
    /// assigned to one positive atom that holds it. `Err` is why none exists.
    fn cover<'a>(&self, rule: &'a Rule, columns: &[usize]) -> Result<Vec<Step<'a>>, String> {
        let mut remaining = Vec::<&str>::new();
        for &column in columns {
            match &rule.arguments[column] {
                Argument::Variable(name) => if !remaining.contains(&name.as_str()) { remaining.push(name); },
                Argument::Constant => {}
                Argument::Computed => return Err(format!("{}: head column {} is computed from query variables",
                    rule.name, self.columns[rule.head][column])),
            }
        }
        let mut assigned = Vec::<&str>::new();
        let mut cover = Vec::new();
        while !remaining.is_empty() {
            let holds = |atom: &Atom, name: &str| atom.columns.iter().flatten().any(|held| held == name);
            let held = |atom: &Atom| remaining.iter().filter(|name| holds(atom, name)).count();
            // Most variables first, then an input before a derived relation,
            // then body order.
            let (at, atom) = rule.atoms.iter().enumerate()
                .max_by_key(|(at, atom)| (held(atom), !self.derived(atom.relation), std::cmp::Reverse(*at)))
                .filter(|(_, atom)| held(atom) > 0)
                .ok_or_else(|| format!("{}: no positive atom holds head variable {}", rule.name, remaining[0]))?;
            let (taken, rest): (Vec<&str>, Vec<&str>) = remaining.iter().copied().partition(|name| holds(atom, name));
            let key = if self.loaded(atom.relation) { assigned.iter().copied().filter(|name| holds(atom, name)).collect() } else { Vec::new() };
            assigned.extend(&taken);
            cover.push(Step { atom: at, taken, key });
            remaining = rest;
        }
        Ok(cover)
    }

    /// The forms the transfers of `(relation, columns)` read.
    fn reads(&self, relation: usize, columns: &[usize]) -> Vec<(usize, Vec<usize>)> {
        let mut reads = Vec::new();
        for &rule in &self.heads[relation] {
            let rule = &self.rules[rule];
            let Ok(cover) = self.cover(rule, columns) else { continue; };
            for step in cover {
                let atom = &rule.atoms[step.atom];
                if self.loaded(atom.relation) { continue; }
                let mut assigned: Vec<usize> = step.taken.iter().map(|name| position(atom, name)).collect();
                assigned.sort_unstable();
                reads.push((atom.relation, assigned));
            }
        }
        reads
    }

    /// The terms one rule contributes to `F(head, columns)` from the forms
    /// computed so far. `Err` makes the form unknown.
    fn transfer(&self, rule: &Rule, columns: &[usize]) -> Result<Vec<Term>, String> {
        let cover = self.cover(rule, columns)?;
        let heads = |name: &str| -> Vec<usize> { columns.iter().copied()
            .filter(|&column| matches!(&rule.arguments[column], Argument::Variable(held) if held == name)).collect() };
        let mut terms: Vec<Term> = vec![columns.iter()
            .filter(|&&column| matches!(rule.arguments[column], Argument::Constant))
            .map(|&column| ((vec![column], Vec::new()), BTreeSet::from([Source::Constant]))).collect()];
        for step in cover {
            let atom = &rule.atoms[step.atom];
            if self.loaded(atom.relation) {
                // (head column, source column) of the block and of its key.
                let mut block: Vec<(usize, usize)> = step.taken.iter()
                    .flat_map(|name| heads(name).into_iter().map(|head| (head, position(atom, name)))).collect();
                block.sort_unstable();
                let mut key: Vec<(usize, usize)> = step.key.iter().map(|name| (heads(name)[0], position(atom, name))).collect();
                key.sort_unstable();
                let source = Source::Input { relation: atom.relation,
                    columns: block.iter().map(|&(_, source)| source).collect(), key: key.iter().map(|&(_, source)| source).collect() };
                let placed: Block = (block.iter().map(|&(head, _)| head).collect(), key.iter().map(|&(head, _)| head).collect());
                for term in &mut terms { term.push((placed.clone(), BTreeSet::from([source.clone()]))); }
                continue;
            }
            let mut assigned: Vec<usize> = step.taken.iter().map(|name| position(atom, name)).collect();
            assigned.sort_unstable();
            let form = self.forms.get(&(atom.relation, assigned)).ok_or("internal: form read before it was scheduled")?;
            if let Some(reason) = &form.unknown { return Err(reason.clone()); }
            let mut extended = Vec::new();
            for (blocks, sources) in &form.terms {
                // Carry each block of the atom to the head columns of its variables.
                let carried: Term = blocks.iter().zip(sources).map(|((block, key), sources)| {
                    let variable = |column: usize| atom.columns[column].as_deref().expect("assigned column holds a variable");
                    let mut pairs: Vec<(usize, usize)> = block.iter().enumerate()
                        .flat_map(|(within, &column)| heads(variable(column)).into_iter().map(move |head| (head, within))).collect();
                    pairs.sort_unstable();
                    let mut keys: Vec<(usize, usize)> = key.iter().enumerate()
                        .map(|(within, &column)| (heads(variable(column))[0], within)).collect();
                    keys.sort_unstable();
                    let sources = sources.iter().map(|source| match source {
                        Source::Constant => Source::Constant,
                        Source::Input { relation, columns, key } => Source::Input { relation: *relation,
                            columns: pairs.iter().map(|&(_, within)| columns[within]).collect(),
                            key: keys.iter().map(|&(_, within)| key[within]).collect() },
                    }).collect();
                    ((pairs.into_iter().map(|(head, _)| head).collect(), keys.into_iter().map(|(head, _)| head).collect()), sources)
                }).collect();
                for term in &terms {
                    extended.push(term.iter().cloned().chain(carried.iter().cloned()).collect());
                }
            }
            terms = extended;
        }
        Ok(terms)
    }

    /// Compute `F(relation, columns)` and every form it reads, as the least
    /// fixpoint of the transfers.
    fn ensure(&mut self, relation: usize, columns: &[usize]) {
        if self.forms.contains_key(&(relation, columns.to_vec())) { return; }
        let mut pending = BTreeSet::new();
        let mut stack = vec![(relation, columns.to_vec())];
        while let Some(key) = stack.pop() {
            if self.forms.contains_key(&key) || !pending.insert(key.clone()) { continue; }
            stack.extend(self.reads(key.0, &key.1));
        }
        for key in &pending { self.forms.insert(key.clone(), Form::default()); }
        loop {
            let mut changed = false;
            for key in &pending {
                if self.forms[key].unknown.is_some() { continue; }
                let (relation, columns) = (key.0, &key.1);
                let mut terms = Vec::new();
                let mut unknown = None;
                // Loaded rows are their own source.
                if self.input[relation] {
                    terms.push(vec![((columns.clone(), Vec::new()), BTreeSet::from([Source::Input { relation, columns: columns.clone(), key: Vec::new() }]))]);
                }
                for &rule in &self.heads[relation] {
                    match self.transfer(&self.rules[rule], columns) {
                        Ok(found) => terms.extend(found),
                        Err(reason) => { unknown = Some(reason); break; }
                    }
                }
                let form = self.forms.get_mut(key).expect("scheduled form");
                if let Some(reason) = unknown { form.unknown = Some(reason); changed = true; continue; }
                for mut term in terms {
                    term.sort();
                    let (blocks, sources): (Vec<Block>, Vec<BTreeSet<Source>>) = term.into_iter().unzip();
                    let held = form.terms.entry(blocks).or_insert_with(|| { changed = true; vec![BTreeSet::new(); sources.len()] });
                    for (held, found) in held.iter_mut().zip(sources) {
                        for source in found { changed |= held.insert(source); }
                    }
                }
            }
            if !changed { break; }
        }
    }

    /// `F(relation, columns)`; `columns` are distinct and nonempty.
    pub fn form(&mut self, relation: usize, columns: &BTreeSet<usize>) -> &Form {
        let columns: Vec<usize> = columns.iter().copied().collect();
        self.ensure(relation, &columns);
        &self.forms[&(relation, columns)]
    }

    /// The form's terms as relaxed rule bodies over inputs: a head column
    /// name at each source column that feeds it, in brackets where it keys
    /// the source, `_` elsewhere.
    pub fn render(&self, relation: usize, form: &Form) -> Vec<String> {
        let named = |column: usize| self.columns[relation][column].as_str();
        form.terms.iter().map(|(blocks, sources)| blocks.iter().zip(sources).map(|((block, keyed), sources)| {
            let alternatives: Vec<String> = sources.iter().map(|source| match source {
                Source::Constant => format!("const({})", block.iter().map(|&column| named(column)).collect::<Vec<_>>().join(",")),
                Source::Input { relation: input, columns, key } => {
                    let arguments: Vec<String> = (0..self.columns[*input].len()).map(|at| {
                        let fed: Vec<&str> = block.iter().zip(columns).filter(|(_, source)| **source == at).map(|(&column, _)| named(column)).collect();
                        let keys: Vec<String> = keyed.iter().zip(key).filter(|(_, source)| **source == at).map(|(&column, _)| format!("[{}]", named(column))).collect();
                        if fed.is_empty() && keys.is_empty() { "_".into() }
                        else { fed.into_iter().map(str::to_string).chain(keys).collect::<Vec<_>>().join("=") }
                    }).collect();
                    format!("{}({})", self.names[*input], arguments.join(","))
                }
            }).collect();
            if alternatives.len() == 1 { alternatives.into_iter().next().unwrap() } else { format!("{{{}}}", alternatives.join(" | ")) }
        }).collect::<Vec<_>>().join(", ")).collect()
    }

    fn direct(&mut self, facts: &InputFacts, relation: usize, columns: &[usize]) -> Result<Certified, String> {
        self.ensure(relation, columns);
        let form = &self.forms[&(relation, columns.to_vec())];
        if let Some(reason) = &form.unknown { return Err(reason.clone()); }
        let mut total = Certified { value: BigInt::zero(), citations: BTreeSet::new() };
        for sources in form.terms.values() {
            let mut product = BigInt::one();
            for block in sources {
                let mut sum = BigInt::zero();
                for source in block {
                    let support = facts.support(self, source)?;
                    sum += support.value;
                    total.citations.extend(support.citations);
                }
                product *= sum;
            }
            total.value += product;
        }
        Ok(total)
    }

    /// `D_q(C)`: a certified bound on the distinct values of the relation's
    /// projection onto `columns`. All columns give its rows. `Err` is why no
    /// bound exists.
    pub fn support(&mut self, facts: &InputFacts, relation: usize, columns: &BTreeSet<usize>) -> Result<Certified, String> {
        let columns: Vec<usize> = columns.iter().copied().collect();
        if columns.is_empty() { return Ok(Certified { value: BigInt::one(), citations: BTreeSet::new() }); }
        let joint = self.direct(facts, relation, &columns)?;
        if columns.len() == 1 { return Ok(joint); }
        // Each column alone: D_q(C) <= prod D_q({c}).
        let mut separate = Certified { value: BigInt::one(), citations: BTreeSet::new() };
        for &column in &columns {
            let single = self.direct(facts, relation, &[column])?;
            separate.value *= single.value;
            separate.citations.extend(single.citations);
        }
        Ok(if separate.value < joint.value { separate } else { joint })
    }

    /// `M_q(K) <= D_q(columns \ K)`.
    pub fn degree(&mut self, facts: &InputFacts, relation: usize, key: &BTreeSet<usize>) -> Result<Certified, String> {
        let rest: BTreeSet<usize> = (0..self.columns[relation].len()).filter(|column| !key.contains(column)).collect();
        self.support(facts, relation, &rest)
    }
}

/// The first column of the atom that holds the variable.
fn position(atom: &Atom, name: &str) -> usize {
    atom.columns.iter().position(|held| held.as_deref() == Some(name)).expect("cover assigns a held variable")
}

/// A certified upper bound and the input facts it cites.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Certified { pub value: BigInt, pub citations: BTreeSet<String> }

/// Certified upper bounds of the input relations: rows, and per column the
/// distinct values and the most rows one value has. Indexed by declaration
/// and column, each with its fact id.
#[derive(Clone, Debug, Default)]
pub struct InputFacts {
    pub rows: BTreeMap<usize, (u64, String)>,
    pub distinct: BTreeMap<(usize, usize), (u64, String)>,
    pub degree: BTreeMap<(usize, usize), (u64, String)>,
}
impl InputFacts {
    /// The facts of the input relations the program's evidence attachment
    /// binds in full.
    pub fn bound(program: &dl::Program, evidence: &DatasetEvidence) -> export::Result<Self> {
        let attachment = program.declared_evidence.as_ref().ok_or_else(|| error("the program has no evidence attachment"))?;
        let mut facts = Self::default();
        for binding in attachment.bindings.iter().filter(|binding| binding.view == EvidenceView::Full) {
            let name = symbol_name(&binding.relation);
            let Some(relation) = program.declarations.iter().position(|declaration| symbol_name(&declaration.name) == name) else {
                return Err(error(format!("evidence binds undeclared relation {name}")));
            };
            if program.declarations[relation].value.is_none() { continue; }
            let recorded = evidence.relations.get(&binding.source)
                .ok_or_else(|| error("explicit evidence binding names an absent dataset relation"))?;
            if let Some(fact) = &recorded.rows {
                if let Some(&rows) = bounds::safe_upper(fact) { facts.rows.insert(relation, (rows, fact.id.clone())); }
            }
            for column in &binding.columns {
                let held = symbol_name(&column.column);
                let Some(at) = program.declarations[relation].columns.iter().position(|declared| symbol_name(&declared.name) == held) else {
                    return Err(error(format!("evidence binds undeclared column {name}.{held}")));
                };
                let recorded = recorded.columns.get(&column.source)
                    .ok_or_else(|| error("explicit evidence binding names an absent dataset column"))?;
                if let Some(fact) = &recorded.distinct {
                    if let Some(&distinct) = bounds::safe_upper(fact) { facts.distinct.insert((relation, at), (distinct, fact.id.clone())); }
                }
                if let Some(fact) = &recorded.max_degree {
                    if let Some(&degree) = bounds::safe_upper(fact) { facts.degree.insert((relation, at), (degree, fact.id.clone())); }
                }
            }
        }
        Ok(facts)
    }

    /// `D_e(j..)`: the exact distinct count for one column, else
    /// `min(U_e, prod_j D_e(j))`; under a key also `M_e(k)` for each key
    /// column; 1 for a constant.
    fn support(&self, program: &Inferred, source: &Source) -> Result<Certified, String> {
        let Source::Input { relation, columns, key } = source else {
            return Ok(Certified { value: BigInt::one(), citations: BTreeSet::new() });
        };
        let columns: BTreeSet<usize> = columns.iter().copied().collect();
        let single = |(count, id): &(u64, String)| Certified { value: BigInt::from(*count), citations: BTreeSet::from([id.clone()]) };
        let mut candidates: Vec<Certified> = self.rows.get(relation).map(single).into_iter().collect();
        let distinct: Option<Vec<&(u64, String)>> = columns.iter().map(|column| self.distinct.get(&(*relation, *column))).collect();
        if let Some(distinct) = distinct {
            candidates.push(Certified {
                value: distinct.iter().map(|(count, _)| BigInt::from(*count)).product(),
                citations: distinct.iter().map(|(_, id)| id.clone()).collect(),
            });
        }
        // The rows of one key value hold at most that many block values.
        candidates.extend(key.iter().filter_map(|column| self.degree.get(&(*relation, *column))).map(single));
        candidates.into_iter().min_by(|left, right| left.value.cmp(&right.value))
            .ok_or_else(|| format!("no certified row or distinct fact for input {}", program.names[*relation]))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferredColumn {
    pub column: String,
    /// Upper bound on distinct values, in decimal; absent when unknown.
    pub distinct: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferredRelation {
    pub relation: String,
    pub recursive: bool,
    /// Upper bound on rows, in decimal; absent when unknown.
    pub rows: Option<String>,
    pub unknown: Option<String>,
    pub columns: Vec<InferredColumn>,
    /// The cover form of all columns, one relaxed rule body per term.
    pub form: Vec<String>,
    pub citations: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferredReport {
    pub schema: String,
    pub input_hash: String,
    pub evidence_hash: String,
    pub dataset_hash: String,
    pub relations: Vec<InferredRelation>,
}

/// Structural bounds of every derived relation of a checked Datalog input,
/// from the exact facts of its bound inputs.
pub fn analyze(input: &PlannerInput, evidence: &DatasetEvidence) -> export::Result<InferredReport> {
    input.check()?;
    bounds::validate_dataset(input, evidence)?;
    let CheckedIr::Datalog { round: recorded } = &input.checked_ir else { return Err(error("structural bounds need a recorded Datalog Round")); };
    let program: round::Program = syn::parse_str(recorded)?;
    let source = &program.storage.incremental.strata.source;
    let facts = InputFacts::bound(source, evidence)?;
    let mut inferred = Inferred::new(source)?;
    let mut relations = Vec::new();
    let derived: Vec<usize> = (0..inferred.relations()).filter(|&relation| inferred.derived(relation)).collect();
    for relation in derived {
        let all: BTreeSet<usize> = (0..inferred.column_names(relation).len()).collect();
        let rows = inferred.support(&facts, relation, &all);
        let mut citations = rows.as_ref().map(|bound| bound.citations.clone()).unwrap_or_default();
        let columns = all.iter().map(|&column| {
            let distinct = inferred.support(&facts, relation, &BTreeSet::from([column])).ok();
            if let Some(bound) = &distinct { citations.extend(bound.citations.iter().cloned()); }
            InferredColumn { column: inferred.column_names(relation)[column].clone(), distinct: distinct.map(|bound| bound.value.to_string()) }
        }).collect();
        let form = inferred.form(relation, &all).clone();
        relations.push(InferredRelation {
            relation: inferred.name(relation).into(), recursive: inferred.recursive(relation),
            unknown: rows.as_ref().err().cloned(), rows: rows.ok().map(|bound| bound.value.to_string()),
            columns, form: inferred.render(relation, &form), citations,
        });
    }
    Ok(InferredReport {
        schema: SCHEMA.into(), input_hash: input.input_hash.clone(),
        evidence_hash: export::sha256(evidence.canonical_json().map_err(|e| error(e.to_string()))?),
        dataset_hash: evidence.dataset.hash.clone(), relations,
    })
}
