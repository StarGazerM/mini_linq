//! IX → Physical: choose operators and retention using plan reads only.
//! Host renames are reusable pipe reads. Literal selections are one-key point
//! reads: native leading-prefix dispatch or scan/filter, with no index build.
//! Meaning is bottom + Set. Opaque partial Rust bind/guard nodes retain their
//! ancestry chain; relational steps may move earlier, never later across them.
//! I/O, mutation and evaluation counts are outside the denotation.
//! Canonicality follows only from plan operators and explicit canon boundaries.
//!
//! Relational equality is Leibniz identity, decided by admissible column `==`.
//! Functional extensionality of shared-reference Rust inputs permits canonical
//! projection whenever free variables remain; see the premises in
//! `cq-syntax`.

use std::collections::{BTreeMap, BTreeSet};

use cq_lang_ix as ix;
use cq_lang_ix::ra::Operator as Op;
use cq_lang_ix::syntax::symbol_name;
use cq_lang_physical as physical;
use syn::Ident;

/// How one result is read, gathered from every reader in RA order.
#[derive(Default)]
struct Reads {
    /// Operand references, the return counted once.
    count: usize,
    /// Key columns of every `probe`/`semi` read of the result.
    keyed: Vec<Vec<String>>,
    /// Key columns of every `merge` read of the result.
    merged: Vec<Vec<String>>,
}

/// Lower IX to Physical with deterministic defaults, then check the Physical contract.
pub fn compile(source: &ix::Program) -> syn::Result<physical::Program> {
    ix::contract::check(source)?;
    let info = ix::ra::contract::check(&source.ra)?;
    let operations: Vec<_> = source
        .ra
        .plan
        .definitions
        .iter()
        .map(|d| (d.result.clone(), d.operator.clone()))
        .collect();
    let returned = source.ra.plan.output.input.clone();
    let (operators, reads) = operators(source, &info, &operations)?;
    let modes = modes(&operations, &operators, &reads);
    let unique = canonical(&info, &operations, &returned);
    let definitions = operations
        .iter()
        .zip(modes)
        .zip(operators)
        .map(|(((name, _op), mode), operator)| physical::Definition {
            name: name.clone(),
            eq_token: Default::default(),
            mode: if matches!(mode, physical::Mode::Let(_))
                && info[&symbol_name(name)].heading.is_empty()
            {
                physical::Mode::Exists(Default::default())
            } else {
                mode
            },
            canon: (!unique.contains(&symbol_name(name)) && name == &returned)
                .then(Default::default),
            operator,
            semi_token: Default::default(),
        })
        .collect();
    let target = physical::Program {
        physical_token: Default::default(),
        brace: Default::default(),
        program: physical::Body {
            declarations: source.ra.plan.declarations.clone(),
            definitions,
            return_token: Default::default(),
            returned,
            as_token: Default::default(),
            head: source.ra.plan.output.head.clone(),
            return_semi: Default::default(),
        },
    };
    physical::contract::check(&target)?;
    check(source, &target)?;
    Ok(target)
}

fn operators(
    source: &ix::Program,
    info: &ix::ra::Analysis,
    operations: &[(Ident, Op)],
) -> syn::Result<(Vec<physical::Operator>, BTreeMap<String, Reads>)> {
    let mut reads: BTreeMap<String, Reads> = operations
        .iter()
        .map(|(id, _)| (symbol_name(id), Reads::default()))
        .collect();
    let mut operators = Vec::new();
    for (_, op) in operations {
        let operator = match op {
            Op::Unit(u) => physical::Operator::Unit(u.clone()),
            Op::Rename(r) => physical::Operator::Rename(r.clone()),
            Op::NaturalJoin(_) | Op::Semijoin(_) => {
                let (left, right) = match op {
                    Op::NaturalJoin(j) => (&j.left, &j.right),
                    Op::Semijoin(j) => (&j.left, &j.right),
                    _ => unreachable!(),
                };
                let lh = &info[&symbol_name(left)];
                let rh = &info[&symbol_name(right)];
                let columns: Vec<String> = lh.heading.intersection(&rh.heading).cloned().collect();
                scan(&mut reads, left);
                let keyed = physical::Keyed {
                    choice: Some(if matches!(op, Op::Semijoin(_)) {
                        physical::Choice::Eager(Default::default())
                    } else {
                        physical::Choice::First(Default::default())
                    }),
                    bracket: Default::default(),
                    columns: columns.iter().map(|c| lh.bindings[c].clone()).collect(),
                    left: left.clone(),
                    comma: Default::default(),
                    right: right.clone(),
                };
                if matches!(op, Op::Semijoin(_)) {
                    probe(&mut reads, right, columns);
                    physical::Operator::Semi(Default::default(), keyed)
                } else {
                    probe(&mut reads, right, columns);
                    physical::Operator::Probe(Default::default(), keyed)
                }
            }
            Op::Select(s) => {
                scan(&mut reads, &s.input);
                if s.condition.point_columns().is_some() {
                    physical::Operator::Point(Default::default(), s.clone())
                } else if s.condition.range_bounds().is_some() {
                    physical::Operator::Range(Default::default(), s.clone())
                } else {
                    physical::Operator::Select(s.clone())
                }
            }
            Op::Bind(b) => {
                scan(&mut reads, &b.input);
                physical::Operator::Bind(b.clone())
            }
            Op::Guard(g) => {
                scan(&mut reads, &g.input);
                physical::Operator::Guard(g.clone())
            }
            Op::Project(p) => {
                scan(&mut reads, &p.input);
                physical::Operator::Project(p.clone())
            }
        };
        operators.push(operator);
    }
    scan(&mut reads, &source.ra.plan.output.input);
    Ok((operators, reads))
}

