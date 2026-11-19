//! The tool's choice against the optimal order on the DOOP default program.
//!
//! An ignored measurement, not part of the suite:
//!
//! ```text
//! DOOP_PREPARED=<prepared dataset> [DOOP_EXAMPLE=<prepared dataset>] \
//!   [DOOP_ORACLE_OUT=<report.tsv>] cargo test --release -p cq-cost \
//!   --test doop_order -- --ignored --nocapture
//! ```
//!
//! A reference evaluator, independent of the cost model, runs the program by
//! stratified semi-naive evaluation and records the round in which each
//! tuple was admitted. Every candidate valuation of every rule variant is
//! then counted exactly, round by round, from those labels: in round `r` a
//! variant reads the tuples of its component admitted before `r` and, at its
//! changing occurrence, those admitted in round `r - 1`. The planner sees
//! only input facts and a profile of the example dataset's run (the planned
//! dataset itself by default, so every derived-relation fact is exact).
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};
use std::path::Path;

use cq_cost::cost::{self, estimate, Candidate, IndexVersion, Space};
use cq_cost::export::{self, Configuration, Origin, PlannerInput, Target, Version};
use cq_evidence::{self as ev, DatasetEvidence, RelationSemantics, ScalarDomain};
use cq_lang_cq::BodyItem;
use cq_lang_datalog_round::incremental::source as dl;
use cq_load::{stats::{ColumnSpec, DistinctMode, RelationSpec}, stats_manifest::{DatasetRequest, MetadataInput, RelationInput, REQUEST_SCHEMA}, stats_source::{SourceFormat, SourceOptions}};
use cq_syntax::{constraint::{Comparison, Constraint, Term}, declared_evidence::{ColumnBinding, DeclaredEvidence, EvidenceView, RelationBinding}, rust, symbol_name};
use quote::ToTokens;

/// Widest DOOP relation.
const W: usize = 5;
type Row = [i32; W];

#[derive(Default, Clone, Copy)]
struct Fx(u64);
impl Hasher for Fx {
    fn finish(&self) -> u64 { self.0 }
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks { self.write_u64(u64::from_le_bytes(chunk.try_into().unwrap())); }
        for &byte in chunks.remainder() { self.write_u64(byte as u64); }
    }
    fn write_u32(&mut self, value: u32) { self.write_u64(value as u64); }
    fn write_i32(&mut self, value: i32) { self.write_u64(value as u32 as u64); }
    fn write_usize(&mut self, value: usize) { self.write_u64(value as u64); }
    fn write_u64(&mut self, value: u64) { self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95); }
}
type Fast = BuildHasherDefault<Fx>;

/// One relation: its rows, the round each was admitted (0 for seeds and for
/// relations outside a recursive component), and indexes by column sets.
#[derive(Default)]
struct Table { rows: Vec<Row>, stage: Vec<u32>, seen: HashSet<Row, Fast>, indexes: HashMap<Vec<usize>, HashMap<Row, Vec<u32>, Fast>> }
fn key(row: &Row, columns: &[usize]) -> Row {
    let mut key = [0; W];
    for (at, &column) in columns.iter().enumerate() { key[at] = row[column]; }
    key
}
impl Table {
    fn insert(&mut self, row: Row, stage: u32) -> bool {
        if !self.seen.insert(row) { return false; }
        let id = self.rows.len() as u32;
        self.rows.push(row);
        self.stage.push(stage);
        for (columns, index) in &mut self.indexes { index.entry(key(&row, columns)).or_default().push(id); }
        true
    }
    fn index(&mut self, columns: &[usize]) {
        if self.indexes.contains_key(columns) { return; }
        let mut index: HashMap<Row, Vec<u32>, Fast> = HashMap::default();
        for (id, row) in self.rows.iter().enumerate() { index.entry(key(row, columns)).or_default().push(id as u32); }
        self.indexes.insert(columns.to_vec(), index);
    }
    fn matching(&self, columns: &[usize], probe: &Row) -> &[u32] {
        self.indexes[columns].get(probe).map_or(&[], Vec::as_slice)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Arg { Var(usize), Const(i32), Any }
/// A comparison of a constraint.
#[derive(Clone, Copy, Debug)]
enum Op2 { Eq, Ne, Lt, Le, Gt, Ge }
#[derive(Clone, Debug)]
enum Cond { Pos(usize, Vec<Arg>), Neg(usize, Vec<Arg>), Cmp(Arg, Op2, Arg), Or(Vec<Vec<Cond>>) }
/// `variables` counts the rule's distinct variables, at most a frame's width.
struct Rule { head: usize, head_args: Vec<Arg>, body: Vec<Cond>, variables: usize }

/// The meta.json key of each `constants.c_*` field, read from the generated loader.
fn constant_keys(generated: &str) -> BTreeMap<String, String> {
    let flat: String = generated.split_whitespace().collect::<Vec<_>>().join("");
    let mut keys = BTreeMap::new();
    let mut rest = flat.as_str();
    while let Some(at) = rest.find(":*meta.get(\"") {
        let before = &rest[..at];
        let field = &before[before.rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(0, |at| at + 1)..];
        let after = &rest[at + ":*meta.get(\"".len()..];
        let end = after.find('"').unwrap();
        keys.insert(field.to_string(), after[..end].to_string());
        rest = &after[end..];
    }
    keys
}
/// The value of a closed expression: a `constants.c_*` field or an integer literal.
fn closed(expression: &syn::Expr, constants: &BTreeMap<String, i32>) -> i32 {
    let text = expression.to_token_stream().to_string().replace(' ', "");
    let text = text.trim_start_matches('(').trim_end_matches(')');
    if let Some(field) = text.strip_prefix("constants.") { return constants[field]; }
    text.parse().unwrap_or_else(|_| panic!("unsupported closed expression {text}"))
}

struct Model { names: Vec<String>, arity: Vec<usize>, input: Vec<bool>, rules: Vec<Rule>, constants: BTreeMap<String, i32> }
impl Model {
    fn new(program: &dl::Program, constants: BTreeMap<String, i32>) -> Self {
        let names: Vec<String> = program.declarations.iter().map(|declaration| symbol_name(&declaration.name)).collect();
        let index = |name: &syn::Ident| names.iter().position(|known| *known == symbol_name(name)).unwrap();
        let mut rules = Vec::new();
        for query in &program.rules {
            let variables = std::cell::RefCell::new(Vec::<String>::new());
            let var = |name: String| -> usize {
                let mut variables = variables.borrow_mut();
                if let Some(at) = variables.iter().position(|known| *known == name) { return at; }
                variables.push(name);
                variables.len() - 1
            };
            let arg = |expression: &syn::Expr| -> Arg {
                let mut inner = expression;
                while let syn::Expr::Unary(syn::ExprUnary { op: syn::UnOp::Deref(_), expr, .. }) = inner { inner = expr; }
                if let Some(name) = rust::variable(inner) { return Arg::Var(var(symbol_name(name))); }
                if matches!(inner, syn::Expr::Infer(_)) { return Arg::Any; }
                Arg::Const(closed(inner, &constants))
            };
            let mut body = Vec::new();
            for item in query.body.iter() {
                match item {
                    BodyItem::Positive { atom } => body.push(Cond::Pos(index(&atom.relation), atom.args.iter().map(arg).collect())),
                    BodyItem::Negation(negation) => body.push(Cond::Neg(index(&negation.atom.relation), negation.atom.args.iter().map(arg).collect())),
                    BodyItem::Constraint(constraint) => body.push(condition(constraint, &mut |term| match term {
                        Term::Variable(name) => Arg::Var(var(symbol_name(name))),
                        Term::Closed(expression) => Arg::Const(closed(expression, &constants)),
                        Term::Literal(expression) => Arg::Const(closed(expression, &constants)),
                    })),
                    other => panic!("unsupported body item {}", other.to_token_stream()),
                }
            }
            let head_args = query.head.args.iter().map(arg).collect();
            let variables = variables.borrow().len();
            rules.push(Rule { head: index(&query.head.relation), head_args, body, variables });
        }
        Self { arity: program.declarations.iter().map(|declaration| declaration.columns.len()).collect(),
            input: program.declarations.iter().map(|declaration| declaration.value.is_some()).collect(), names, rules, constants }
    }
}
fn condition(constraint: &Constraint, term: &mut dyn FnMut(&Term) -> Arg) -> Cond {
    match constraint {
        Constraint::Compare { left, comparison, right } => Cond::Cmp(term(left), match comparison {
            Comparison::Eq(_) => Op2::Eq, Comparison::Ne(_) => Op2::Ne, Comparison::Lt(_) => Op2::Lt,
            Comparison::Le(_) => Op2::Le, Comparison::Gt(_) => Op2::Gt, Comparison::Ge(_) => Op2::Ge,
        }, term(right)),
        Constraint::Or(branches) => Cond::Or(branches.iter().map(|branch| branch.iter().map(|part| condition(part, term)).collect()).collect()),
        other => panic!("unsupported constraint {other:?}"),
    }
}
fn compare(left: i32, comparison: Op2, right: i32) -> bool {
    match comparison {
        Op2::Eq => left == right, Op2::Ne => left != right, Op2::Lt => left < right,
        Op2::Le => left <= right, Op2::Gt => left > right, Op2::Ge => left >= right,
    }
}

/// The evaluated database: every relation with admission rounds, the
/// recursive component of each derived relation, and the rounds each
/// component executed.
struct Run { tables: Vec<Table>, component: Vec<Option<usize>>, rounds: Vec<u32> }

/// Variables bound so far: value and whether bound.
type Frame = [Option<i32>; 64];
fn value(frame: &Frame, arg: Arg) -> Option<i32> {
    match arg { Arg::Var(at) => frame[at], Arg::Const(value) => Some(value), Arg::Any => None }
}
fn holds(cond: &Cond, frame: &Frame) -> Option<bool> {
    match cond {
        Cond::Cmp(left, comparison, right) => Some(compare(value(frame, *left)?, *comparison, value(frame, *right)?)),
        Cond::Or(branches) => {
            let mut any = false;
            for branch in branches {
                let mut all = true;
                for part in branch { all &= holds(part, frame)?; }
                any |= all;
            }
            Some(any)
        }
        _ => unreachable!(),
    }
}

impl Run {
    /// Semi-naive evaluation, component by component in dependency order.
    fn evaluate(model: &Model, inputs: BTreeMap<usize, Vec<Row>>) -> Self {
        let count = model.names.len();
        let mut tables: Vec<Table> = (0..count).map(|_| Table::default()).collect();
        for (relation, rows) in inputs { for row in rows { tables[relation].insert(row, 0); } }
        let heads: Vec<Vec<usize>> = (0..count).map(|relation| (0..model.rules.len()).filter(|&rule| model.rules[rule].head == relation).collect()).collect();
        let reads = |relation: usize| -> BTreeSet<usize> {
            heads[relation].iter().flat_map(|&rule| model.rules[rule].body.iter().filter_map(|cond| match cond {
                Cond::Pos(read, _) | Cond::Neg(read, _) => Some(*read), _ => None })).filter(|read| !heads[*read].is_empty()).collect()
        };
        let reaches = |relation: usize| -> BTreeSet<usize> {
            let mut seen = BTreeSet::new();
            let mut stack = vec![relation];
            while let Some(next) = stack.pop() { for read in reads(next) { if seen.insert(read) { stack.push(read); } } }
            seen
        };
        let reach: Vec<BTreeSet<usize>> = (0..count).map(reaches).collect();
        let mut pending: Vec<usize> = (0..count).filter(|&relation| !heads[relation].is_empty()).collect();
        let mut component = vec![None; count];
        let mut rounds = Vec::new();
        while !pending.is_empty() {
            let first = *pending.iter().find(|&&relation| reach[relation].iter().all(|read| !pending.contains(read) || reach[*read].contains(&relation))).expect("stratified");
            let members: Vec<usize> = pending.iter().copied().filter(|&other| other == first || (reach[first].contains(&other) && reach[other].contains(&first))).collect();
            pending.retain(|other| !members.contains(other));
            let id = rounds.len();
            for &member in &members { component[member] = Some(id); }
            let rules: Vec<usize> = members.iter().flat_map(|&member| heads[member].iter().copied()).collect();
            let recursive = |rule: usize| model.rules[rule].body.iter().any(|cond| matches!(cond, Cond::Pos(read, _) if members.contains(read)));
            // Seeds, once.
            let seeds: Vec<Plan> = rules.iter().filter(|&&rule| !recursive(rule)).map(|&rule| plan(model, &mut tables, rule, None)).collect();
            for (relation, row) in execute_all(model, &tables, &seeds, &members, 0) { tables[relation].insert(row, 0); }
            let mut round = 1;
            while rules.iter().any(|&rule| recursive(rule)) && members.iter().any(|&member| tables[member].stage.last() == Some(&(round - 1))) {
                let mut plans = Vec::new();
                for &rule in rules.iter().filter(|&&rule| recursive(rule)) {
                    let positions: Vec<usize> = model.rules[rule].body.iter().enumerate()
                        .filter(|(_, cond)| matches!(cond, Cond::Pos(read, _) if members.contains(read))).map(|(at, _)| at).collect();
                    for at in positions { plans.push(plan(model, &mut tables, rule, Some(at))); }
                }
                let started = std::time::Instant::now();
                let found = execute_all(model, &tables, &plans, &members, round);
                if std::env::var_os("DOOP_ORACLE_TRACE").is_some() && started.elapsed().as_secs_f64() > 0.5 {
                    eprintln!("  round {round}: {} derived in {:.1?}", found.len(), started.elapsed());
                }
                for (relation, row) in found { tables[relation].insert(row, round); }
                round += 1;
            }
            rounds.push(round - 1);
            if std::env::var_os("DOOP_ORACLE_TRACE").is_some() {
                eprintln!("component {:?}: rounds={} rows={:?}", members.iter().map(|&m| &model.names[m]).collect::<Vec<_>>(), round - 1,
                    members.iter().map(|&m| tables[m].rows.len()).collect::<Vec<_>>());
            }
        }
        Self { tables, component, rounds }
    }
}

/// How one rule, `changing` at a body position or not, is derived: the
/// changing atom first, then the atom with the most bound columns; each
/// check where its variables are first bound.
struct Plan { rule: usize, changing: Option<usize>, order: Vec<usize>, keys: Vec<Vec<usize>>, checks: Vec<Vec<usize>>, negated: Vec<Vec<usize>> }
fn plan(model: &Model, tables: &mut [Table], rule: usize, changing: Option<usize>) -> Plan {
    let rule_ = &model.rules[rule];
    assert!(rule_.variables <= 64, "rule {rule} has more variables than a frame holds");
    let vars = |at: usize| -> Vec<usize> {
        let args = match &rule_.body[at] { Cond::Pos(_, args) | Cond::Neg(_, args) => args.clone(), Cond::Cmp(left, _, right) => vec![*left, *right], Cond::Or(branches) => {
            fn flat(cond: &Cond, into: &mut Vec<Arg>) { match cond { Cond::Cmp(l, _, r) => into.extend([*l, *r]), Cond::Or(bs) => for b in bs { for c in b { flat(c, into) } }, _ => {} } }
            let mut into = Vec::new(); for branch in branches { for cond in branch { flat(cond, &mut into); } } into
        } };
        args.iter().filter_map(|arg| match arg { Arg::Var(v) => Some(*v), _ => None }).collect()
    };
    let mut order: Vec<usize> = Vec::new();
    let mut bound_vars = BTreeSet::new();
    let mut left: Vec<usize> = rule_.body.iter().enumerate().filter(|(_, cond)| matches!(cond, Cond::Pos(..))).map(|(at, _)| at).collect();
    if let Some(at) = changing { order.push(at); left.retain(|&other| other != at); bound_vars.extend(vars(at)); }
    while !left.is_empty() {
        let next = *left.iter().max_by_key(|&&at| {
            let Cond::Pos(relation, args) = &rule_.body[at] else { unreachable!() };
            let fixed = args.iter().filter(|arg| match arg { Arg::Var(v) => bound_vars.contains(v), Arg::Const(_) => true, Arg::Any => false }).count();
            (fixed, std::cmp::Reverse(tables[*relation].rows.len()))
        }).unwrap();
        left.retain(|&other| other != next);
        bound_vars.extend(vars(next));
        order.push(next);
    }
    let mut keys = Vec::new();
    let mut checks = vec![Vec::new(); order.len() + 1];
    let mut known = BTreeSet::new();
    let mut placed = BTreeSet::new();
    for depth in 0..=order.len() {
        for (at, cond) in rule_.body.iter().enumerate() {
            if matches!(cond, Cond::Pos(..)) || placed.contains(&at) || !vars(at).iter().all(|v| known.contains(v)) { continue; }
            placed.insert(at);
            checks[depth].push(at);
        }
        let Some(&at) = order.get(depth) else { break };
        let Cond::Pos(relation, args) = &rule_.body[at] else { unreachable!() };
        let columns: Vec<usize> = args.iter().enumerate().filter(|(_, arg)| match arg { Arg::Var(v) => known.contains(v), Arg::Const(_) => true, Arg::Any => false }).map(|(c, _)| c).collect();
        tables[*relation].index(&columns);
        known.extend(vars(at));
        keys.push(columns);
    }
    let negated = rule_.body.iter().map(|cond| match cond {
        Cond::Neg(relation, args) => {
            let columns: Vec<usize> = args.iter().enumerate().filter(|(_, arg)| !matches!(arg, Arg::Any)).map(|(c, _)| c).collect();
            tables[*relation].index(&columns);
            columns
        }
        _ => Vec::new(),
    }).collect();
    Plan { rule, changing, order, keys, checks, negated }
}

/// New head rows of every plan, in parallel; the tables do not change.
fn execute_all(model: &Model, tables: &[Table], plans: &[Plan], members: &[usize], round: u32) -> Vec<(usize, Row)> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let found = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| for _ in 0..24 { scope.spawn(|| {
        let mut local = Vec::new();
        loop {
            let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let Some(plan) = plans.get(at) else { break };
            let started = std::time::Instant::now();
            let before = local.len();
            execute(model, tables, plan, members, round, &mut local);
            if std::env::var_os("DOOP_ORACLE_TRACE").is_some() && started.elapsed().as_secs_f64() > 1.0 {
                eprintln!("    slow plan: rule {} changing {:?} order {:?} keys {:?}: {} rows in {:.1?}", plan.rule, plan.changing, plan.order, plan.keys, local.len() - before, started.elapsed());
            }
        }
        found.lock().unwrap().append(&mut local);
    }); });
    found.into_inner().unwrap()
}

