//! Test-only Stream semantics: bags of integer rows, cursors interpreted by
//! headings, no planning or lowering helpers. Compared against the RA
//! oracle, this is the semantic correspondence gate for IX → Stream.

use std::collections::{BTreeMap, BTreeSet};

use cq_ir::{cq, stream, symbol_name};

use super::oracle::{Database, evaluate_predicate};

type Row = Vec<i32>;
type Env = BTreeMap<String, i32>;

pub struct Evaluation {
    /// Every definition as a bag of rows in its sorted heading order.
    pub definitions: BTreeMap<String, Vec<Row>>,
    /// The returned definition as a set of rows in the RA output head order.
    pub output: BTreeSet<Row>,
}

pub fn evaluate(module: &stream::Module, database: &Database) -> Evaluation {
    let analysis = stream::contract::check(module).expect("stream contract");
    let program = &module.index_requirements.relational_plan.program;
    let mut relations: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    for input in program.inputs() {
        let name = symbol_name(&input.name);
        let declared: Vec<_> = input.columns.iter().map(|c| symbol_name(&c.name)).collect();
        let sorted = &analysis.headings[&name];
        let permutation: Vec<usize> = sorted
            .iter()
            .map(|c| declared.iter().position(|d| d == c).unwrap())
            .collect();
        relations.insert(
            name.clone(),
            database[&name]
                .iter()
                .map(|row| permutation.iter().map(|&i| row[i]).collect())
                .collect(),
        );
    }
    for definition in &module.program.definitions {
        let name = symbol_name(&definition.name);
        let rows = match &definition.realization {
            stream::Realization::Alias { host, .. } => {
                // The alias reads the host in its own sorted heading order.
                let host_heading = &analysis.headings[&symbol_name(host)];
                let heading = &analysis.headings[&name];
                let rename = module
                    .index_requirements
                    .relational_plan
                    .plan
                    .definitions
                    .iter()
                    .find(|d| symbol_name(&d.result) == name)
                    .and_then(|d| match &d.operator {
                        cq_ir::relational_plan::Operator::Rename(r) => Some(r),
                        _ => None,
                    })
                    .expect("alias realizes a rename");
                let permutation: Vec<usize> = heading
                    .iter()
                    .map(|target| {
                        let source = rename
                            .mapping
                            .iter()
                            .find(|m| symbol_name(&m.target) == *target)
                            .map(|m| symbol_name(&m.source))
                            .unwrap();
                        host_heading.iter().position(|h| *h == source).unwrap()
                    })
                    .collect();
                relations[&symbol_name(host)]
                    .iter()
                    .map(|row| permutation.iter().map(|&i| row[i]).collect())
                    .collect()
            }
            stream::Realization::Stream(streamed) => {
                let mut rows = Vec::new();
                body(&streamed.body, &Env::new(), &relations, &mut rows, false);
                rows
            }
        };
        relations.insert(name, rows);
    }
    let returned = symbol_name(&module.program.return_.name);
    let heading = &analysis.headings[&returned];
    let head: Vec<_> = module
        .index_requirements
        .relational_plan
        .plan
        .output
        .head
        .variables()
        .map(symbol_name)
        .collect();
    let output = relations[&returned]
        .iter()
        .map(|row| {
            head.iter()
                .map(|v| row[heading.iter().position(|h| h == v).unwrap()])
                .collect()
        })
        .collect();
    Evaluation {
        definitions: relations,
        output,
    }
}

fn bind(env: &mut Env, pattern: &syn::Pat, values: &[i32]) {
    let names = cq::rust::bindings(pattern);
    assert_eq!(names.len(), values.len(), "pattern arity");
    for (name, value) in names.iter().zip(values) {
        env.insert(symbol_name(name), *value);
    }
}

fn key_values(env: &Env, keys: &stream::Keys) -> Row {
    keys.variables
        .iter()
        .map(|k| env[&symbol_name(k)])
        .collect()
}

