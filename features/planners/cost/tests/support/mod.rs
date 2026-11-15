//! Positive Datalog over integer tuples with a reference evaluator that is
//! independent of the cost tool: nested-loop joins over explicit tuple sets.
#![allow(dead_code)]
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy)]
pub enum Term { Var(&'static str), Const(i64) }
pub use Term::{Const, Var};
#[derive(Clone)]
pub struct Atom(pub &'static str, pub Vec<Term>);
/// Head, positive body, and atoms over inputs that must be absent.
#[derive(Clone)]
pub struct Rule(pub Atom, pub Vec<Atom>, pub Vec<Atom>);
pub struct Program { pub inputs: Vec<(&'static str, usize)>, pub derived: Vec<(&'static str, usize)>, pub rules: Vec<Rule> }
pub type Rows = BTreeSet<Vec<i64>>;
pub type Database = BTreeMap<&'static str, Rows>;

pub fn atom(relation: &'static str, arguments: &[Term]) -> Atom { Atom(relation, arguments.to_vec()) }
pub fn rule(head: Atom, body: &[Atom]) -> Rule { Rule(head, body.to_vec(), Vec::new()) }

/// One steady round of a recursive component: the rows new in it, and
/// every relation as the round reads it (the new rows included).
pub struct Round { pub delta: Database, pub full: Database }
/// A semi-naive run: the fixpoint, and per recursive component its rounds.
pub struct History {
    pub database: Database,
    /// The component of each derived relation.
    pub component: BTreeMap<&'static str, usize>,
    pub rounds: Vec<Vec<Round>>,
}

impl Program {
    pub fn relations(&self) -> impl Iterator<Item = (&'static str, usize)> + '_ { self.inputs.iter().chain(&self.derived).copied() }
    pub fn index(&self, name: &str) -> usize { self.relations().position(|(known, _)| known == name).unwrap() }
    /// The program text; `attachment` goes between declarations and outputs.
    pub fn source_with(&self, attachment: &str) -> String {
        let columns = |arity: usize| (0..arity).map(|column| format!("c{column}:i64")).collect::<Vec<_>>().join(",");
        let mut text = String::from("struct P;\n");
        for (name, arity) in &self.inputs { text += &format!("relation {name}({}) = ({});\n", columns(*arity), name.to_lowercase()); }
        for (name, arity) in &self.derived { text += &format!("relation {name}({});\n", columns(*arity)); }
        text += attachment;
        text += &format!("output {};\n", self.derived.iter().map(|(name, _)| *name).collect::<Vec<_>>().join(", "));
        let body = |atom: &Atom| format!("{}({})", atom.0, atom.1.iter().map(|term| match term {
            Var(name) => (*name).to_string(), Const(value) => value.to_string() }).collect::<Vec<_>>().join(", "));
        for (at, Rule(head, positive, negative)) in self.rules.iter().enumerate() {
            let arguments = head.1.iter().map(|term| match term { Var(name) => format!("*{name}"), Const(value) => value.to_string() });
            let clauses = positive.iter().map(&body).chain(negative.iter().map(|atom| format!("!{}", body(atom))));
            text += &format!("Rule{at}: {}({}) :- {};\n", head.0, arguments.collect::<Vec<_>>().join(", "), clauses.collect::<Vec<_>>().join(", "));
        }
        text
    }
    pub fn source(&self) -> String { self.source_with("") }

    /// The head rows one rule derives when body atom `at` reads `read(at)`.
    /// A large prefix looks rows up by its bound variables; the match test
    /// is the same either way.
    fn derive<'a>(&self, Rule(head, positive, negative): &Rule, database: &Database, read: impl Fn(usize) -> &'a Rows) -> Vec<Vec<i64>> {
        let mut bindings = vec![BTreeMap::<&str, i64>::new()];
        let mut bound = BTreeSet::<&str>::new();
        for (at, Atom(_, arguments)) in positive.iter().enumerate() {
            let keyed: Vec<(usize, &str)> = arguments.iter().enumerate()
                .filter_map(|(position, term)| match term { Var(name) if bound.contains(name) => Some((position, *name)), _ => None }).collect();
            let mut index = std::collections::HashMap::<Vec<i64>, Vec<&Vec<i64>>>::new();
            let indexed = bindings.len() > 16 && !keyed.is_empty();
            if indexed {
                for row in read(at) { index.entry(keyed.iter().map(|&(position, _)| row[position]).collect()).or_default().push(row); }
            }
            let mut extended = Vec::new();
            for binding in &bindings {
                let rows: Box<dyn Iterator<Item = &Vec<i64>>> = if indexed {
                    Box::new(index.get(&keyed.iter().map(|&(_, name)| binding[name]).collect::<Vec<_>>()).into_iter().flatten().copied())
                } else { Box::new(read(at).iter()) };
                for row in rows {
                    let mut next = binding.clone();
                    if arguments.iter().zip(row).all(|(term, value)| match term {
                        Const(expected) => expected == value,
                        Var(name) => *next.entry(name).or_insert(*value) == *value,
                    }) { extended.push(next); }
                }
            }
            bindings = extended;
            bound.extend(arguments.iter().filter_map(|term| match term { Var(name) => Some(*name), Const(_) => None }));
        }
        let value = |binding: &BTreeMap<&str, i64>, term: &Term| match term { Var(name) => binding[name], Const(value) => *value };
        bindings.iter().filter(|binding| !negative.iter().any(|Atom(relation, arguments)|
            database[relation].contains(&arguments.iter().map(|term| value(binding, term)).collect::<Vec<_>>())))
            .map(|binding| head.1.iter().map(|term| value(binding, term)).collect()).collect()
    }

    /// The least fixpoint by naive iteration.
    pub fn evaluate(&self, inputs: &Database) -> Database {
        let mut database = inputs.clone();
        for (name, _) in &self.derived { database.insert(name, Rows::new()); }
        loop {
            let mut changed = false;
            for rule in &self.rules {
                for row in self.derive(rule, &database, |at| &database[rule.1[at].0]) {
                    changed |= database.get_mut(rule.0.0).unwrap().insert(row);
                }
            }
            if !changed { return database; }
        }
    }

    /// The least fixpoint by stratified semi-naive evaluation, with the
    /// rounds of every recursive component.
    pub fn semi_naive(&self, inputs: &Database) -> History {
        let derived: Vec<&'static str> = self.derived.iter().map(|(name, _)| *name).collect();
        let reads = |name: &str| -> BTreeSet<&'static str> {
            self.rules.iter().filter(|rule| rule.0.0 == name).flat_map(|rule| rule.1.iter().map(|atom| atom.0))
                .filter(|read| derived.contains(read)).collect()
        };
        let reaches = |name: &'static str| -> BTreeSet<&'static str> {
            let mut seen = BTreeSet::new();
            let mut stack = vec![name];
            while let Some(next) = stack.pop() { for read in reads(next) { if seen.insert(read) { stack.push(read); } } }
            seen
        };
        // Components in dependency order: every relation a component reads
        // outside itself is complete before it starts.
        let mut pending = derived.clone();
        let mut database = inputs.clone();
        for name in &derived { database.insert(name, Rows::new()); }
        let mut history = History { database: Database::new(), component: BTreeMap::new(), rounds: Vec::new() };
        while !pending.is_empty() {
            let name = *pending.iter().find(|name| reaches(name).iter().all(|read| !pending.contains(read) || reaches(read).contains(*name)))
                .expect("stratifiable program");
            let members: Vec<&'static str> = pending.iter().copied()
                .filter(|other| *other == name || (reaches(name).contains(other) && reaches(other).contains(name))).collect();
            pending.retain(|other| !members.contains(other));
            let recursive = |rule: &Rule| rule.1.iter().any(|atom| members.contains(&atom.0));
            for rule in self.rules.iter().filter(|rule| members.contains(&rule.0.0) && !recursive(rule)) {
                for row in self.derive(rule, &database, |at| &database[rule.1[at].0]) { database.get_mut(rule.0.0).unwrap().insert(row); }
            }
            let mut rounds = Vec::new();
            let mut delta: Database = members.iter().map(|member| (*member, database[member].clone())).collect();
            while self.rules.iter().any(|rule| members.contains(&rule.0.0) && recursive(rule)) && delta.values().any(|rows| !rows.is_empty()) {
                let mut next: Database = members.iter().map(|member| (*member, Rows::new())).collect();
                for rule in self.rules.iter().filter(|rule| members.contains(&rule.0.0) && recursive(rule)) {
                    for changing in (0..rule.1.len()).filter(|&at| members.contains(&rule.1[at].0)) {
                        for row in self.derive(rule, &database, |at| if at == changing { &delta[rule.1[at].0] } else { &database[rule.1[at].0] }) {
                            if !database[rule.0.0].contains(&row) { next.get_mut(rule.0.0).unwrap().insert(row); }
                        }
                    }
                }
                rounds.push(Round { delta, full: database.clone() });
                for (member, rows) in &next { database.get_mut(member).unwrap().extend(rows.iter().cloned()); }
                delta = next;
            }
            for member in &members { history.component.insert(member, history.rounds.len()); }
            history.rounds.push(rounds);
        }
        history.database = database;
        history
    }
}