fn modes(
    operations: &[(Ident, Op)],
    operators: &[physical::Operator],
    reads: &BTreeMap<String, Reads>,
) -> Vec<physical::Mode> {
    let own_merge: BTreeMap<String, Vec<String>> = operations
        .iter()
        .zip(operators)
        .filter_map(|((name, _), op)| match op {
            physical::Operator::Merge(_, k) => Some((
                symbol_name(name),
                k.columns.iter().map(symbol_name).collect(),
            )),
            _ => None,
        })
        .collect();
    operations
        .iter()
        .map(|(name, op)| {
            let name = symbol_name(name);
            let result = &reads[&name];
            match op {
                Op::Rename(_) => physical::Mode::Pipe(Default::default()),
                _ if !result.keyed.is_empty()
                    || result.count >= 2
                    || result
                        .merged
                        .iter()
                        .any(|c| own_merge.get(&name) != Some(c)) =>
                {
                    physical::Mode::Let(Default::default())
                }
                _ => physical::Mode::Pipe(Default::default()),
            }
        })
        .collect()
}

fn canonical(
    info: &ix::ra::Analysis,
    operations: &[(Ident, Op)],
    returned: &Ident,
) -> BTreeSet<String> {
    let mut unique: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut inherited = BTreeSet::new();
    for (name, op) in operations {
        let name = symbol_name(name);
        let heading: Vec<_> = info[&name].heading.iter().cloned().collect();
        let key = match op {
            Op::Unit(_) => Some(Vec::new()),
            Op::Rename(_) => None,
            Op::NaturalJoin(j) => match (
                unique.get(&symbol_name(&j.left)),
                unique.get(&symbol_name(&j.right)),
            ) {
                (Some(l), Some(r)) => {
                    let mut key: Vec<_> = l.iter().chain(r).cloned().collect();
                    key.sort();
                    key.dedup();
                    Some(key)
                }
                _ => None,
            },
            Op::Semijoin(j) => unique.get(&symbol_name(&j.left)).cloned(),
            Op::Select(s) => unique.get(&symbol_name(&s.input)).cloned(),
            Op::Bind(b) => unique.get(&symbol_name(&b.input)).cloned(),
            Op::Guard(g) => unique.get(&symbol_name(&g.input)).cloned(),
            Op::Project(p) => unique
                .get(&symbol_name(&p.input))
                .filter(|key| key.iter().all(|c| heading.contains(c)))
                .cloned(),
        };
        if let Some(key) = key {
            inherited.insert(name.clone());
            unique.insert(name, key);
        } else if name == symbol_name(returned) {
            unique.insert(name, heading);
        }
    }
    inherited
}

fn scan(reads: &mut BTreeMap<String, Reads>, operand: &Ident) {
    reads
        .get_mut(&symbol_name(operand))
        .expect("operand is a result")
        .count += 1;
}

fn probe(reads: &mut BTreeMap<String, Reads>, right: &Ident, columns: Vec<String>) {
    let right = reads
        .get_mut(&symbol_name(right))
        .expect("operand is a result");
    right.count += 1;
    right.keyed.push(columns);
}

pub use physical::check_rewrite;
pub use physical::erase;

/// Formal rule CK-Demands: Require each stream/const/range keyed-read descriptor in source IX.
fn check_demands(source: &ix::Program, target: &physical::Program) -> syn::Result<()> {
    let analysis = physical::contract::check(target)?;
    let demands: BTreeSet<_> = source
        .indexes
        .iter()
        .map(|i| ix::signature(&i.requirement))
        .collect();
    for (input, reads) in &analysis.keyed_reads {
        for (reader, columns, supply) in reads {
            let definition = target
                .program
                .definitions
                .iter()
                .find(|d| symbol_name(&d.name) == *reader)
                .expect("reader exists");
            if matches!(&definition.operator, physical::Operator::Merge(_, k) if symbol_name(&k.left) == *input && k.left != k.right)
            {
                continue;
            }
            if !demands.contains(&(
                input.clone(),
                physical::ordinals(&analysis, input, columns),
                *supply,
            )) {
                return Err(syn::Error::new_spanned(
                    &target
                        .program
                        .definitions
                        .iter()
                        .find(|d| symbol_name(&d.name) == *reader)
                        .expect("reader exists")
                        .name,
                    format!(
                        "read `{reader}` needs demand `@{input}[{}] {supply:?}`",
                        columns.join(",")
                    ),
                ));
            }
        }
    }
    Ok(())
}

mod correspondence;
pub use correspondence::check;
