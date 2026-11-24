//! Structural bounds of derived relations against exact naive fixpoints.
//! The evaluator in `support` is independent of the analysis: it joins by
//! nested loops over explicit tuple sets.
use std::collections::{BTreeMap, BTreeSet};

use cq_cost::cost::inferred::{Inferred, InputFacts};
use cq_lang_datalog_round::incremental::source as dl;
use num_bigint::BigInt;

mod support;
use support::{Database, Program, Rule, atom, rule, Const, Var};

impl Program {
    fn analysis(&self) -> Inferred {
        let parsed: dl::Program = syn::parse_str(&self.source()).unwrap_or_else(|error| panic!("{error}\n{}", self.source()));
        Inferred::new(&parsed).unwrap()
    }
    /// Exact rows and per-column distinct counts of the inputs.
    fn facts(&self, inputs: &Database) -> InputFacts {
        let mut facts = InputFacts::default();
        for (name, arity) in &self.inputs {
            let relation = self.index(name);
            facts.rows.insert(relation, (inputs[name].len() as u64, format!("{name}.rows")));
            for column in 0..*arity {
                let distinct: BTreeSet<i64> = inputs[name].iter().map(|row| row[column]).collect();
                facts.distinct.insert((relation, column), (distinct.len() as u64, format!("{name}.c{column}.distinct")));
                let mut groups = BTreeMap::<i64, u64>::new();
                for row in &inputs[name] { *groups.entry(row[column]).or_default() += 1; }
                facts.degree.insert((relation, column), (groups.values().copied().max().unwrap_or(0), format!("{name}.c{column}.max_degree")));
            }
        }
        facts
    }
    /// Every certified support and degree bound of every derived relation is
    /// at least the exact count on this database.
    fn check(&self, analysis: &mut Inferred, inputs: &Database) {
        let database = self.evaluate(inputs);
        let facts = self.facts(inputs);
        for (name, arity) in &self.derived {
            let relation = self.index(name);
            for mask in 1..(1_u32 << arity) {
                let columns: BTreeSet<usize> = (0..*arity).filter(|column| mask & (1 << column) != 0).collect();
                let project = |row: &Vec<i64>, columns: &BTreeSet<usize>| columns.iter().map(|&column| row[column]).collect::<Vec<_>>();
                let exact: BTreeSet<Vec<i64>> = database[name].iter().map(|row| project(row, &columns)).collect();
                let bound = analysis.support(&facts, relation, &columns).unwrap();
                assert!(bound.value >= BigInt::from(exact.len()), "{name}{columns:?}: {} < {} on {inputs:?}", bound.value, exact.len());
                let mut groups = BTreeMap::<Vec<i64>, u64>::new();
                for row in &database[name] { *groups.entry(project(row, &columns)).or_default() += 1; }
                let degree = analysis.degree(&facts, relation, &columns).unwrap();
                let largest = groups.values().copied().max().unwrap_or(0);
                assert!(degree.value >= BigInt::from(largest), "{name} degree on {columns:?}: {} < {largest} on {inputs:?}", degree.value);
            }
        }
    }
    /// Every database whose inputs are subsets of all tuples over `domain`.
    fn exhaustive(&self, domain: i64) {
        let mut analysis = self.analysis();
        let universe = |arity: usize| -> Vec<Vec<i64>> {
            (0..domain.pow(arity as u32)).map(|code| (0..arity).map(|at| code / domain.pow(at as u32) % domain).collect()).collect()
        };
        let sizes: Vec<u32> = self.inputs.iter().map(|(_, arity)| domain.pow(*arity as u32) as u32).collect();
        let mut masks = vec![0_u64; self.inputs.len()];
        loop {
            let inputs: Database = self.inputs.iter().zip(&masks).map(|((name, arity), mask)|
                (*name, universe(*arity).into_iter().enumerate().filter(|(at, _)| mask & (1 << at) != 0).map(|(_, row)| row).collect())).collect();
            self.check(&mut analysis, &inputs);
            let mut at = 0;
            loop {
                if at == masks.len() { return; }
                masks[at] += 1;
                if masks[at] < 1 << sizes[at] { break; }
                masks[at] = 0; at += 1;
            }
        }
    }
    /// Deterministic pseudo-random databases over `domain`.
    fn sampled(&self, domain: i64, databases: usize) {
        let mut analysis = self.analysis();
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move || { state ^= state << 13; state ^= state >> 7; state ^= state << 17; state };
        for _ in 0..databases {
            let inputs: Database = self.inputs.iter().map(|(name, arity)| {
                let rows = next() % 9;
                (*name, (0..rows).map(|_| (0..*arity).map(|_| (next() % domain as u64) as i64).collect()).collect())
            }).collect();
            self.check(&mut analysis, &inputs);
        }
    }
}