fn split(row: &[i32], columns: &[usize]) -> (Row, Row) {
    let key = columns.iter().map(|&c| row[c]).collect();
    let payload = row
        .iter()
        .enumerate()
        .filter(|(i, _)| !columns.contains(i))
        .map(|(_, v)| *v)
        .collect();
    (key, payload)
}

fn body(
    body: &stream::Body,
    env: &Env,
    relations: &BTreeMap<String, Vec<Row>>,
    out: &mut Vec<Row>,
    support: bool,
) {
    match body {
        stream::Body::For(f) => match &f.cursor {
            stream::Cursor::Scan { relation, .. } => {
                for row in &relations[&symbol_name(relation)] {
                    let mut env = env.clone();
                    bind(&mut env, &f.pattern, row);
                    self::body(&f.body, &env, relations, out, support);
                }
            }
            stream::Cursor::Seek {
                relation,
                columns,
                keys,
                ..
            } => {
                let probe = key_values(env, keys);
                let columns = columns.ordinals();
                for row in &relations[&symbol_name(relation)] {
                    let (key, payload) = split(row, &columns);
                    if key == probe {
                        let mut env = env.clone();
                        bind(&mut env, &f.pattern, &payload);
                        self::body(&f.body, &env, relations, out, support);
                    }
                }
            }
            stream::Cursor::Order { .. } => unreachable!("order only inside co"),
        },
        stream::Body::Co(c) => {
            // Group every cursor by key; intersect keys; cross payloads.
            let groups: Vec<(&syn::Pat, BTreeMap<Row, Vec<Row>>)> = c
                .cursors
                .iter()
                .map(|bound| {
                    let stream::Cursor::Order {
                        relation, columns, ..
                    } = &bound.cursor
                    else {
                        unreachable!()
                    };
                    let columns = columns.ordinals();
                    let mut groups: BTreeMap<Row, Vec<Row>> = BTreeMap::new();
                    for row in &relations[&symbol_name(relation)] {
                        let (key, payload) = split(row, &columns);
                        groups.entry(key).or_default().push(payload);
                    }
                    (&bound.pattern, groups)
                })
                .collect();
            let keys: BTreeSet<Row> = groups[0].1.keys().cloned().collect();
            for key in keys {
                if !groups.iter().all(|(_, g)| g.contains_key(&key)) {
                    continue;
                }
                let mut envs = vec![{
                    let mut env = env.clone();
                    bind(&mut env, &c.key, &key);
                    env
                }];
                for (pattern, group) in &groups {
                    envs = envs
                        .iter()
                        .flat_map(|env| {
                            group[&key].iter().map(move |payload| {
                                let mut env = env.clone();
                                bind(&mut env, pattern, payload);
                                env
                            })
                        })
                        .collect();
                }
                for env in envs {
                    self::body(&c.body, &env, relations, out, support);
                }
            }
        }
        stream::Body::If(i) => {
            let pass = match &i.guard {
                stream::Guard::Expr(p) => evaluate_predicate(&p.expr, env),
                stream::Guard::Contains {
                    relation,
                    columns,
                    keys,
                    ..
                } => {
                    let probe = key_values(env, keys);
                    let columns = columns.ordinals();
                    relations[&symbol_name(relation)]
                        .iter()
                        .any(|row| split(row, &columns).0 == probe)
                }
            };
            if pass {
                self::body(&i.body, env, relations, out, support);
            }
        }
        stream::Body::Let(_) => panic!("Rust computations are tested by compiled end-to-end tests"),
        stream::Body::Support(s) => {
            let mut inner = Vec::new();
            self::body(&s.body, env, relations, &mut inner, true);
            for row in inner {
                if !out.contains(&row) {
                    out.push(row);
                }
            }
        }
        stream::Body::Unit(u) => self::body(&u.body, env, relations, out, support),
        stream::Body::Emit(e) => {
            let row: Row = e.variables.iter().map(|v| env[&symbol_name(v)]).collect();
            if !support || !out.contains(&row) {
                out.push(row);
            }
        }
    }
}