/// Derive one plan: its changing atom reads only the previous round's
/// tuples of the component, every other component atom those admitted
/// before this round.
fn execute(model: &Model, tables: &[Table], plan: &Plan, members: &[usize], round: u32, admitted: &mut Vec<(usize, Row)>) {
    let rule = &model.rules[plan.rule];
    fn walk(tables: &[Table], rule: &Rule, plan: &Plan, depth: usize, frame: &mut Frame, members: &[usize], round: u32, admitted: &mut Vec<(usize, Row)>) {
        for &at in &plan.checks[depth] {
            let pass = match &rule.body[at] {
                Cond::Neg(relation, args) => {
                    let mut probe = [0; W];
                    for (slot, &c) in plan.negated[at].iter().enumerate() { probe[slot] = value(frame, args[c]).unwrap(); }
                    tables[*relation].matching(&plan.negated[at], &probe).is_empty()
                }
                other => holds(other, frame).unwrap(),
            };
            if !pass { return; }
        }
        let Some(&at) = plan.order.get(depth) else {
            let mut row = [0; W];
            for (c, arg) in rule.head_args.iter().enumerate() { row[c] = value(frame, *arg).unwrap(); }
            if !tables[rule.head].seen.contains(&row) { admitted.push((rule.head, row)); }
            return;
        };
        let Cond::Pos(relation, args) = &rule.body[at] else { unreachable!() };
        let mut probe = [0; W];
        for (slot, &c) in plan.keys[depth].iter().enumerate() { probe[slot] = value(frame, args[c]).unwrap(); }
        let in_component = members.contains(relation);
        for &id in tables[*relation].matching(&plan.keys[depth], &probe) {
            let stage = tables[*relation].stage[id as usize];
            if in_component && (if Some(at) == plan.changing { stage + 1 != round } else { round > 0 && stage >= round }) { continue; }
            let row = &tables[*relation].rows[id as usize];
            let mut assigned = 0_u64;
            let mut ok = true;
            for (c, arg) in args.iter().enumerate() {
                if let Arg::Var(v) = arg {
                    match frame[*v] { Some(existing) => if existing != row[c] { ok = false; break; }, None => { frame[*v] = Some(row[c]); assigned |= 1 << v; } }
                }
            }
            if ok { walk(tables, rule, plan, depth + 1, frame, members, round, admitted); }
            while assigned != 0 { let v = assigned.trailing_zeros() as usize; frame[v] = None; assigned &= assigned - 1; }
        }
    }
    let mut frame: Frame = [None; 64];
    walk(tables, rule, plan, 0, &mut frame, members, round, admitted);
}

/// The DOOP default program, its declared constants and inputs.
struct Doop { program: dl::Program, generated: String, inputs: serde_json::Map<String, serde_json::Value> }
impl Doop {
    /// The program, after `rewrite` of its source text.
    fn load(rewrite: impl Fn(&str) -> String) -> Self {
        let srd = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../datalog/doop/srd/src");
        let schema: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(srd.join("generated_default.schema.json")).unwrap()).unwrap();
        let generated = rewrite(&std::fs::read_to_string(srd.join("generated_default.rs")).unwrap());
        let lines: Vec<&str> = generated.lines().collect();
        let first = lines.iter().position(|line| line.trim() == "struct SrdDoop;").unwrap();
        let last = first + lines[first..].iter().position(|line| line.trim() == "};").unwrap();
        let program = syn::parse_str(&lines[first..last].join("\n")).unwrap();
        Self { program, inputs: schema["inputs"].as_object().unwrap().clone(), generated }
    }
    fn input_name(&self, relation: &str) -> &str {
        self.inputs.keys().find(|name| format!("r_{}", name.to_lowercase()) == relation).unwrap_or_else(|| panic!("no input file for {relation}"))
    }
    fn constants(&self, dataset: &Path) -> BTreeMap<String, i32> {
        let meta: BTreeMap<String, i32> = serde_json::from_str(&std::fs::read_to_string(dataset.join("meta.json")).unwrap()).unwrap();
        constant_keys(&self.generated).into_iter().map(|(field, key)| (field, meta[&key])).collect()
    }
    fn rows(&self, dataset: &Path, model: &Model) -> BTreeMap<usize, Vec<Row>> {
        let mut rows = BTreeMap::new();
        for (relation, name) in model.names.iter().enumerate().filter(|(relation, _)| model.input[*relation]) {
            let file = self.inputs[self.input_name(name)]["file"].as_str().unwrap();
            let text = std::fs::read_to_string(dataset.join(file)).unwrap();
            rows.insert(relation, text.lines().filter(|line| !line.is_empty()).map(|line| {
                let mut row = [0; W];
                for (c, field) in line.split('\t').enumerate() { row[c] = field.parse().unwrap(); }
                row
            }).collect());
        }
        rows
    }
    /// The statistics request of the prepared inputs and the constants file.
    fn request(&self) -> DatasetRequest {
        DatasetRequest {
            schema: REQUEST_SCHEMA.into(), name: "doop".into(), date: "2026-11-11".into(), formulation: "doop default".into(),
            hash_mode: ev::HASH_MODE.into(), distinct: DistinctMode::Exact,
            metadata: vec![MetadataInput { path: "meta.json".into(), role: ev::FileRole::Constants }],
            relations: self.inputs.iter().map(|(name, input)| RelationInput {
                relation: RelationSpec { name: name.clone(), semantics: RelationSemantics::Set, key_sets: Vec::new(),
                    columns: (0..input["arity"].as_u64().unwrap()).map(|c| ColumnSpec { name: format!("c{c}"), domain: ScalarDomain::SignedInteger }).collect() },
                path: input["file"].as_str().unwrap().into(),
                options: SourceOptions { format: SourceFormat::LiteralTabs, delimiter: b'\t', headers: false, trailing_delimiter: false, ..Default::default() },
                missing_as_empty: false,
            }).collect(),
        }
    }
    /// Exact evidence of the inputs and the checked compiler record bound to it.
    fn planned(&self, dataset: &Path) -> (PlannerInput, DatasetEvidence) {
        let request = self.request();
        let profiling = std::time::Instant::now();
        let mut evidence = request.collect(dataset).unwrap();
        eprintln!("input evidence of {} relations collected in {:.2?}", request.relations.len(), profiling.elapsed());
        let configuration = Configuration { compiler_hash: export::sha256("doop optimal-order check"), target: Target::Cpu, invocation: "doop_order.rs".into() };
        let unit = |id: &str| ev::Fact { id: id.into(), source: ev::Source::Declared, claim: ev::Claim::Equal,
            value: Some(ev::Rational { numerator: "1".into(), denominator: "1".into() }), lower: None, upper: None, dataset_hash: evidence.dataset.hash.clone(),
            unit: ev::Unit::DeclaredRowWork, provenance: ev::Provenance { producer: "doop_order.rs".into(), method: "explicit h=m=1 row work".into(), locator: "doop_order.rs".into(), run: None, artifact_hash: None } };
        // VP-Index: 16 bindings a group, measured on bloat (PLANNING-REDESIGN).
        let group = ev::Fact { value: Some(ev::Rational { numerator: "16".into(), denominator: "1".into() }),
            provenance: ev::Provenance { producer: "doop_order.rs".into(), method: "one grouped index of the CPU engine on bloat, 2026-11-17: 250 ns a group against 16 ns a binding".into(), locator: "doc/PLANNING-REDESIGN.md VP-Index".into(), run: None, artifact_hash: None },
            ..unit("cpu.g") };
        evidence.targets.insert("cpu".into(), ev::TargetEvidence { unit: ev::Unit::DeclaredRowWork, index_per_group_g: Some(group),
            configuration_hash: export::sha256(serde_json::to_vec(&configuration).unwrap()), build_per_row_h: unit("cpu.h"), merge_per_row_m: unit("cpu.m") });
        for (field, value) in self.constants(dataset) {
            evidence.constants.insert(field.clone(), ev::Fact { id: format!("constant:{field}"), source: ev::Source::Declared, claim: ev::Claim::Equal,
                value: Some(ev::Scalar::SignedInteger { value: value.into() }), lower: None, upper: None, dataset_hash: evidence.dataset.hash.clone(),
                unit: ev::Unit::Scalar, provenance: ev::Provenance { producer: "doop_order.rs".into(), method: "the dataset's meta.json constant".into(), locator: "meta.json".into(), run: None, artifact_hash: None } });
        }
        let mut source = self.program.clone();
        let bindings = source.declarations.iter().filter(|declaration| declaration.value.is_some()).map(|declaration| {
            RelationBinding { relation: declaration.name.clone(), source: self.input_name(&declaration.name.to_string()).to_string(), view: EvidenceView::Full,
                columns: declaration.columns.iter().map(|column| ColumnBinding { column: column.name.clone(), source: column.name.to_string() }).collect() }
        }).collect();
        source.declared_evidence = Some(DeclaredEvidence::from_json(&evidence.canonical_json().unwrap(), bindings).unwrap());
        // `DOOP_DRIVER=delta_first` compiles as `srd-doop-gpu` does, so the
        // written order is the one the GPU runs; the default keeps the text order.
        let driver = match std::env::var("DOOP_DRIVER").as_deref() {
            Ok("delta_first") => cq_datalog_incremental::DriverPolicy::DeltaFirst,
            _ => cq_datalog_incremental::DriverPolicy::WrittenOrder,
        };
        (export::compile_datalog_with_driver(&source.to_token_stream().to_string(), configuration, driver).unwrap(), evidence)
    }
}

/// What the run produced, as an observation of the program on its dataset.
fn observed(model: &Model, run: &Run, input: &PlannerInput, evidence: &DatasetEvidence) -> estimate::Observed {
    let relations = (0..model.names.len()).filter(|&relation| run.component[relation].is_some() && !model.input[relation]).map(|relation| {
        let rows = &run.tables[relation].rows;
        let columns = (0..model.arity[relation]).map(|c| {
            let mut groups: HashMap<i32, u64, Fast> = HashMap::default();
            for row in rows { *groups.entry(row[c]).or_default() += 1; }
            (format!("c{c}"), estimate::ObservedColumn { distinct: Some(groups.len() as u64), max_degree: Some(groups.values().copied().max().unwrap_or(0)) })
        }).collect();
        let rounds = run.rounds[run.component[relation].unwrap()];
        (model.names[relation].clone(), estimate::ObservedRelation { rows: rows.len() as u64, rounds: (rounds > 0).then_some(rounds as u64), columns })
    }).collect();
    estimate::Observed { schema: estimate::OBSERVED_SCHEMA.into(), program_hash: input.identity.program_hash.clone(), dataset_hash: evidence.dataset.hash.clone(), relations }
}