fn closure() -> Program {
    Program { inputs: vec![("E", 2)], derived: vec![("T", 2)], rules: vec![
        rule(atom("T", &[Var("x"), Var("y")]), &[atom("E", &[Var("x"), Var("y")])]),
        rule(atom("T", &[Var("x"), Var("y")]), &[atom("T", &[Var("x"), Var("z")]), atom("E", &[Var("z"), Var("y")])]),
    ] }
}
/// A field-sensitive points-to analysis in miniature.
fn points_to() -> Program {
    Program { inputs: vec![("New", 2), ("Assign", 2), ("Store", 2), ("Load", 2)], derived: vec![("V", 2), ("F", 2)], rules: vec![
        rule(atom("V", &[Var("v"), Var("h")]), &[atom("New", &[Var("v"), Var("h")])]),
        rule(atom("V", &[Var("to"), Var("h")]), &[atom("Assign", &[Var("to"), Var("from")]), atom("V", &[Var("from"), Var("h")])]),
        rule(atom("F", &[Var("h"), Var("g")]), &[atom("Store", &[Var("base"), Var("v")]), atom("V", &[Var("base"), Var("h")]), atom("V", &[Var("v"), Var("g")])]),
        rule(atom("V", &[Var("to"), Var("g")]), &[atom("Load", &[Var("to"), Var("base")]), atom("V", &[Var("base"), Var("h")]), atom("F", &[Var("h"), Var("g")])]),
    ] }
}

#[test]
fn transitive_closure_has_the_two_adornments_of_the_paper() {
    let program = closure();
    let mut analysis = program.analysis();
    let relation = program.index("T");
    let form = analysis.form(relation, &BTreeSet::from([0, 1])).clone();
    assert!(form.unknown.is_none());
    // The source of one edge and the target of another, or one edge.
    assert_eq!(analysis.render(relation, &form), ["E(c0,_), E(_,c1)", "E(c0,c1)"]);
    // On a complete graph the bound is attained: it is a worst case, not slack.
    let complete: Database = BTreeMap::from([("E", (0..4).flat_map(|x| (0..4).map(move |y| vec![x, y])).collect())]);
    let exact = program.evaluate(&complete)["T"].len();
    let bound = analysis.support(&program.facts(&complete), relation, &BTreeSet::from([0, 1])).unwrap();
    assert_eq!((exact, bound.value.clone()), (16, BigInt::from(16)));
    assert_eq!(bound.citations, BTreeSet::from(["E.c0.distinct".to_string(), "E.c1.distinct".to_string()]));
    // On a path it is loose: 3 edges reach 6 pairs, the bound is 3 * 3.
    let path: Database = BTreeMap::from([("E", (0..3).map(|x| vec![x, x + 1]).collect())]);
    let bound = analysis.support(&program.facts(&path), relation, &BTreeSet::from([0, 1])).unwrap();
    assert_eq!((program.evaluate(&path)["T"].len(), bound.value), (6, BigInt::from(9)));
}