/// Rounds as a bit set, round `r` at bit `r`.
#[derive(Clone, Copy, PartialEq)]
struct Rounds([u64; 4]);
impl Rounds {
    fn from(first: u32, last: u32) -> Self {
        let mut set = [0; 4];
        let last = last.min(255);
        for (word, bits) in set.iter_mut().enumerate() {
            let (low, high) = (first.max(word as u32 * 64), last.min(word as u32 * 64 + 63));
            if low > high { continue; }
            let width = high - low + 1;
            *bits = (if width == 64 { u64::MAX } else { (1 << width) - 1 }) << (low % 64);
        }
        Self(set)
    }
    fn and(self, other: Self) -> Self { Self([self.0[0] & other.0[0], self.0[1] & other.0[1], self.0[2] & other.0[2], self.0[3] & other.0[3]]) }
    fn or(self, other: Self) -> Self { Self([self.0[0] | other.0[0], self.0[1] | other.0[1], self.0[2] | other.0[2], self.0[3] | other.0[3]]) }
    fn count(self) -> u64 { self.0.iter().map(|word| word.count_ones() as u64).sum() }
    fn empty(self) -> bool { self.0 == [0; 4] }
}

/// One step of a valuation as the oracle executes it. A read retains
/// `(column, slot)` pairs and is keyed by `key`, those bound before it; a
/// negation probes `columns`.
enum Op {
    /// `distinct`: a bind, which enumerates each distinct value of its new
    /// columns once.
    Read { relation: usize, columns: Vec<(usize, usize)>, key: Vec<usize>, probe: Vec<Arg>, introduces: bool, delta: bool, distinct: bool },
    Absent { relation: usize, args: Vec<Arg>, columns: Vec<usize> },
    Check(Cond),
    /// A restriction: the filtered view of a read, charged as its read.
    Restrict(Cond),
    Pass,
    /// A step a restriction discharged: no work.
    Skip,
}

/// Frames charged so far, in total and per step, and the frames arriving
/// at each step.
struct Tally { total: u64, steps: Vec<u64>, arrive: Vec<u64> }
impl Tally {
    fn charge(&mut self, depth: usize, frames: u64) { self.total += frames; self.steps[depth] += frames; }
}

/// The exact work of a valuation over the whole run, counted per round, in
/// total and per step, with the frames arriving at each step, or `None`
/// above `cap`. A read that introduces is charged its output frames; every
/// other step its input frames.
fn work(ops: &[Op], tables: &[Table], members: &[usize], rounds: u32, steady: bool, fixed: &[(usize, i32)], cap: u64) -> Option<(u64, Vec<u64>, Vec<u64>)> {
    assert!(rounds <= 255, "more rounds than the oracle's round set holds");
    let mut frame: Frame = [None; 64];
    for &(slot, value) in fixed { frame[slot] = Some(value); }
    let all = if steady { Rounds::from(1, rounds) } else { Rounds::from(1, 1) };
    let mut tally = Tally { total: 0, steps: vec![0; ops.len()], arrive: vec![0; ops.len()] };
    fn walk(ops: &[Op], tables: &[Table], members: &[usize], steady: bool, depth: usize, frame: &mut Frame, live: Rounds, tally: &mut Tally, cap: u64) -> bool {
        let Some(op) = ops.get(depth) else { return true; };
        tally.arrive[depth] += live.count();
        match op {
            Op::Pass => { tally.charge(depth, live.count()); walk(ops, tables, members, steady, depth + 1, frame, live, tally, cap) }
            Op::Skip => walk(ops, tables, members, steady, depth + 1, frame, live, tally, cap),
            Op::Check(cond) | Op::Restrict(cond) => {
                if matches!(op, Op::Check(_)) { tally.charge(depth, live.count()); }
                let pass = holds(cond, frame).expect("a check runs once its variables are bound");
                !pass || walk(ops, tables, members, steady, depth + 1, frame, live, tally, cap)
            }
            Op::Absent { relation, args, columns } => {
                tally.charge(depth, live.count());
                let mut probe = [0; W];
                for (slot, &c) in columns.iter().enumerate() { probe[slot] = value(frame, args[c]).unwrap(); }
                !tables[*relation].matching(columns, &probe).is_empty() || walk(ops, tables, members, steady, depth + 1, frame, live, tally, cap)
            }
            Op::Read { relation, columns, key, probe: slots, introduces, delta, distinct } => {
                let mut probe = [0; W];
                for (at, argument) in slots.iter().enumerate() { probe[at] = value(frame, *argument).unwrap(); }
                let staged = steady && members.contains(relation);
                if !introduces {
                    tally.charge(depth, live.count());
                    // A semijoin: the rounds in which some matching tuple is visible.
                    let mut seen = Rounds([0; 4]);
                    for &id in tables[*relation].matching(key, &probe) {
                        let stage = tables[*relation].stage[id as usize];
                        seen = seen.or(if !staged { Rounds::from(0, 255) } else if *delta { Rounds::from(stage + 1, stage + 1) } else { Rounds::from(stage + 1, 255) });
                        if live.and(seen) == live { break; }
                    }
                    let next = live.and(seen);
                    return next.empty() || walk(ops, tables, members, steady, depth + 1, frame, next, tally, cap);
                }
                if *distinct {
                    // Each distinct value of the new columns, live in the
                    // rounds some row carrying it is visible.
                    let mut fresh: Vec<(usize, usize)> = Vec::new();
                    for &(c, slot) in columns { if frame[slot].is_none() && !fresh.iter().any(|(_, known)| *known == slot) { fresh.push((c, slot)); } }
                    let mut values: Vec<(Row, Rounds)> = Vec::new();
                    for &id in tables[*relation].matching(key, &probe) {
                        let row = tables[*relation].rows[id as usize];
                        let stage = tables[*relation].stage[id as usize];
                        let next = if !staged { live } else if *delta { live.and(Rounds::from(stage + 1, stage + 1)) } else { live.and(Rounds::from(stage + 1, 255)) };
                        if next.empty() || columns.iter().any(|&(c, slot)| frame[slot].is_some_and(|existing| existing != row[c])) { continue; }
                        let mut value = [0; W];
                        for (at, &(c, _)) in fresh.iter().enumerate() { value[at] = row[c]; }
                        values.push((value, next));
                    }
                    values.sort_unstable_by_key(|(value, _)| *value);
                    let mut at = 0;
                    while at < values.len() {
                        let (value, mut rounds) = values[at];
                        at += 1;
                        while at < values.len() && values[at].0 == value { rounds = rounds.or(values[at].1); at += 1; }
                        for (position, &(_, slot)) in fresh.iter().enumerate() { frame[slot] = Some(value[position]); }
                        tally.charge(depth, rounds.count());
                        let carry_on = tally.total <= cap && walk(ops, tables, members, steady, depth + 1, frame, rounds, tally, cap);
                        for &(_, slot) in &fresh { frame[slot] = None; }
                        if !carry_on { return false; }
                    }
                    return true;
                }
                for &id in tables[*relation].matching(key, &probe) {
                    let row = tables[*relation].rows[id as usize];
                    let stage = tables[*relation].stage[id as usize];
                    let next = if !staged { live } else if *delta { live.and(Rounds::from(stage + 1, stage + 1)) } else { live.and(Rounds::from(stage + 1, 255)) };
                    if next.empty() { continue; }
                    let mut assigned = 0_u64;
                    let mut ok = true;
                    for &(c, slot) in columns {
                        match frame[slot] { Some(existing) => if existing != row[c] { ok = false; break; }, None => { frame[slot] = Some(row[c]); assigned |= 1 << slot; } }
                    }
                    let mut carry_on = true;
                    if ok {
                        tally.charge(depth, next.count());
                        carry_on = tally.total <= cap && walk(ops, tables, members, steady, depth + 1, frame, next, tally, cap);
                    }
                    while assigned != 0 { let slot = assigned.trailing_zeros() as usize; frame[slot] = None; assigned &= assigned - 1; }
                    if !carry_on { return false; }
                }
                true
            }
        }
    }
    let complete = walk(ops, tables, members, steady, 0, &mut frame, all, &mut tally, cap);
    (complete && tally.total <= cap).then_some((tally.total, tally.steps, tally.arrive))
}

/// Statistics of the final relations, measured once each.
#[derive(Default)]
struct Pairs {
    memo: std::sync::Mutex<HashMap<String, (f64, f64, f64)>>,
    counts: std::sync::Mutex<HashMap<(usize, Vec<usize>), std::sync::Arc<HashMap<Row, u32, Fast>>>>,
}
/// How much of a component's relations a round sees: of a relation read in
/// full beside the changing occurrence, the share of its rows no younger
/// than a delta row (`after`); read before it, its average share over the
/// rounds (`before`). 1 for a relation outside the component.
struct Visible { after: Vec<f64>, before: Vec<f64> }
impl Visible {
    fn new(tables: &[Table], members: &[usize], delta: Option<usize>, rounds: u32) -> Self {
        let (mut after, mut before) = (vec![1.0; tables.len()], vec![1.0; tables.len()]);
        let histogram = |relation: usize| { let mut rows = vec![0f64; rounds as usize + 2]; for &stage in &tables[relation].stage { rows[(stage as usize).min(rounds as usize + 1)] += 1.0; } rows };
        for &member in members {
            let rows = tables[member].rows.len() as f64;
            if rows == 0.0 { continue; }
            let stages = histogram(member);
            // Rows of stage at most s, as a share.
            let mut seen = Vec::with_capacity(stages.len());
            let mut sum = 0.0;
            for count in &stages { sum += count; seen.push(sum / rows); }
            // Round t reads the rows of stage below t.
            before[member] = (1..=rounds as usize).map(|round| seen[round - 1]).sum::<f64>() / rounds.max(1) as f64;
            if let Some(delta) = delta {
                let changing = histogram(delta);
                let total: f64 = changing.iter().sum();
                if total > 0.0 { after[member] = changing.iter().zip(&seen).map(|(count, share)| count * share).sum::<f64>() / total; }
            }
        }
        Self { after, before }
    }
}
/// Where a key column of a probe comes from: a column of the relation that
/// bound its variable, or a constant.
#[derive(Clone, Copy, Debug)]
enum Source { Column(usize), Value(i32) }
impl Pairs {
    fn memoized(&self, name: String, compute: impl FnOnce() -> (f64, f64, f64)) -> (f64, f64, f64) {
        if let Some(known) = self.memo.lock().unwrap().get(&name) { return *known; }
        let result = compute();
        self.memo.lock().unwrap().insert(name, result);
        result
    }
    /// The rows of `s` passing its constant `filter`; the rows of `r` their
    /// keys match, summed; and how many of them match at all.
    fn join(&self, tables: &[Table], s: usize, filter: &[(usize, i32)], r: usize, key: &[usize], source: &[Source]) -> (f64, f64, f64) {
        self.memoized(format!("join {s} {filter:?} {r} {key:?} {source:?}"), || {
            let (mut rows, mut joined, mut hit) = (0u64, 0u64, 0u64);
            for row in &tables[s].rows {
                if filter.iter().any(|&(c, v)| row[c] != v) { continue; }
                let mut probe = [0; W];
                for (at, source) in source.iter().enumerate() { probe[at] = match *source { Source::Column(c) => row[c], Source::Value(v) => v }; }
                let matched = tables[r].matching(key, &probe).len() as u64;
                rows += 1; joined += matched; hit += (matched > 0) as u64;
            }
            (rows as f64, joined as f64, hit as f64)
        })
    }
    /// The rows of `r` per value of `columns`.
    fn counted(&self, tables: &[Table], r: usize, columns: &[usize]) -> std::sync::Arc<HashMap<Row, u32, Fast>> {
        if let Some(known) = self.counts.lock().unwrap().get(&(r, columns.to_vec())) { return known.clone(); }
        let mut counts: HashMap<Row, u32, Fast> = HashMap::default();
        for row in &tables[r].rows { *counts.entry(key(row, columns)).or_default() += 1; }
        let counts = std::sync::Arc::new(counts);
        self.counts.lock().unwrap().insert((r, columns.to_vec()), counts.clone());
        counts
    }
    /// `join` on a part of a key `r` has no index for; also the groups of
    /// `r` on that part.
    fn partial(&self, tables: &[Table], s: usize, filter: &[(usize, i32)], r: usize, columns: &[usize], source: &[Source]) -> (f64, f64, f64, f64) {
        let counts = self.counted(tables, r, columns);
        let (rows, joined, hit) = self.memoized(format!("partial {s} {filter:?} {r} {columns:?} {source:?}"), || {
            let (mut rows, mut joined, mut hit) = (0u64, 0u64, 0u64);
            for row in &tables[s].rows {
                if filter.iter().any(|&(c, v)| row[c] != v) { continue; }
                let mut probe = [0; W];
                for (at, source) in source.iter().enumerate() { probe[at] = match *source { Source::Column(c) => row[c], Source::Value(v) => v }; }
                let matched = counts.get(&probe).copied().unwrap_or(0) as u64;
                rows += 1; joined += matched; hit += (matched > 0) as u64;
            }
            (rows as f64, joined as f64, hit as f64)
        });
        (rows, joined, hit, counts.len() as f64)
    }
    /// The groups of `r` on `key` and its largest group.
    fn groups(&self, tables: &[Table], r: usize, key: &[usize]) -> (f64, f64) {
        let (groups, largest, _) = self.memoized(format!("groups {r} {key:?}"), || {
            let index = &tables[r].indexes[key];
            (index.len() as f64, index.values().map(Vec::len).max().unwrap_or(0) as f64, 0.0)
        });
        (groups, largest)
    }
    /// The share of the filtered rows of `s` satisfying `cond`, whose
    /// variables `slots` name columns of `s`.
    fn pass(&self, tables: &[Table], s: usize, filter: &[(usize, i32)], cond: &Cond, slots: &[(usize, usize)], fixed: &[(usize, i32)]) -> f64 {
        let (rows, pass, _) = self.memoized(format!("pass {s} {filter:?} {cond:?} {slots:?}"), || {
            let (mut rows, mut pass) = (0u64, 0u64);
            let mut frame: Frame = [None; 64];
            for &(slot, value) in fixed { frame[slot] = Some(value); }
            for row in &tables[s].rows {
                if filter.iter().any(|&(c, v)| row[c] != v) { continue; }
                for &(slot, c) in slots { frame[slot] = Some(row[c]); }
                rows += 1; pass += holds(cond, &frame).unwrap_or(true) as u64;
            }
            (rows as f64, pass as f64, 0.0)
        });
        if rows > 0.0 { pass / rows } else { 1.0 }
    }
}
fn slots_of(cond: &Cond, into: &mut BTreeSet<usize>) {
    match cond {
        Cond::Cmp(left, _, right) => for arg in [left, right] { if let Arg::Var(at) = arg { into.insert(*at); } },
        Cond::Or(branches) => for branch in branches { for cond in branch { slots_of(cond, into); } },
        _ => {}
    }
}

/// How a chain model takes a read's fan-out: the relation's average group
/// on the key; its largest group; the measured pair (the relation that
/// bound the key joined with the one read), the average where the key comes
/// from several relations; or `Full`: the pair, a key from several
/// relations taken from the binder that predicts fewest rows (its part of
/// the key matched, the rest of the key assumed to select one of that
/// part's groups), and a component's relations scaled by the share of
/// their rows the round sees.
#[derive(Clone, Copy, PartialEq)]
enum Fan { Average, Largest, Pair, Full }

/// A chain model of a valuation's whole-run frames: each step multiplies
/// the frames before it by a ratio taken from statistics of the final
/// relations alone. Returns the work under `fan` and, per op, the ratio
/// under each of the four (`NaN` where `Pair` has no single binder).
fn predict(ops: &[Op], tables: &[Table], fixed: &[(usize, i32)], rounds: u32, steady: bool, pairs: &Pairs, visible: &Visible, fan: Fan) -> Option<(f64, Vec<[f64; 4]>)> {
    let n = if steady { rounds as f64 } else { 1.0 };
    let constant: HashMap<usize, i32> = fixed.iter().copied().collect();
    // Per variable: the op that bound it, its relation and column.
    let mut binder: HashMap<usize, (usize, usize, usize)> = HashMap::new();
    let mut filters: Vec<Vec<(usize, i32)>> = vec![Vec::new(); ops.len()];
    let mut relations: Vec<usize> = vec![usize::MAX; ops.len()];
    let (mut frames, mut varies, mut total) = (n, false, 0.0);
    let mut ratios = Vec::with_capacity(ops.len());
    // The sources of a probe and, per key position, the op that bound it.
    let resolve = |args: &[Arg], binder: &HashMap<usize, (usize, usize, usize)>| -> Option<(Vec<Source>, Vec<Option<usize>>)> {
        let mut sources = Vec::new();
        let mut owners = Vec::new();
        for arg in args {
            let (source, owner) = match *arg {
                Arg::Const(value) => (Source::Value(value), None),
                Arg::Var(slot) => match constant.get(&slot) {
                    Some(&value) => (Source::Value(value), None),
                    None => { let &(op, _, column) = binder.get(&slot)?; (Source::Column(column), Some(op)) }
                },
                Arg::Any => return None,
            };
            sources.push(source); owners.push(owner);
        }
        Some((sources, owners))
    };
    for (at, op) in ops.iter().enumerate() {
        match op {
            Op::Skip => ratios.push([1.0; 4]),
            Op::Pass => { total += frames; ratios.push([1.0; 4]); }
            Op::Check(cond) | Op::Restrict(cond) => {
                if matches!(op, Op::Check(_)) { total += frames; }
                let mut slots = BTreeSet::new();
                slots_of(cond, &mut slots);
                let free: Vec<usize> = slots.into_iter().filter(|slot| !constant.contains_key(slot)).collect();
                let owners: BTreeSet<usize> = free.iter().filter_map(|slot| binder.get(slot).map(|bound| bound.0)).collect();
                let share = if owners.len() == 1 && free.iter().all(|slot| binder.contains_key(slot)) {
                    let owner = *owners.iter().next().unwrap();
                    let columns: Vec<(usize, usize)> = free.iter().map(|slot| (*slot, binder[slot].2)).collect();
                    pairs.pass(tables, relations[owner], &filters[owner], cond, &columns, fixed)
                } else { 1.0 };
                frames *= share;
                ratios.push([share; 4]);
            }
            Op::Absent { relation, args, columns } => {
                total += frames;
                let probe: Vec<Arg> = columns.iter().map(|&c| args[c]).collect();
                let share = match resolve(&probe, &binder) {
                    Some((sources, owners)) if owners.iter().flatten().collect::<BTreeSet<_>>().len() == 1 => {
                        let owner = *owners.iter().flatten().next().unwrap();
                        let (rows, _, hit) = pairs.join(tables, relations[owner], &filters[owner], *relation, columns, &sources);
                        if rows > 0.0 { 1.0 - hit / rows } else { 1.0 }
                    }
                    _ => 1.0,
                };
                frames *= share;
                ratios.push([share; 4]);
            }
            Op::Read { relation, columns, key, probe, introduces, delta, .. } => {
                let table = &tables[*relation];
                relations[at] = *relation;
                let (sources, owners) = resolve(probe, &binder)?;
                filters[at] = key.iter().zip(&sources).filter_map(|(&c, source)| match source { Source::Value(v) => Some((c, *v)), Source::Column(_) => None }).collect();
                let distinct: BTreeSet<usize> = owners.iter().flatten().copied().collect();
                // A delta read after a round-independent prefix meets each
                // delta row in one round: its final groups, once over the run.
                let scale = if *delta && steady && !varies { 1.0 / n } else { 1.0 };
                // (fan-out, hit share) under each way.
                let (average, largest, pair, full) = if distinct.is_empty() {
                    let mut constants = [0; W];
                    for (slot, source) in sources.iter().enumerate() { if let Source::Value(v) = source { constants[slot] = *v; } }
                    let matched = table.matching(key, &constants).len() as f64;
                    let exact = (matched, if matched > 0.0 { 1.0 } else { 0.0 });
                    (matched, matched, Some(exact), exact)
                } else {
                    let (groups, largest) = pairs.groups(tables, *relation, key);
                    let average = table.rows.len() as f64 / groups.max(1.0);
                    if distinct.len() == 1 {
                        let owner = *distinct.iter().next().unwrap();
                        let (rows, joined, hit) = pairs.join(tables, relations[owner], &filters[owner], *relation, key, &sources);
                        let pair = (rows > 0.0).then(|| (joined / rows, hit / rows));
                        (average, largest, pair, pair.unwrap_or((average, 1.0)))
                    } else {
                        // Each binder's part of the key, constants included.
                        let mut least: Option<(f64, f64)> = None;
                        for &owner in &distinct {
                            let part: Vec<usize> = (0..key.len()).filter(|&j| owners[j].is_none_or(|bound| bound == owner)).collect();
                            let part_columns: Vec<usize> = part.iter().map(|&j| key[j]).collect();
                            let part_sources: Vec<Source> = part.iter().map(|&j| sources[j]).collect();
                            let (rows, joined, hit, part_groups) = pairs.partial(tables, relations[owner], &filters[owner], *relation, &part_columns, &part_sources);
                            if rows == 0.0 { continue; }
                            let candidate = (joined / rows * part_groups / groups.max(1.0), hit / rows);
                            least = Some(match least { Some(known) => (known.0.min(candidate.0), known.1.min(candidate.1)), None => candidate });
                        }
                        (average, largest, None, least.unwrap_or((average, 1.0)))
                    }
                };
                let sees = if steady && !*delta { if varies { visible.after[*relation] } else { visible.before[*relation] } } else { 1.0 };
                let full = (full.0 * sees, full.1 * sees);
                let chosen = match fan { Fan::Average => (average, 1.0), Fan::Largest => (largest, 1.0), Fan::Pair => pair.unwrap_or((average, 1.0)), Fan::Full => full };
                if *introduces {
                    frames *= chosen.0 * scale;
                    total += frames;
                    ratios.push([average * scale, largest * scale, pair.map_or(f64::NAN, |pair| pair.0 * scale), full.0 * scale]);
                } else {
                    total += frames;
                    let kept = |fanout: f64, hit: f64| if scale < 1.0 { (fanout * scale).min(hit) } else { hit };
                    frames *= kept(chosen.0, chosen.1);
                    ratios.push([kept(average, 1.0), kept(largest, 1.0), pair.map_or(f64::NAN, |pair| kept(pair.0, pair.1)), kept(full.0, full.1)]);
                }
                if *delta { varies = true; }
                for &(c, slot) in columns { binder.entry(slot).or_insert((at, *relation, c)); }
            }
        }
    }
    Some((total, ratios))
}

/// The maintenance policy the compiler recorded for a demand.
fn policy<'a>(input: &'a PlannerInput, demand: &cost::IndexDemand) -> Option<&'a str> {
    let version = if demand.version == IndexVersion::Full { Version::Full } else { Version::Delta };
    let layouts: Vec<_> = input.layouts.iter().filter(|layout| layout.logical_relation == demand.logical_relation && layout.version == version).collect();
    let key: BTreeSet<usize> = demand.key.iter().copied().collect();
    layouts.iter().find(|layout| layout.columns.iter().take(layout.key).copied().collect::<BTreeSet<_>>() == key).map(|layout| layout.maintenance.as_str())
        .or_else(|| layouts.iter().all(|layout| layout.maintenance == layouts[0].maintenance).then(|| layouts.first().map(|layout| layout.maintenance.as_str())).flatten())
}

/// The rows the run writes into one index a hole demands, under its
/// recorded policy; `None` for a policy the model does not charge.
fn upkeep(input: &PlannerInput, demand: &cost::IndexDemand, model: &Model, run: &Run, rounds: u32, steady: bool) -> Option<u64> {
    let relation = model.names.iter().position(|name| *name == demand.logical_relation)?;
    let table = &run.tables[relation];
    let recursive = run.component[relation].is_some_and(|component| run.rounds[component] > 0);
    Some(match (policy(input, demand), demand.version, recursive) {
        (Some("replace_each_run"), IndexVersion::Full, true) => (1..=rounds).map(|round| table.stage.iter().filter(|&&stage| stage < round).count() as u64).sum(),
        (Some("replace_each_run"), _, false) if steady => rounds as u64 * table.rows.len() as u64,
        (Some("admitted_rows" | "replace_each_run"), _, _) => table.rows.len() as u64,
        _ => return None,
    })
}
/// Rows written into the indexes a valuation demands beyond those already
/// provisioned, under each recorded policy, h = m = 1.
fn maintained(input: &PlannerInput, hole: &cost::HoleAdvice, candidate: &Candidate, model: &Model, run: &Run, rounds: u32, steady: bool) -> Option<u64> {
    candidate.demands.difference(&hole.shared_before).map(|demand| upkeep(input, demand, model, run, rounds, steady)).sum()
}

/// A certificate's value at the synthesized facts and the input rows.
fn certificate(cost: &cost::LinearCost, report: &cost::AdviceReport, evidence: &DatasetEvidence) -> f64 {
    let number = |value: &ev::Rational| value.numerator.parse::<f64>().unwrap() / value.denominator.parse::<f64>().unwrap();
    let facts = &report.profile.as_ref().unwrap().facts;
    let fact = |name: &str, what: &str| [estimate::ESTIMATE, estimate::STRUCTURAL].iter()
        .find_map(|prefix| facts.get(&format!("{prefix}{name}:{what}"))).copied().unwrap_or_else(|| panic!("no fact {name}:{what}")) as f64;
    number(&cost.constant) + cost.terms.iter().map(|(term, coefficient)| number(coefficient) * match term.split(':').collect::<Vec<_>>()[..] {
        ["final_rows", "inferred", name] | ["rows", "inferred", name] => fact(name, "rows"),
        ["rounds", "inferred", name] => fact(name, "rounds"),
        ["rows", name] => evidence.relations[name].rows.as_ref().unwrap().value.unwrap() as f64,
        _ => panic!("unexpected certificate term {term}"),
    }).sum::<f64>()
}