#[test]
fn a_copy_rule_adds_no_source_for_the_copied_column() {
    let program = points_to();
    let mut analysis = program.analysis();
    let relation = program.index("V");
    // Heap values of V come from allocations only, whatever flows through
    // assignments, loads and stores.
    let heap = analysis.form(relation, &BTreeSet::from([1])).clone();
    assert_eq!(analysis.render(relation, &heap), ["New(_,c1)"]);
    let variable = analysis.form(relation, &BTreeSet::from([0])).clone();
    assert_eq!(analysis.render(relation, &variable), ["{New(c0,_) | Assign(c0,_) | Load(c0,_)}"]);
    assert!(analysis.recursive(relation) && analysis.recursive(program.index("F")));
}

#[test]
fn bounds_hold_on_every_small_database() {
    closure().exhaustive(3);
    points_to().exhaustive(2);
    points_to().sampled(4, 400);
}

#[test]
fn bounds_hold_through_constants_repeated_variables_negation_and_mutual_recursion() {
    // Q repeats a variable and fixes a column; R reads both through Q.
    let shapes = Program { inputs: vec![("E", 2), ("N", 1)], derived: vec![("Q", 3), ("R", 2), ("S", 3)], rules: vec![
        rule(atom("Q", &[Var("x"), Var("x"), Const(7)]), &[atom("E", &[Var("x"), Var("y")])]),
        Rule(atom("Q", &[Var("y"), Var("x"), Var("y")]), vec![atom("E", &[Var("x"), Var("y")]), atom("E", &[Var("y"), Const(1)])], vec![atom("N", &[Var("x")])]),
        rule(atom("R", &[Var("a"), Var("w")]), &[atom("Q", &[Var("a"), Var("b"), Var("w")]), atom("E", &[Var("a"), Var("c")])]),
        rule(atom("S", &[Var("w"), Var("a"), Var("b")]), &[atom("Q", &[Var("a"), Var("b"), Var("w")]), atom("R", &[Var("b"), Var("w")])]),
    ] };
    shapes.exhaustive(2);
    shapes.sampled(4, 400);
    let parity = Program { inputs: vec![("Zero", 1), ("Next", 2)], derived: vec![("Even", 1), ("Odd", 1)], rules: vec![
        rule(atom("Even", &[Var("x")]), &[atom("Zero", &[Var("x")])]),
        rule(atom("Odd", &[Var("y")]), &[atom("Even", &[Var("x")]), atom("Next", &[Var("x"), Var("y")])]),
        rule(atom("Even", &[Var("y")]), &[atom("Odd", &[Var("x")]), atom("Next", &[Var("x"), Var("y")])]),
    ] };
    parity.exhaustive(2);
    parity.sampled(5, 400);
    let generation = Program { inputs: vec![("Flat", 2), ("Up", 2), ("Down", 2)], derived: vec![("Same", 2)], rules: vec![
        rule(atom("Same", &[Var("x"), Var("y")]), &[atom("Flat", &[Var("x"), Var("y")])]),
        rule(atom("Same", &[Var("x"), Var("y")]), &[atom("Up", &[Var("x"), Var("a")]), atom("Same", &[Var("a"), Var("b")]), atom("Down", &[Var("b"), Var("y")])]),
    ] };
    generation.exhaustive(2);
    generation.sampled(4, 400);
}