/// The oracle's steps for one valuation of a hole.
fn ops(candidate: &Candidate, hole: &export::Hole, model: &Model, slots: &mut BTreeMap<String, usize>, mut bound: BTreeSet<usize>) -> Option<Vec<Op>> {
    let normal: cq_lang_cq::Program = syn::parse_str(&hole.normalized_cq).ok()?;
    let source: cq_lang_cq::Program = syn::parse_str(&hole.source_cq).ok()?;
    let body: Vec<&BodyItem> = normal.query.body.iter().collect();
    let mut slot = |name: &str| -> usize { let next = slots.len(); *slots.entry(name.to_string()).or_insert(next) };
    let mut result = Vec::new();
    let initial = bound.clone();
    let mut bound_after: Vec<BTreeSet<usize>> = Vec::new();
    for step in &candidate.steps {
        if let Some(read) = &step.read {
            let relation = model.names.iter().position(|name| *name == read.logical_relation)?;
            let columns: Vec<(usize, usize)> = read.columns.iter().filter_map(|column| Some((column.source_column, slot(column.variable.as_deref()?)))).collect();
            // A constant column is a key: the engine reads it through a
            // constant-keyed index, never by scanning the relation.
            let occurrence = hole.occurrences.iter().find(|occurrence| occurrence.normalized_path == [read.normalized_clause])?;
            let BodyItem::Positive { atom } = body.get(read.normalized_clause)? else { return None };
            let mut key: Vec<(usize, Arg)> = Vec::new();
            for column in &read.columns {
                match &column.variable {
                    Some(name) => { let at = slot(name); if bound.contains(&at) { key.push((column.source_column, Arg::Var(at))); } }
                    None => {
                        let argument = atom.args.iter().nth(occurrence.columns.iter().position(|&source| source == column.source_column)?)?;
                        key.push((column.source_column, match rust::variable(argument) {
                            Some(name) => Arg::Var(slot(&symbol_name(name))),
                            None => Arg::Const(closed(argument, &model.constants)),
                        }));
                    }
                }
            }
            bound.extend(columns.iter().map(|(_, slot)| *slot));
            result.push(Op::Read { relation, key: key.iter().map(|(c, _)| *c).collect(), probe: key.iter().map(|(_, arg)| *arg).collect(),
                columns, introduces: step.introduces, delta: read.version == Version::Delta, distinct: read.partial && step.introduces });
            bound_after.push(bound.clone());
            continue;
        }
        match body.get(step.clause)? {
            BodyItem::Constraint(constraint) => result.push(Op::Check(condition(constraint, &mut |term| match term {
                Term::Variable(name) => Arg::Var(slot(&symbol_name(name))),
                Term::Closed(expression) => Arg::Const(closed(expression, &model.constants)),
                Term::Literal(expression) => Arg::Const(closed(expression, &model.constants)),
            }))),
            BodyItem::Negation(negation) => {
                // A key-set alias: the source negation over the same variables.
                let wanted: Vec<String> = negation.atom.args.iter().filter_map(rust::variable).map(symbol_name).collect();
                let original = source.query.body.iter().find_map(|item| match item {
                    BodyItem::Negation(original) if original.atom.args.iter().filter_map(rust::variable).map(symbol_name).collect::<Vec<_>>() == wanted => Some(original),
                    _ => None,
                })?;
                let relation = model.names.iter().position(|name| *name == symbol_name(&original.atom.relation))?;
                let args: Vec<Arg> = original.atom.args.iter().map(|expression| match rust::variable(expression) {
                    Some(name) => Arg::Var(slot(&symbol_name(name))),
                    None if matches!(expression, syn::Expr::Infer(_)) => Arg::Any,
                    None => Arg::Const(closed(expression, &model.constants)),
                }).collect();
                let columns = args.iter().enumerate().filter(|(_, arg)| !matches!(arg, Arg::Any)).map(|(c, _)| c).collect();
                result.push(Op::Absent { relation, args, columns });
            }
            _ => result.push(Op::Pass),
        }
        bound_after.push(bound.clone());
    }
    // A predicate over one atom's variables is a restriction of that atom
    // (O-Restrict, whatever the order): the engine applies it at that read,
    // so it filters as soon as its variables are bound, whether the compiler
    // absorbed it or the valuation places a step for it.
    fn vars(cond: &Cond, into: &mut BTreeSet<usize>) {
        match cond {
            Cond::Cmp(left, _, right) => for arg in [left, right] { if let Arg::Var(at) = arg { into.insert(*at); } },
            Cond::Or(branches) => for branch in branches { for cond in branch { vars(cond, into); } },
            _ => {}
        }
    }
    let atoms: Vec<BTreeSet<usize>> = body.iter().filter_map(|item| match item {
        BodyItem::Positive { atom } => Some(atom.args.iter().filter_map(rust::variable).map(|name| slot(&symbol_name(name))).collect()),
        _ => None,
    }).collect();
    let mut absorbed = Vec::new();
    for (clause, item) in body.iter().enumerate() {
        let BodyItem::Constraint(constraint) = item else { continue };
        let placed = candidate.steps.iter().position(|step| step.clause == clause && step.read.is_none());
        let cond = condition(constraint, &mut |term| match term {
            Term::Variable(name) => Arg::Var(slot(&symbol_name(name))),
            Term::Closed(expression) => Arg::Const(closed(expression, &model.constants)),
            Term::Literal(expression) => Arg::Const(closed(expression, &model.constants)),
        });
        let mut probe = BTreeSet::new();
        vars(&cond, &mut probe);
        let local = atoms.iter().any(|atom| probe.is_subset(atom));
        match placed {
            Some(_) if !local => continue,
            Some(at) => result[at] = Op::Skip,
            None => {}
        }
        let at = if probe.is_subset(&initial) { 0 } else { bound_after.iter().position(|after| probe.is_subset(after))? + 1 };
        absorbed.push((at, cond));
    }
    absorbed.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    for (at, cond) in absorbed { result.insert(at, Op::Restrict(cond)); }
    Some(result)
}
/// Variables a top-level constraint fixes to a closed value.
fn fixed(hole: &export::Hole, model: &Model, slots: &mut BTreeMap<String, usize>) -> Vec<(usize, i32)> {
    let Ok(normal) = syn::parse_str::<cq_lang_cq::Program>(&hole.normalized_cq) else { return Vec::new() };
    let mut result = Vec::new();
    for item in normal.query.body.iter() {
        let BodyItem::Constraint(Constraint::Compare { left, comparison: Comparison::Eq(_), right }) = item else { continue };
        let constant = |term: &Term| match term { Term::Closed(value) => Some(closed(value, &model.constants)), Term::Literal(value) => Some(closed(value, &model.constants)), Term::Variable(_) => None };
        let (name, value) = match (left, right) {
            (Term::Variable(name), other) | (other, Term::Variable(name)) if constant(other).is_some() => (name, constant(other).unwrap()),
            _ => continue,
        };
        let next = slots.len();
        result.push((*slots.entry(symbol_name(name)).or_insert(next), value));
    }
    result
}

/// A rule line with the positive atoms of its body permuted: reversed, or
/// shuffled by a seed and the rule's label. Any other line as it is.
fn scrambled(line: &str, mode: &str) -> String {
    let Some((head, body)) = line.split_once(" :- ") else { return line.to_string() };
    let Some(body) = body.trim_end().strip_suffix(';') else { return line.to_string() };
    // Top-level items; everything from `where` on is one tail.
    let (mut items, mut depth, mut start) = (Vec::new(), 0usize, 0usize);
    let bytes = body.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b',' if depth == 0 => {
                items.push(body[start..at].trim());
                start = at + 1;
                if body[start..].trim_start().starts_with("where ") { break; }
            }
            _ => {}
        }
        at += 1;
    }
    items.push(body[start..].trim());
    let (mut positive, rest): (Vec<&str>, Vec<&str>) = items.into_iter().partition(|item| item.starts_with("r_"));
    match mode.split_once(':') {
        None if mode == "reverse" => positive.reverse(),
        Some(("shuffle", seed)) => {
            let label = head.trim().split(':').next().unwrap_or("");
            let mut state = label.bytes().fold(seed.parse::<u64>().expect("a shuffle seed") ^ 0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ byte as u64).wrapping_mul(0x100_0000_01b3)) | 1;
            let mut next = || { state ^= state << 13; state ^= state >> 7; state ^= state << 17; state };
            for last in (1..positive.len()).rev() { positive.swap(last, (next() % (last as u64 + 1)) as usize); }
        }
        _ => panic!("DOOP_SCRAMBLE is reverse or shuffle:<seed>"),
    }
    format!("{head} :- {};", positive.into_iter().chain(rest).collect::<Vec<_>>().join(", "))
}

/// The evidence with the degree facts the variable planner requests,
/// collected over the run's relations with each row's stage (PS-Collect);
/// the requests and the time `finish` took.
fn collect_degrees(input: &PlannerInput, evidence: &DatasetEvidence, model: &Model, run: &Run, threads: usize) -> (DatasetEvidence, usize, std::time::Duration) {
    let requests = cost::variables::Planner::new(input, evidence).unwrap().requests().unwrap();
    let requested = requests.len();
    let mut collector = cq_load::stats::DegreeCollector::new(requests);
    let mut cells = [0i64; W];
    for (relation, name) in model.names.iter().enumerate() {
        let Some(id) = collector.relation(name) else { continue };
        let table = &run.tables[relation];
        for (row, &stage) in table.rows.iter().zip(&table.stage) {
            for column in 0..model.arity[relation] { cells[column] = row[column].into(); }
            collector.push(id, stage, &cells[..model.arity[relation]]);
        }
    }
    let clock = std::time::Instant::now();
    let values = collector.finish(threads).unwrap();
    let collecting = clock.elapsed();
    let provenance = ev::Provenance { producer: "doop_order.rs".into(), method: "exact degrees of the oracle's run".into(), locator: "doop_order.rs".into(), run: None, artifact_hash: None };
    let mut measured = evidence.clone();
    measured.degrees = cq_load::stats::bind_degrees(values, &evidence.dataset.hash, ev::Source::Exact, ev::Claim::Equal, provenance);
    measured.check().unwrap();
    (measured, requested, collecting)
}

/// The evidence with the planner's degree facts as the runtime keeps them
/// (PS-Admit): every row of the run admitted with its stage, each
/// relation's rows in stage order; and the time admitting and finishing took.
fn admit_degrees(input: &PlannerInput, evidence: &DatasetEvidence, model: &Model, run: &Run) -> (DatasetEvidence, std::time::Duration) {
    let requests = cost::variables::Planner::new(input, evidence).unwrap().requests().unwrap();
    let relations: Vec<(&str, usize)> = model.names.iter().map(String::as_str).zip(model.arity.iter().copied()).collect();
    let mut degrees = cq_load::stats::admitted_degrees(&requests, &relations).unwrap();
    // The oracle's tables hold their rows in the order it derived them.
    for table in &run.tables { assert!(table.stage.is_sorted(), "a table's rows are not in stage order"); }
    let clock = std::time::Instant::now();
    let mut cells = [0i64; W];
    for (relation, table) in run.tables.iter().enumerate() {
        if !degrees.wants(relation) { continue; }
        // PS-Heavy: a relation's heavy keys first, then its rows.
        if degrees.observes(relation) {
            for row in &table.rows {
                for column in 0..model.arity[relation] { cells[column] = row[column].into(); }
                degrees.observe(relation, &cells[..model.arity[relation]]);
            }
            degrees.observed(relation);
        }
        for (row, &stage) in table.rows.iter().zip(&table.stage) {
            for column in 0..model.arity[relation] { cells[column] = row[column].into(); }
            degrees.admit(relation, stage, stage, &cells[..model.arity[relation]]);
        }
    }
    let values = requests.into_iter().zip(degrees.finish()).map(|(request, (numerator, denominator))| (request, numerator, denominator)).collect();
    let admitting = clock.elapsed();
    // An admitted degree observes one run; it is no exact fact of the dataset.
    let observed = ev::RunIdentity { program_hash: input.identity.program_hash.clone(), configuration_hash: input.identity.configuration_hash.clone(),
        admission: ev::Admission::InsertOnlySet, run_id: "doop-order-oracle".into(), completed: true };
    let provenance = ev::Provenance { producer: "doop_order.rs".into(), method: "degrees kept as the oracle's rows are admitted: sketches and a sample of keys".into(), locator: "doop_order.rs".into(), run: Some(observed), artifact_hash: None };
    let mut measured = evidence.clone();
    measured.degrees = cq_load::stats::bind_degrees(values, &evidence.dataset.hash, ev::Source::Calibrated, ev::Claim::Observation, provenance);
    measured.check().unwrap();
    (measured, admitting)
}

/// What the planner's degree facts cost on a DOOP dataset, exact and as
/// the runtime keeps them: the time of `DegreeCollector::finish` at every
/// core and at 1 thread; the time of admitting every row; how far each
/// admitted fact is from the exact one; and what the difference costs the
/// plans, in the exact facts' own measure.
#[test]
#[ignore = "measurement over a prepared DOOP dataset: set DOOP_PREPARED"]
fn the_cost_of_collecting_degree_facts_on_doop() {
    let dataset = std::path::PathBuf::from(std::env::var("DOOP_PREPARED").expect("DOOP_PREPARED names a prepared DOOP dataset"));
    let doop = Doop::load(|text| text.to_string());
    let model = Model::new(&doop.program, doop.constants(&dataset));
    let run = Run::evaluate(&model, doop.rows(&dataset, &model));
    let (input, evidence) = doop.planned(&dataset);
    let peak = || std::fs::read_to_string("/proc/self/status").unwrap().lines().find_map(|line| line.strip_prefix("VmHWM:").map(|rest| rest.trim().to_string())).unwrap();
    let rows: usize = run.tables.iter().map(|table| table.rows.len()).sum();
    eprintln!("{} rows in all relations; peak before collecting {}", rows, peak());
    let mut facts = Vec::new();
    for threads in [std::thread::available_parallelism().map_or(1, usize::from), 1] {
        let (measured, requested, collecting) = collect_degrees(&input, &evidence, &model, &run, threads);
        eprintln!("{requested} requests, {threads} threads: {collecting:.2?}; peak {}", peak());
        facts.push(measured.degrees);
    }
    assert_eq!(facts[0], facts[1], "the facts depend on the thread count");
    // `DOOP_DEGREES_OUT=<file>`: every fact as `id<TAB>numerator/denominator`,
    // to compare with those a calibrated engine run collects.
    if let Ok(path) = std::env::var("DOOP_DEGREES_OUT") {
        let text: String = facts[0].iter().map(|degree| { let value = degree.value.value.as_ref().unwrap(); format!("{}\t{}/{}\n", degree.value.id, value.numerator, value.denominator) }).collect();
        std::fs::write(path, text).unwrap();
    }
    let ratio = |degree: &ev::DegreeEvidence| { let value = degree.value.value.as_ref().unwrap(); value.numerator.parse::<f64>().unwrap() / value.denominator.parse::<f64>().unwrap() };
    let exact: BTreeMap<&str, f64> = facts[0].iter().map(|degree| (degree.value.id.as_str(), ratio(degree))).collect();
    let (admitted, admitting) = admit_degrees(&input, &evidence, &model, &run);
    // Per kind of request: the facts, the mean relative error, and how
    // many are off by more than a quarter and by more than a factor of two.
    let mut kinds: BTreeMap<&str, (usize, f64, usize, usize)> = BTreeMap::new();
    let mut worst: Vec<(f64, String)> = Vec::new();
    for degree in &admitted.degrees {
        let (truth, estimate) = (exact[degree.value.id.as_str()], ratio(degree));
        let error = if truth == 0.0 { estimate } else { (estimate - truth).abs() / truth };
        let kind = kind_of(&degree.request);
        let entry = kinds.entry(kind).or_default();
        *entry = (entry.0 + 1, entry.1 + error, entry.2 + usize::from(error > 0.25), entry.3 + usize::from(!(0.5..=2.0).contains(&(estimate / truth.max(f64::MIN_POSITIVE))) && truth != estimate));
        worst.push((error, format!("{} exact {truth:.4} admitted {estimate:.4}", degree.value.id)));
    }
    eprintln!("admitted: {admitting:.2?}; peak {}", peak());
    for (kind, (count, sum, quarter, double)) in kinds { eprintln!("  {kind:15} {count:4} facts, mean error {:.3}, {quarter} off by a quarter, {double} by a factor of two", sum / count as f64); }
    worst.sort_by(|left, right| right.0.total_cmp(&left.0));
    for (error, fact) in worst.iter().take(6) { eprintln!("  worst {error:.2}: {fact}"); }
    // The plans: the order each set of facts chooses, costed by the exact facts.
    let mut measured = evidence.clone();
    measured.degrees = facts.swap_remove(0);
    let (by_exact, by_admitted) = (cost::variables::Planner::new(&input, &measured).unwrap(), cost::variables::Planner::new(&input, &admitted).unwrap());
    let (mut planned, mut differ, mut best, mut chosen) = (0, 0, 0.0, 0.0);
    for hole in 0..input.holes.len() {
        let (Some(exact), Some(admitted)) = (by_exact.plan(hole).unwrap(), by_admitted.plan(hole).unwrap()) else { continue };
        planned += 1;
        best += exact.estimate;
        if exact.order == admitted.order { chosen += exact.estimate; continue; }
        differ += 1;
        let costed = by_exact.lower(hole, &admitted.order).unwrap().expect("an order of the hole's variables");
        chosen += costed.estimate;
        eprintln!("  hole {hole}: exact order {:?} costs {:.4}; admitted order {:?} costs {:.4} by the exact facts", exact.order, exact.estimate, admitted.order, costed.estimate);
    }
    eprintln!("{planned} holes planned from both; {differ} orders differ; summed VP-Cost by the exact facts: {best:.3} for the exact orders, {chosen:.3} for the admitted ones");
}