#[test]
fn a_computed_head_value_is_unknown_and_unknown_propagates() {
    let parsed: dl::Program = syn::parse_str(r#"
        struct P;
        relation E(c0:i64,c1:i64) = (e);
        relation Shifted(c0:i64,c1:i64);
        relation Reads(c0:i64);
        relation Keeps(c0:i64);
        output Shifted, Reads, Keeps;
        Shifted(*x + 1, *y) :- E(x, y);
        Reads(*z) :- Shifted(z, y);
        Keeps(*y) :- Shifted(x, y);
    "#).unwrap();
    let mut analysis = Inferred::new(&parsed).unwrap();
    let facts = InputFacts { rows: BTreeMap::from([(0, (5, "E.rows".into()))]),
        distinct: BTreeMap::from([((0, 0), (3, "E.c0.distinct".into())), ((0, 1), (2, "E.c1.distinct".into()))]), ..Default::default() };
    // The computed column and everything that copies it have no bound.
    assert!(analysis.support(&facts, 1, &BTreeSet::from([0])).unwrap_err().contains("computed"));
    assert!(analysis.support(&facts, 1, &BTreeSet::from([0, 1])).is_err());
    assert!(analysis.support(&facts, 2, &BTreeSet::from([0])).is_err());
    // The copied column keeps its bound, also through the derived relation.
    assert_eq!(analysis.support(&facts, 1, &BTreeSet::from([1])).unwrap().value, BigInt::from(2));
    assert_eq!(analysis.support(&facts, 3, &BTreeSet::from([0])).unwrap().value, BigInt::from(2));
}

#[test]
fn a_missing_input_fact_is_unknown_not_zero() {
    let program = closure();
    let mut analysis = program.analysis();
    let reason = analysis.support(&InputFacts::default(), program.index("T"), &BTreeSet::from([0, 1])).unwrap_err();
    assert!(reason.contains("no certified row or distinct fact for input E"), "{reason}");
}

/// Joined extends R through a lookup of S; Wide extends the derived Joined
/// through a lookup of U; Ends reads two columns of Wide.
fn lookups() -> Program {
    Program { inputs: vec![("R", 2), ("S", 2), ("U", 2)], derived: vec![("Joined", 3), ("Wide", 4), ("Ends", 2)], rules: vec![
        rule(atom("Joined", &[Var("a"), Var("b"), Var("c")]), &[atom("R", &[Var("a"), Var("b")]), atom("S", &[Var("b"), Var("c")])]),
        rule(atom("Wide", &[Var("a"), Var("b"), Var("c"), Var("d")]), &[atom("Joined", &[Var("a"), Var("b"), Var("c")]), atom("U", &[Var("c"), Var("d")])]),
        rule(atom("Ends", &[Var("d"), Var("a")]), &[atom("Wide", &[Var("a"), Var("b"), Var("c"), Var("d")])]),
    ] }
}

#[test]
fn a_lookup_of_an_input_multiplies_by_its_degree_not_its_support() {
    let program = lookups();
    let mut analysis = program.analysis();
    let joined = program.index("Joined");
    let form = analysis.form(joined, &BTreeSet::from([0, 1, 2])).clone();
    // S is read under the value of b that R supplied.
    assert_eq!(analysis.render(joined, &form), ["R(c0,c1), S([c1],c2)"]);
    // Twelve rows of R; each b has one c in S, among five distinct c.
    let inputs: Database = BTreeMap::from([
        ("R", (0..12).map(|a| vec![a, a % 5]).collect()), ("S", (0..5).map(|b| vec![b, b + 10]).collect()),
        ("U", (10..15).flat_map(|c| (0..2).map(move |d| vec![c, d])).collect()),
    ]);
    let facts = program.facts(&inputs);
    let bound = analysis.support(&facts, joined, &BTreeSet::from([0, 1, 2])).unwrap();
    assert_eq!((program.evaluate(&inputs)["Joined"].len(), bound.value.clone()), (12, BigInt::from(12)));
    assert!(bound.citations.contains("S.c0.max_degree") && bound.citations.contains("R.rows"));
    // The key carries through the derived relation: 12 * 1 * 2.
    let wide = analysis.support(&facts, program.index("Wide"), &BTreeSet::from([0, 1, 2, 3])).unwrap();
    assert_eq!((program.evaluate(&inputs)["Wide"].len(), wide.value), (24, BigInt::from(24)));
    program.exhaustive(2);
    program.sampled(4, 400);
}