fn kind_of(request: &ev::DegreeRequest) -> &'static str {
    match request {
        ev::DegreeRequest::Distinct { .. } => "distinct",
        ev::DegreeRequest::Visible { .. } => "visible",
        ev::DegreeRequest::Rounds { .. } => "rounds",
        ev::DegreeRequest::Joined { reach: None, staged: false, .. } => "joined",
        ev::DegreeRequest::Joined { reach: None, staged: true, .. } => "joined staged",
        ev::DegreeRequest::Joined { reach: Some(_), staged: false, .. } => "reached",
        ev::DegreeRequest::Joined { reach: Some(_), staged: true, .. } => "reached staged",
    }
}

/// What planning a DOOP dataset needs of its statistics. Of the facts
/// requested: those the search reads, and those the chosen orders are
/// valued by. Then what each kind is worth: the orders chosen with that
/// kind replaced by what a cheaper statistic gives, costed by the exact
/// facts; a hole weighs the rows of its changing relation, a seed one.
#[test]
#[ignore = "measurement over a prepared DOOP dataset: set DOOP_PREPARED"]
fn what_the_plans_need_of_degree_facts_on_doop() {
    use ev::DegreeRequest as Request;
    let dataset = std::path::PathBuf::from(std::env::var("DOOP_PREPARED").expect("DOOP_PREPARED names a prepared DOOP dataset"));
    let doop = Doop::load(|text| text.to_string());
    let model = Model::new(&doop.program, doop.constants(&dataset));
    let run = Run::evaluate(&model, doop.rows(&dataset, &model));
    let (input, evidence) = doop.planned(&dataset);
    let requests = cost::variables::Planner::new(&input, &evidence).unwrap().requests().unwrap();
    // What stands in for a matched degree: the same without rounds; the
    // same without the reach (a direct staged degree is another
    // statistic, so without rounds too); the read relation's average.
    let unstaged = |request: &Request| {
        let mut twin = request.clone();
        match &mut twin { Request::Joined { staged, .. } if *staged => *staged = false, _ => return None }
        Some(twin)
    };
    let direct = |request: &Request| {
        let mut twin = request.clone();
        match &mut twin { Request::Joined { reach, staged, .. } if reach.is_some() => { *reach = None; *staged = false; } _ => return None }
        Some(twin)
    };
    let average = |request: &Request| {
        let Request::Joined { read, bound, column, .. } = request else { return None };
        let mut columns = bound.clone();
        columns.push(*column);
        columns.sort_unstable();
        Some([Request::Distinct { rows: read.clone(), columns }, Request::Distinct { rows: read.clone(), columns: bound.clone() }])
    };
    let mut all: BTreeSet<Request> = requests.iter().cloned().collect();
    for request in &requests {
        all.extend(unstaged(request));
        all.extend(direct(request));
        all.extend(average(request).into_iter().flatten());
    }
    let mut collector = cq_load::stats::DegreeCollector::new(all.into_iter().collect());
    let mut cells = [0i64; W];
    for (relation, name) in model.names.iter().enumerate() {
        let Some(id) = collector.relation(name) else { continue };
        let table = &run.tables[relation];
        for (row, &stage) in table.rows.iter().zip(&table.stage) {
            for column in 0..model.arity[relation] { cells[column] = row[column].into(); }
            collector.push(id, stage, &cells[..model.arity[relation]]);
        }
    }
    let exact: HashMap<Request, (u64, u64)> = collector.finish(std::thread::available_parallelism().map_or(1, usize::from)).unwrap()
        .into_iter().map(|(request, numerator, denominator)| (request, (numerator, denominator))).collect();
    let bind = |value: &dyn Fn(&Request) -> (u64, u64)| {
        let provenance = ev::Provenance { producer: "doop_order.rs".into(), method: "degrees of the oracle's run, a kind replaced".into(), locator: "doop_order.rs".into(), run: None, artifact_hash: None };
        let values = requests.iter().map(|request| {
            let (numerator, denominator) = value(request);
            if denominator == 0 { (request.clone(), 0, 1) } else { (request.clone(), numerator, denominator) }
        }).collect();
        let mut measured = evidence.clone();
        measured.degrees = cq_load::stats::bind_degrees(values, &evidence.dataset.hash, ev::Source::Exact, ev::Claim::Equal, provenance);
        measured.check().unwrap();
        measured
    };
    let of = |request: &Request| exact[request];
    let averaged = |request: &Request| average(request).map(|[values, groups]| (of(&values).0, of(&groups).0));
    let by_exact = cost::variables::Planner::new(&input, &bind(&of)).unwrap();

    let (mut searched, mut valued) = (BTreeSet::new(), BTreeSet::new());
    let mut plans = Vec::new();
    for hole in 0..input.holes.len() {
        let Some(planned) = by_exact.plan(hole).unwrap() else { continue };
        searched.extend(planned.read.iter().cloned());
        valued.extend(by_exact.lower(hole, &planned.order).unwrap().expect("the planned order").read);
        let weight = planned.candidate.steps.iter().filter_map(|step| step.read.as_ref()).find(|read| read.version == Version::Delta)
            .and_then(|read| model.names.iter().position(|name| *name == read.logical_relation)).map_or(1.0, |relation| run.tables[relation].rows.len() as f64);
        plans.push((hole, weight, planned.order, planned.estimate));
    }
    let mut kinds: BTreeMap<&str, [usize; 3]> = BTreeMap::new();
    for request in &requests {
        let entry = kinds.entry(kind_of(request)).or_default();
        entry[0] += 1;
        entry[1] += usize::from(searched.contains(request));
        entry[2] += usize::from(valued.contains(request));
    }
    eprintln!("{} holes planned; facts requested, read by the search, valuing the chosen orders:", plans.len());
    for (kind, [requested, searched, valued]) in &kinds { eprintln!("  {kind:15} {requested:4} {searched:4} {valued:4}"); }
    eprintln!("  {:15} {:4} {:4} {:4}", "all", requests.len(), searched.len(), valued.len());

    let visible = |request: &Request| matches!(request, Request::Visible { .. });
    let variants: [(&str, &dyn Fn(&Request) -> (u64, u64)); 6] = [
        ("every fact exact", &of),
        ("rounds dropped: staged as unstaged, every row visible", &|request| if visible(request) { (1, 1) } else { unstaged(request).map_or_else(|| of(request), |twin| of(&twin)) }),
        ("reach dropped: reached as direct", &|request| direct(request).map_or_else(|| of(request), |twin| of(&twin))),
        ("rounds and reach dropped: what a stored relation gives", &|request| if visible(request) { (1, 1) } else { direct(request).or_else(|| unstaged(request)).map_or_else(|| of(request), |twin| of(&twin)) }),
        ("matched degrees dropped: a relation's average degree, visible shares kept", &|request| averaged(request).unwrap_or_else(|| of(request))),
        ("distinct counts only: average degrees, every row visible", &|request| if visible(request) { (1, 1) } else { averaged(request).unwrap_or_else(|| of(request)) }),
    ];
    for (name, value) in &variants {
        let planner = cost::variables::Planner::new(&input, &bind(*value)).unwrap();
        let (mut differ, mut best, mut chosen, mut worse) = (0, 0.0, 0.0, Vec::new());
        for (hole, weight, order, estimate) in &plans {
            best += weight * estimate;
            let planned = planner.plan(*hole).unwrap().expect("a hole the exact facts plan");
            if planned.order == *order { chosen += weight * estimate; continue; }
            differ += 1;
            let costed = by_exact.lower(*hole, &planned.order).unwrap().expect("an order of the hole's variables").estimate;
            chosen += weight * costed;
            if costed > estimate * 1.1 { worse.push((weight * (costed - estimate), costed / estimate, *weight, *hole)); }
        }
        eprintln!("{name}: {differ} of {} orders differ, {} cost over 1.1 times the best; weighted cost {:.4} times the best", plans.len(), worse.len(), chosen / best);
        worse.sort_by(|left, right| right.0.total_cmp(&left.0));
        for (_, ratio, weight, hole) in worse.iter().take(4) {
            let Origin::Rule { source_label, variant, seed, .. } = &input.holes[*hole].origin else { unreachable!() };
            eprintln!("    {}{}#{variant}: {ratio:.2} times, {weight:.0} changing rows", source_label.clone().unwrap_or_default(), if *seed { " seed" } else { "" });
        }
    }
}

/// The inputs of a calibrated run of the CPU engine (`srd-doop-mini
/// --features planner-calibration,srd-doop-shared/default-query --
/// PREPARED --calibrate plans.json evidence.json request.json OUT RUN`),
/// written to `DOOP_CALIBRATION_OUT`: the checked plans of the program in
/// its written orders, the exact input evidence, and its request.
#[test]
#[ignore = "writes calibration inputs for a prepared DOOP dataset: set DOOP_PREPARED and DOOP_CALIBRATION_OUT"]
fn the_calibration_inputs_of_doop() {
    let dataset = std::path::PathBuf::from(std::env::var("DOOP_PREPARED").expect("DOOP_PREPARED names a prepared DOOP dataset"));
    let out = std::path::PathBuf::from(std::env::var("DOOP_CALIBRATION_OUT").expect("DOOP_CALIBRATION_OUT names a directory"));
    assert!(std::env::var("DOOP_DRIVER").is_err(), "the CPU engine compiles the written orders");
    let doop = Doop::load(|text| text.to_string());
    let (input, evidence) = doop.planned(&dataset);
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("plans.json"), serde_json::to_vec(&input).unwrap()).unwrap();
    std::fs::write(out.join("evidence.json"), evidence.canonical_json().unwrap()).unwrap();
    std::fs::write(out.join("request.json"), serde_json::to_vec_pretty(&doop.request()).unwrap()).unwrap();
    eprintln!("{} holes, dataset {}", input.holes.len(), evidence.dataset.hash);
}

#[test]
#[ignore = "measurement over a prepared DOOP dataset: set DOOP_PREPARED"]
fn the_order_the_tool_selects_against_the_optimal_order_on_doop() {
    let planned_dir = std::path::PathBuf::from(std::env::var("DOOP_PREPARED").expect("DOOP_PREPARED names a prepared DOOP dataset"));
    let example_dir = std::env::var("DOOP_EXAMPLE").map(std::path::PathBuf::from).unwrap_or_else(|_| planned_dir.clone());
    let evaluated = |doop: &Doop, dir: &Path| -> (Model, Run) {
        let model = Model::new(&doop.program, doop.constants(dir));
        let rows = doop.rows(dir, &model);
        let run = Run::evaluate(&model, rows);
        (model, run)
    };
    let clock = std::time::Instant::now();
    let mut doop = Doop::load(|text| text.to_string());
    let (mut model, mut run) = evaluated(&doop, &planned_dir);
    eprintln!("evaluated {} in {:.1?}", planned_dir.display(), clock.elapsed());
    // `DOOP_SPLIT=<rule>:<variable>:<relation>:<column>` splits a rule on the
    // heavy values of a relation's column (more than four times its average
    // group, at most 32): one rule per heavy value `h` with `variable = h`,
    // and the rule itself with `variable != h` for all of them.
    if let Ok(split) = std::env::var("DOOP_SPLIT") {
        let [label, variable, relation, column]: [&str; 4] = split.split(':').collect::<Vec<_>>().try_into().expect("rule:variable:relation:column");
        let at = model.names.iter().position(|name| name == relation).expect("split relation");
        let column: usize = column.parse().unwrap();
        let mut counts: BTreeMap<i32, usize> = BTreeMap::new();
        for row in &run.tables[at].rows { *counts.entry(row[column]).or_default() += 1; }
        let average = run.tables[at].rows.len() as f64 / counts.len().max(1) as f64;
        let mut heavy: Vec<(i32, usize)> = counts.into_iter().filter(|(_, count)| *count as f64 > 4.0 * average).collect();
        heavy.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        heavy.truncate(32);
        eprintln!("split {label} on {variable}: heavy values of {relation}.c{column} (average group {average:.1}): {heavy:?}");
        doop = Doop::load(|text| text.lines().map(|line| {
            let Some(rule) = line.trim().strip_prefix(&format!("{label}:")) else { return line.to_string() };
            let rule = rule.trim().strip_suffix(';').unwrap();
            let joined = |condition: String| if rule.contains(", where ") { format!("{rule} && {condition};") } else { format!("{rule}, where {condition};") };
            let residual = heavy.iter().map(|(value, _)| format!("{variable} != {value}")).collect::<Vec<_>>().join(" && ");
            // The variable's every occurrence becomes the value, so its reads
            // are constant-keyed.
            let substitute = |value: i32| {
                let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
                let mut out = String::new();
                let mut rest = rule;
                while let Some(at) = rest.find(variable) {
                    let before = rest[..at].chars().last();
                    let after = rest[at + variable.len()..].chars().next();
                    out.push_str(&rest[..at]);
                    if before.is_some_and(ident) || after.is_some_and(ident) { out.push_str(variable); } else { out.push_str(&value.to_string()); }
                    rest = &rest[at + variable.len()..];
                }
                out + rest
            };
            std::iter::once(format!("{label}: {}", joined(residual)))
                .chain(heavy.iter().enumerate().map(|(k, (value, _))| format!("{label}_heavy{k}: {};", substitute(*value))))
                .collect::<Vec<_>>().join("\n")
        }).collect::<Vec<_>>().join("\n"));
        (model, run) = evaluated(&doop, &planned_dir);
        eprintln!("evaluated the split program in {:.1?}", clock.elapsed());
    }
    // `DOOP_SCRAMBLE=reverse|shuffle:<seed>` permutes the positive atoms of
    // every rule body (negations and the `where` clause stay last) before
    // planning; the run above is of the same rules. `DOOP_SCRAMBLE_OUT`
    // receives the permuted program text for an engine build.
    if let Ok(mode) = std::env::var("DOOP_SCRAMBLE") {
        let rewrite = |text: &str| text.lines().map(|line| scrambled(line, &mode)).collect::<Vec<_>>().join("\n");
        doop = Doop::load(rewrite);
        if let Ok(out) = std::env::var("DOOP_SCRAMBLE_OUT") {
            let srd = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../datalog/doop/srd/src/generated_default.rs");
            std::fs::write(out, rewrite(&std::fs::read_to_string(srd).unwrap())).unwrap();
        }
    }
    // `DOOP_ORACLE_DUMP=<dir>` writes every relation with each tuple's
    // admission round, for offline analysis of degrees and skew.
    if let Ok(dump) = std::env::var("DOOP_ORACLE_DUMP") {
        use std::io::Write;
        std::fs::create_dir_all(&dump).unwrap();
        for (relation, name) in model.names.iter().enumerate() {
            let mut out = std::io::BufWriter::new(std::fs::File::create(Path::new(&dump).join(format!("{name}.tsv"))).unwrap());
            let table = &run.tables[relation];
            for (row, stage) in table.rows.iter().zip(&table.stage) {
                for value in &row[..model.arity[relation]] { write!(out, "{value}\t").unwrap(); }
                writeln!(out, "{stage}").unwrap();
            }
        }
        eprintln!("dumped to {dump} in {:.1?}", clock.elapsed());
    }
    let mut counts = String::new();
    for (relation, name) in model.names.iter().enumerate().filter(|(relation, _)| !model.input[*relation]) {
        counts += &format!("{name}\t{}\t{}\n", run.tables[relation].rows.len(), run.component[relation].map_or(0, |c| run.rounds[c]));
    }
    let (input, evidence) = doop.planned(&planned_dir);
    let profile = {
        let mut profile = estimate::Profile::new(input.identity.program_hash.clone());
        if example_dir == planned_dir {
            profile.add(estimate::example(&input, &evidence, &observed(&model, &run, &input, &evidence)).unwrap());
        } else {
            let (example_model, example_run) = evaluated(&doop, &example_dir);
            let (example_input, example_evidence) = doop.planned(&example_dir);
            profile.add(estimate::example(&example_input, &example_evidence, &observed(&example_model, &example_run, &example_input, &example_evidence)).unwrap());
        }
        profile
    };
    let report = cost::analyze_predicted(&input, &evidence, "cpu", Space::default(), &profile).unwrap();
    eprintln!("planned in {:.1?}", clock.elapsed());
    // `DOOP_ORACLE_ADVICE=<file>` writes the advised directives for adoption.
    if let Ok(path) = std::env::var("DOOP_ORACLE_ADVICE") { std::fs::write(path, &report.advice).unwrap(); }
    let listings = cost::listed(&input, &evidence, "cpu", Space::default(), Some(&profile)).unwrap();
    eprintln!("listed in {:.1?}", clock.elapsed());
    // Per rule hole, every valuation of the space with its estimate and step
    // estimates, then any kept valuation outside it; the kept valuations'
    // places in that list.
    type Valuation<'a> = (&'a Candidate, Option<f64>, &'a [Option<f64>]);
    // Variable orders (`cost::variables`) from degree facts collected over
    // the run's relations with each row's stage (PS-Collect): planned, and,
    // with `DOOP_HAND_ORDERS=<SRDatalog rules.txt>`, the hand order of the
    // same variant under the same lowering.
    let hand_orders: BTreeMap<String, Vec<String>> = std::env::var("DOOP_HAND_ORDERS").ok().map(|path| {
        let mut orders: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut current = String::new();
        for line in std::fs::read_to_string(path).unwrap().lines() {
            if let Some(name) = line.strip_prefix("rule ") { current = name.trim().to_string(); orders.entry(current.clone()).or_default(); continue; }
            let step = line.trim_start();
            let Some(rest) = ["join ", "cart ", "scan "].iter().find_map(|kind| step.strip_prefix(kind)) else { continue };
            let Some(names) = rest.split(':').next() else { continue };
            orders.entry(current.clone()).or_default().extend(names.trim().trim_matches(|c| c == '(' || c == ')').split_whitespace().map(|name| format!("v_{name}")));
        }
        orders
    }).unwrap_or_default();
    let (measured, requested, collecting) = collect_degrees(&input, &evidence, &model, &run, std::thread::available_parallelism().map_or(1, usize::from));
    eprintln!("{requested} degree requests collected in {collecting:.1?}");
    let planner = cost::variables::Planner::new(&input, &measured).unwrap();
    let selected: Option<Vec<String>> = std::env::var("DOOP_ORACLE_HOLES").ok().map(|list| list.split(',').map(str::to_owned).collect());
    let label = |hole: usize| match &input.holes[hole].origin {
        Origin::Rule { source_label, variant, seed, .. } => format!("{}{}#{variant}", source_label.clone().unwrap_or_default(), if *seed { " seed" } else { "" }),
        _ => format!("hole {hole}"),
    };
    let variable_plans: Vec<[Option<cost::variables::Planned>; 2]> = input.holes.iter().enumerate().zip(&report.holes).map(|((at, hole), advice)| {
        let Origin::Rule { source_label, seed, .. } = &hole.origin else { return [None, None] };
        if selected.as_ref().is_some_and(|only| !only.iter().any(|part| label(at).contains(part.as_str()))) { return [None, None]; }
        let changing = advice.candidates.iter().find(|record| record.plan.current)
            .and_then(|record| record.plan.steps.iter().filter_map(|step| step.read.as_ref()).find(|read| read.version == Version::Delta))
            .map(|read| read.source_occurrence);
        // A rule of several heads is one hand rule: `Rule_k` plans as `Rule`.
        let rule = source_label.clone().unwrap_or_default();
        let named = |rule: &str| match (seed, changing) { (false, Some(at)) => format!("{rule}_D{at}"), _ => rule.to_string() };
        let single = rule.rsplit_once('_').filter(|(_, head)| !head.is_empty() && head.chars().all(|c| c.is_ascii_digit())).map(|(rule, _)| rule.to_string());
        let hand = hand_orders.get(&named(&rule)).or_else(|| single.as_ref().and_then(|rule| hand_orders.get(&named(rule))));
        [planner.plan(at).unwrap(), hand.and_then(|order| planner.lower(at, order).unwrap())]
    }).collect();
    eprintln!("variable orders planned in {:.1?}", clock.elapsed());
    // `DOOP_VARS_ADVICE=<prefix>` writes the planned and the hand variable
    // orders as advice the production compiler accepts
    // (`<prefix>.planned.dl`, `<prefix>.hand.dl`), names every variant it
    // rejects alone (`.rejected`), and stops.
    if let Ok(prefix) = std::env::var("DOOP_VARS_ADVICE") {
        for (slot, role) in ["planned", "hand"].iter().enumerate() {
            let valuations: BTreeMap<usize, Candidate> = variable_plans.iter().enumerate()
                .filter_map(|(hole, plans)| Some((hole, plans[slot].as_ref()?.candidate.clone()))).collect();
            let advised = planner.adopt(&valuations).unwrap();
            let text: String = advised.plans.iter().map(|plan| cost::advice::AdviceList(vec![plan.clone()]).to_token_stream().to_string() + "\n").collect();
            std::fs::write(format!("{prefix}.{role}.dl"), format!("// {role} variable orders of {}; {} variants advised, {} rejected by production\n{text}",
                planned_dir.display(), advised.adopted.len(), advised.rejected.len())).unwrap();
            std::fs::write(format!("{prefix}.{role}.rejected"), advised.rejected.iter().map(|(hole, reason)| format!("{}\t{reason}\n", label(*hole))).collect::<String>()).unwrap();
            eprintln!("{role}: {} variants advised, {} rejected", advised.adopted.len(), advised.rejected.len());
        }
        return;
    }
    struct Judging<'a> { hole: &'a export::Hole, advice: &'a cost::HoleAdvice, plans: Vec<Valuation<'a>>, kept: Vec<usize>, fixed: Vec<(usize, i32)>, ops: Vec<Option<Vec<Op>>>,
        variables: [Option<usize>; 2], planned: &'a [Option<cost::variables::Planned>; 2] }
    let judging: Vec<Judging> = input.holes.iter().zip(&report.holes).zip(&listings).zip(&variable_plans).filter(|(((hole, _), _), _)| matches!(hole.origin, Origin::Rule { .. }))
        .map(|(((hole, advice), listing), planned)| {
            let mut plans: Vec<Valuation> = listing.valuations.iter().map(|valued| (&valued.plan, valued.estimate, valued.steps.as_slice())).collect();
            let kept = advice.candidates.iter().map(|record| match plans.iter().position(|(plan, _, _)| plan.steps == record.plan.steps) {
                Some(at) => at,
                None => { plans.push((&record.plan, record.estimate.map(|work| work as f64), &[])); plans.len() - 1 }
            }).collect();
            let mut variables = [None; 2];
            for (slot, planned) in planned.iter().enumerate() {
                let Some(planned) = planned else { continue };
                variables[slot] = Some(match plans.iter().position(|(plan, _, _)| plan.steps == planned.candidate.steps) {
                    Some(at) => at,
                    None => { plans.push((&planned.candidate, None, &[])); plans.len() - 1 }
                });
            }
            let mut slots = BTreeMap::new();
            let fixed = fixed(hole, &model, &mut slots);
            let bound: BTreeSet<usize> = fixed.iter().map(|(slot, _)| *slot).collect();
            let ops = plans.iter().map(|(plan, _, _)| ops(plan, hole, &model, &mut slots.clone(), bound.clone())).collect();
            Judging { hole, advice, plans, kept, fixed, ops, variables, planned }
        }).collect();
    let mut run = run;
    for judged in &judging {
        for op in judged.ops.iter().flatten().flatten() {
            match op {
                Op::Read { relation, key, .. } => run.tables[*relation].index(key),
                Op::Absent { relation, columns, .. } => run.tables[*relation].index(columns),
                _ => {}
            }
        }
    }
    let run = run;
    eprintln!("indexed in {:.1?}", clock.elapsed());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let lines = std::sync::Mutex::new(Vec::new());
    // `DOOP_ORACLE_HOLES` (comma-separated substrings of variant labels)
    // judges only those holes and writes the kept valuations' steps.
    let only: Option<Vec<String>> = std::env::var("DOOP_ORACLE_HOLES").ok().map(|list| list.split(',').map(str::to_owned).collect());
    let details = std::sync::Mutex::new(Vec::new());
    // The chain models' statistics, and their per-step study of the written,
    // optimal and chosen valuations.
    let pairs = Pairs::default();
    let studies = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| for _ in 0..24 { scope.spawn(|| loop {
        let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let Some(Judging { hole, advice, plans, kept, fixed, ops, variables, planned }) = judging.get(at) else { break };
        let Origin::Rule { source_label, variant, seed, .. } = &hole.origin else { unreachable!() };
        let label = format!("{}{}#{variant}", source_label.clone().unwrap_or_default(), if *seed { " seed" } else { "" });
        if only.as_ref().is_some_and(|only| !only.iter().any(|part| label.contains(part.as_str()))) { continue; }
        let steady = !seed;
        let delta = plans[0].0.steps.iter().filter_map(|step| step.read.as_ref()).find(|read| read.version == Version::Delta)
            .and_then(|read| model.names.iter().position(|name| *name == read.logical_relation));
        let component = delta.and_then(|relation| run.component[relation]);
        let members: Vec<usize> = component.map_or(Vec::new(), |id| (0..model.names.len()).filter(|&relation| run.component[relation] == Some(id)).collect());
        let rounds = component.map_or(1, |id| run.rounds[id]);
        // Valuations in the order of their estimates, so the cap tightens
        // early; the kept ones are counted whatever the cap.
        let mut order: Vec<usize> = (0..plans.len()).collect();
        order.sort_by(|&left, &right| plans[left].1.unwrap_or(f64::INFINITY).total_cmp(&plans[right].1.unwrap_or(f64::INFINITY)));
        let mut exact: Vec<Option<u64>> = vec![None; plans.len()];
        let mut counted: Vec<Option<(Vec<u64>, u64, Vec<u64>)>> = vec![None; plans.len()];
        let mut best = u64::MAX;
        let mut unsupported = None;
        // Size and elephant-key heuristics over the atom orders, as the
        // hand-written SRDatalog orders were chosen: a steady variant reads
        // its delta first; each read is ranked by its fan-out on the key
        // bound before it, compared step by step (lexicographically).
        let fanout = |step: &cost::CostStep| -> Option<(f64, f64)> {
            let read = step.read.as_ref()?;
            if !step.introduces { return Some((1.0, 1.0)); }
            let relation = model.names.iter().position(|name| *name == read.logical_relation)?;
            let table = &run.tables[relation];
            let key: Vec<usize> = read.columns.iter().filter(|column| column.variable.as_ref().is_none_or(|name| step.bound_before.contains(name)))
                .map(|column| column.source_column).collect();
            if key.is_empty() { return Some((table.rows.len() as f64, table.rows.len() as f64)); }
            let groups = table.indexes.get(&key)?;
            let largest = groups.values().map(Vec::len).max().unwrap_or(0) as f64;
            Some((largest, table.rows.len() as f64 / groups.len().max(1) as f64))
        };
        // The driver (first read) costs its rows once if it is the delta,
        // and its rows every round otherwise; every later read its largest
        // group on the bound key. `delta_first` keeps only delta drivers.
        let profile = |at: usize, delta_first: bool| -> Option<Vec<f64>> {
            let plan = plans[at].0;
            if !plan.is_atom_order() || !plan.unbounded.is_empty() { return None; }
            let first = plan.steps.iter().find_map(|step| step.read.as_ref())?;
            if steady && delta_first && first.version != Version::Delta { return None; }
            let mut sequence: Vec<f64> = plan.steps.iter().filter(|step| step.read.is_some()).map(|step| fanout(step).map(|(largest, _)| largest)).collect::<Option<_>>()?;
            if steady && first.version != Version::Delta { sequence[0] *= rounds as f64; }
            Some(sequence)
        };
        let pick = |delta_first: bool| -> Option<usize> {
            let profiles: Vec<Option<Vec<f64>>> = (0..plans.len()).map(|at| profile(at, delta_first)).collect();
            (0..plans.len()).filter(|&at| profiles[at].is_some()).min_by(|&left, &right| {
                let (l, r) = (profiles[left].as_ref().unwrap(), profiles[right].as_ref().unwrap());
                l.iter().zip(r).map(|(a, b)| a.total_cmp(b)).find(|o| o.is_ne()).unwrap_or(std::cmp::Ordering::Equal)
            })
        };
        let by_max = pick(true);
        let by_driver = pick(false);
        // Chain models: the valuation of least predicted frames plus its
        // exact index upkeep, under each way of taking a read's fan-out.
        let upkeeps: Vec<Option<u64>> = (0..plans.len()).map(|at| maintained(&input, advice, plans[at].0, &model, &run, rounds, steady)).collect();
        let visible = Visible::new(&run.tables, &members, if steady { delta } else { None }, rounds);
        let chain = |fan: Fan| -> (Vec<Option<f64>>, Option<usize>) {
            let totals: Vec<Option<f64>> = (0..plans.len()).map(|at| {
                let (frames, _) = predict(ops[at].as_ref()?, &run.tables, fixed, rounds, steady, &pairs, &visible, fan)?;
                Some(frames + upkeeps[at]? as f64)
            }).collect();
            let least = (0..plans.len()).filter(|&at| totals[at].is_some()).min_by(|&left, &right| totals[left].unwrap().total_cmp(&totals[right].unwrap()));
            (totals, least)
        };
        let (average_totals, by_average) = chain(Fan::Average);
        let (largest_totals, by_largest) = chain(Fan::Largest);
        let (pair_totals, by_pair) = chain(Fan::Pair);
        let (full_totals, by_full) = chain(Fan::Full);
        // Guarded choices around the written valuation: leave it only for a
        // predicted gain of half again (`margin`); and then only for a
        // valuation whose largest-group chain is no worse than its own (`guard`).
        let incumbent = advice.candidates.iter().position(|record| record.plan.current).map(|slot| kept[slot]);
        let guarded = |bounded: bool| -> Option<usize> {
            let incumbent = incumbent?;
            let (Some(own), Some(risk)) = (full_totals[incumbent], largest_totals[incumbent]) else { return Some(incumbent) };
            let least = (0..plans.len()).filter(|&at| full_totals[at].is_some() && (!bounded || largest_totals[at].is_some_and(|worst| worst <= risk)))
                .min_by(|&left, &right| full_totals[left].unwrap().total_cmp(&full_totals[right].unwrap()))?;
            Some(if full_totals[least].unwrap() * 1.5 <= own { least } else { incumbent })
        };
        let by_margin = guarded(false);
        let by_guard = guarded(true);
        let heuristic = [by_max, by_driver, by_average, by_largest, by_pair, by_full, by_margin, by_guard, variables[0], variables[1]];
        // A variable plan may demand an index of no recorded policy: its
        // frames are still counted, its upkeep unknown.
        let mut unpriced = vec![false; plans.len()];
        for (&candidate, keep) in order.iter().map(|at| (at, false)).chain(kept.iter().chain(heuristic.iter().flatten()).map(|at| (at, true))) {
            if exact[candidate].is_some() { continue; }
            let variable_plan = variables.contains(&Some(candidate));
            let (Some(steps), Some(upkeep)) = (ops[candidate].as_ref(), upkeeps[candidate].or(variable_plan.then_some(0))) else {
                unsupported = Some("a step the oracle cannot execute"); continue;
            };
            unpriced[candidate] = upkeeps[candidate].is_none();
            let cap = if keep { 1 << 36 } else { best.saturating_mul(4).clamp(1 << 24, 1 << 36) };
            if let Some((frames, per_step, arrive)) = work(steps, &run.tables, &members, rounds, steady, fixed, cap) {
                exact[candidate] = Some(frames + upkeep);
                counted[candidate] = Some((per_step, upkeep, arrive));
                if !unpriced[candidate] { best = best.min(frames + upkeep); }
            }
        }
        // The optimum: least exact work among the valuations whose upkeep is priced.
        let optimum = (0..plans.len()).filter(|&at| exact[at].is_some() && !unpriced[at]).min_by_key(|&at| exact[at]);
        {
            // Per valuation of interest: its totals under every model, and
            // per op the exact frames in and out beside each model's ratio.
            let written = advice.candidates.iter().position(|record| record.plan.current).map(|slot| kept[slot]);
            let mut text = String::new();
            let roles = [("written", written), ("optimal", optimum), ("by_average", by_average), ("by_largest", by_largest), ("by_pair", by_pair), ("by_full", by_full)];
            for (index, (role, at)) in roles.iter().enumerate() {
                let Some(at) = *at else { continue };
                let number = |value: Option<f64>| value.map_or("-".into(), |value| format!("{value:.0}"));
                text += &format!("plan\t{label}\t{role}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n", exact[at].map_or("-".into(), |work| work.to_string()), upkeeps[at].map_or("-".into(), |work| work.to_string()),
                    number(plans[at].1), number(average_totals[at]), number(largest_totals[at]), number(pair_totals[at]), number(full_totals[at]));
                if roles[..index].iter().any(|(_, earlier)| *earlier == Some(at)) { continue; }
                let (Some(steps), Some((per_step, _, arrive))) = (ops[at].as_ref(), counted[at].as_ref()) else { continue };
                let Some((_, ratios)) = predict(steps, &run.tables, fixed, rounds, steady, &pairs, &visible, Fan::Full) else { continue };
                for (depth, op) in steps.iter().enumerate() {
                    let (kind, relation, out) = match op {
                        Op::Read { relation, introduces: true, delta, .. } => (if *delta { "expand-delta" } else { "expand" }, model.names[*relation].as_str(), Some(per_step[depth])),
                        Op::Read { relation, delta, .. } => (if *delta { "lookup-delta" } else { "lookup" }, model.names[*relation].as_str(), arrive.get(depth + 1).copied()),
                        Op::Absent { relation, .. } => ("absent", model.names[*relation].as_str(), arrive.get(depth + 1).copied()),
                        Op::Check(_) => ("check", "", arrive.get(depth + 1).copied()),
                        Op::Restrict(_) => ("restrict", "", arrive.get(depth + 1).copied()),
                        Op::Pass | Op::Skip => continue,
                    };
                    text += &format!("step\t{label}\t{role}\t{depth}\t{kind}\t{relation}\t{}\t{}\t{}\t{}\t{}\t{}\n", arrive[depth], out.map_or("-".into(), |frames| frames.to_string()),
                        ratios[depth][0], ratios[depth][1], ratios[depth][2], ratios[depth][3]);
                }
            }
            studies.lock().unwrap().push((at, text));
        }
        let record = |at: usize| &advice.candidates[at];
        let current = (0..advice.candidates.len()).find(|&at| record(at).plan.current).unwrap();
        // The kept valuation the estimates rank first, before any screen.
        let ranked = (0..advice.candidates.len()).filter_map(|at| Some((record(at).estimate?, at))).min().map_or(current, |(_, at)| at);
        if only.is_some() {
            let mut text = format!("== {label} decision={:?} reason={}\n", advice.decision, advice.reason);
            // The kept valuations, the optimum, and the three of least estimate.
            let shown: Vec<usize> = kept.iter().copied().chain(optimum).chain(variables.iter().flatten().copied()).chain(order.iter().copied().take(3)).collect();
            for (index, &at) in shown.iter().enumerate() {
                if shown[..index].contains(&at) { continue; }
                let role = kept.iter().position(|&held| held == at).map_or(String::new(), |slot| format!(" kept#{slot}{}{} planned_estimate={}",
                    if slot == advice.selected { " SELECTED" } else { "" }, if record(slot).plan.current { " WRITTEN" } else { "" },
                    record(slot).estimate.map_or("-".into(), |work| work.to_string())));
                let certified = kept.iter().position(|&held| held == at).and_then(|slot| record(slot).cost.whole_run.as_ref()).map_or(f64::NAN, |whole| certificate(whole, &report, &evidence));
                text += &format!("  valuation {at}{role} estimate={:.0} certificate={certified:.0} exact={} upkeep={}\n",
                    plans[at].1.unwrap_or(f64::NAN), exact[at].map_or("-".into(), |work| work.to_string()), counted[at].as_ref().map_or("-".into(), |(_, upkeep, _)| upkeep.to_string()));
                // Inserted restrictions are ops without a step of their own.
                let op_of: Vec<usize> = ops[at].as_ref().map_or(Vec::new(), |ops| (0..ops.len()).filter(|&op| !matches!(ops[op], Op::Restrict(_))).collect());
                for (step_at, step) in plans[at].0.steps.iter().enumerate() {
                    let read = step.read.as_ref().map_or(String::new(), |read| format!(" {}{:?}{} cols={:?}", read.logical_relation, read.version,
                        if read.partial { " partial" } else { "" }, read.columns.iter().map(|column| (column.source_column, column.variable.clone().unwrap_or_default())).collect::<Vec<_>>()));
                    text += &format!("    {step_at}: clause {} {:?}{read} bound_before={:?} introduces={} estimate={} counted={}\n", step.clause, step.kind,
                        step.bound_before, step.introduces, plans[at].2.get(step_at).copied().flatten().map_or("-".into(), |work| format!("{work:.0}")),
                        counted[at].as_ref().map_or("-".into(), |(per_step, _, _)| op_of.get(step_at).and_then(|&op| per_step.get(op)).map_or("?".into(), |work| work.to_string())));
                }
            }
            details.lock().unwrap().push((at, text));
        }
        let sound = (0..advice.candidates.len()).filter_map(|slot| Some((record(slot).cost.whole_run.as_ref()?, exact[kept[slot]]?)))
            .all(|(whole, work)| certificate(whole, &report, &evidence) >= work as f64);
        let show = |work: Option<u64>| work.map_or("-".to_string(), |work| work.to_string());
        let line = format!("{label}\t{:?}\t{:?}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            advice.basis, advice.decision, plans.len(), exact.iter().filter(|work| work.is_some()).count(),
            show(exact[kept[current]]), show(exact[kept[advice.selected]]), show(exact[kept[ranked]]),
            if best == u64::MAX { "-".into() } else { best.to_string() },
            show(by_max.and_then(|at| exact[at])), show(by_driver.and_then(|at| exact[at])),
            show(by_average.and_then(|at| exact[at])), show(by_largest.and_then(|at| exact[at])), show(by_pair.and_then(|at| exact[at])),
            show(by_full.and_then(|at| exact[at])), show(by_margin.and_then(|at| exact[at])), show(by_guard.and_then(|at| exact[at])),
            show(variables[0].and_then(|at| exact[at])), show(variables[1].and_then(|at| exact[at])),
            sound, unsupported.unwrap_or(""));
        {
            // The variable orders (planned, by hand) beside the written and
            // optimal valuations: frames without index upkeep, the upkeep
            // where it is priced, and every index each demands with the
            // rows written into it.
            let mut text = String::new();
            let split = |at: Option<usize>| -> (String, String) {
                match at.and_then(|at| Some((at, exact[at]?, counted[at].as_ref()?.1))) {
                    Some((at, work, upkeep)) => ((work - upkeep).to_string(), if unpriced[at] { "?".into() } else { upkeep.to_string() }),
                    None => ("-".into(), "-".into()),
                }
            };
            let roles = [("written", Some(kept[current]), None), ("optimal", optimum, None), ("planned", variables[0], planned[0].as_ref()), ("hand", variables[1], planned[1].as_ref())];
            for (role, at, order) in roles {
                if at.is_none() && order.is_none() { continue; }
                let (frames, upkeep_rows) = split(at);
                let (names, estimate) = order.map_or((String::new(), String::new()), |order| (order.order.join(" "), format!("{:.0}", order.estimate)));
                text += &format!("{label}\t{role}\t{names}\t{estimate}\t{}\t{frames}\t{upkeep_rows}\n", at.map_or(0, |at| plans[at].0.steps.len()));
                let Some(at) = at else { continue };
                for demand in &plans[at].0.demands {
                    let rows = upkeep(&input, demand, &model, &run, rounds, steady).map_or("?".into(), |rows| rows.to_string());
                    text += &format!("{label}\tdemand:{role}\t{} {:?} {:?}\t{rows}\n", demand.logical_relation, demand.version, demand.key);
                }
            }
            studies.lock().unwrap().push((at + judging.len(), text));
        }
        lines.lock().unwrap().push((at, line));
    }); });
    let mut lines = lines.into_inner().unwrap();
    lines.sort();
    let header = "variant\tbasis\tdecision\tvaluations\tcounted\twritten\tselected\tranked\toptimal\tby_max\tby_driver\tby_average\tby_largest\tby_pair\tby_full\tby_margin\tby_guard\tvars\tvars_hand\tsound\tnote";
    let table = std::iter::once(header.to_string()).chain(lines.into_iter().map(|(_, line)| line)).collect::<Vec<_>>().join("\n");
    eprintln!("judged in {:.1?}", clock.elapsed());
    if let Ok(out) = std::env::var("DOOP_ORACLE_OUT") {
        std::fs::write(&out, format!("{table}\n")).unwrap();
        std::fs::write(format!("{out}.counts"), counts).unwrap();
        let mut details = details.into_inner().unwrap();
        details.sort();
        if !details.is_empty() { std::fs::write(format!("{out}.detail"), details.into_iter().map(|(_, text)| text).collect::<String>()).unwrap(); }
        let mut studies = studies.into_inner().unwrap();
        studies.sort();
        let count = judging.len();
        std::fs::write(format!("{out}.models"), studies.iter().filter(|(at, _)| *at < count).map(|(_, text)| text.as_str()).collect::<String>()).unwrap();
        std::fs::write(format!("{out}.vars"), studies.iter().filter(|(at, _)| *at >= count).map(|(_, text)| text.as_str()).collect::<String>()).unwrap();
    }
    println!("{table}");
}
