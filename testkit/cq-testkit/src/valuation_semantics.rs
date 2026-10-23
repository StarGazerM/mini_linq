//! Independent recursive interpretation of authored VO steps and reductions.
//! Union evaluates each arm in the row context and projects branch locals;
//! fallback selects the first nonempty arm. No compiler or CQ body interpreter
//! is called. Integer comparison guards are total; computed Rust is tested by
//! compiled written-order refinement instead.

use crate::{values::*, vo};
use std::collections::{BTreeMap, BTreeSet};
use vo::{
    cq,
    syntax::{Occurrence, symbol_name},
};

type Tables = BTreeMap<Occurrence, Relation>;

/// Execute a contract-valid valuation program over finite integer tables.
pub fn evaluate(program: &vo::Program, database: &Database) -> Rows {
    assert!(
        program.cq.query.group.is_none(),
        "grouped folds use compiled conformance targets"
    );
    crate::cq_semantics::validate_database(&program.cq, database);
    let mut tables = Tables::new();
    collect(&program.cq.query.body, None, database, &mut tables);
    let unit = Relation {
        heading: BTreeSet::new(),
        rows: BTreeSet::from([Binding::new()]),
    };
    execute(
        &program.valuation.steps,
        &program.cq.query.body,
        &program.valuation.steps,
        unit,
        &mut tables,
        database,
    )
    .output(
        &program
            .cq
            .query
            .head
            .variables()
            .map(symbol_name)
            .collect::<Vec<_>>(),
    )
}

fn collect(
    body: &vo::syntax::CommaList<cq::BodyItem>,
    parent: Option<(&Occurrence, usize)>,
    database: &Database,
    tables: &mut Tables,
) {
    for (i, item) in body.iter().enumerate() {
        let path = parent.map_or_else(|| Occurrence::new(i), |(p, j)| p.child(j, i));
        match item {
            cq::BodyItem::Positive { atom } => {
                let columns: Vec<_> = atom.variables().map(symbol_name).collect();
                tables.insert(
                    path,
                    Relation {
                        heading: columns.iter().cloned().collect(),
                        rows: database[&symbol_name(&atom.relation)]
                            .iter()
                            .filter_map(|row| {
                                let mut binding = Binding::new();
                                for (argument, value) in atom.args.iter().zip(row) {
                                    if let Some(variable) = cq::rust::variable(argument) {
                                        if binding
                                            .insert(symbol_name(variable), *value)
                                            .is_some_and(|previous| previous != *value)
                                        {
                                            return None;
                                        }
                                    } else if evaluate_integer(argument, &Binding::new()) != *value
                                    {
                                        return None;
                                    }
                                }
                                Some(binding)
                            })
                            .collect(),
                    },
                );
            }
            cq::BodyItem::Block(block) => {
                for (j, arm) in block.branches.iter().enumerate() {
                    collect(&arm.body, Some((&path, j)), database, tables);
                }
            }
            _ => (),
        }
    }
}

fn find_block<'a>(steps: &'a [vo::Step], path: &Occurrence) -> Option<&'a vo::blocks::Block> {
    for step in steps {
        if let vo::Step::Block(block, _) = step {
            if block.source.clause == *path {
                return Some(block);
            }
            for arm in &block.branches {
                if let Some(block) = find_block(&arm.steps, path) {
                    return Some(block);
                }
            }
        }
    }
    None
}

fn bound(
    body: &vo::syntax::CommaList<cq::BodyItem>,
    mut names: BTreeSet<String>,
) -> BTreeSet<String> {
    for item in body {
        match item {
            cq::BodyItem::Positive { atom } => names.extend(atom.variables().map(symbol_name)),
            cq::BodyItem::Block(block) => {
                let branches = block
                    .branches
                    .iter()
                    .map(|arm| bound(&arm.body, names.clone()))
                    .collect::<Vec<_>>();
                if let Some(first) = branches.first() {
                    names.extend(
                        first
                            .iter()
                            .filter(|name| branches.iter().all(|arm| arm.contains(*name)))
                            .cloned(),
                    );
                }
            }
            _ => (),
        }
    }
    names
}

fn source_context(
    body: &vo::syntax::CommaList<cq::BodyItem>,
    path: &[String],
    mut names: BTreeSet<String>,
) -> BTreeSet<String> {
    let clause: usize = path[0].parse().unwrap();
    let prefix = body.iter().take(clause).cloned().collect();
    names = bound(&prefix, names);
    if path.len() == 1 {
        return names;
    }
    let cq::BodyItem::Block(block) = &body[clause] else {
        panic!("source path")
    };
    source_context(
        &block.branches[path[1].parse::<usize>().unwrap()].body,
        &path[2..],
        names,
    )
}

fn source_block<'a>(
    body: &'a vo::syntax::CommaList<cq::BodyItem>,
    path: &[String],
) -> &'a cq::Block {
    let cq::BodyItem::Block(block) = &body[path[0].parse::<usize>().unwrap()] else {
        panic!("source block")
    };
    if path.len() == 1 {
        block
    } else {
        source_block(
            &block.branches[path[1].parse::<usize>().unwrap()].body,
            &path[2..],
        )
    }
}
fn used(body: &vo::syntax::CommaList<cq::BodyItem>) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for item in body {
        match item {
            cq::BodyItem::Positive { atom } => names.extend(atom.variables().map(symbol_name)),
            cq::BodyItem::Negation(negative) => {
                names.extend(negative.atom.variables().map(symbol_name));
            }
            cq::BodyItem::Equality(eq) => names.extend(eq.variables().map(symbol_name)),
            cq::BodyItem::Constraint(c) => names.extend(c.variables().map(symbol_name)),
            cq::BodyItem::Filter(f) => {
                names.extend(vo::syntax::rust::free_variables(&f.condition).names)
            }
            cq::BodyItem::Block(block) => {
                for arm in &block.branches {
                    names.extend(used(&arm.body));
                }
            }
            _ => (),
        }
    }
    names
}

fn derive(
    block: &vo::blocks::Block,
    body: &vo::syntax::CommaList<cq::BodyItem>,
    root: &[vo::Step],
    prefix: Relation,
    tables: &Tables,
    database: &Database,
) -> Relation {
    let source = source_block(body, &block.source.clause.0);
    let context = source_context(body, &block.source.clause.0, BTreeSet::new());
    let references: BTreeSet<_> = source
        .branches
        .iter()
        .flat_map(|arm| used(&arm.body))
        .collect();
    let free: BTreeSet<_> = context.intersection(&references).cloned().collect();
    let bindings = source
        .branches
        .iter()
        .map(|arm| bound(&arm.body, BTreeSet::new()))
        .collect::<Vec<_>>();
    let mut selected = free.clone();
    if let Some(first) = bindings.first() {
        selected.extend(
            first
                .iter()
                .filter(|v| bindings.iter().all(|arm| arm.contains(*v)))
                .cloned(),
        );
    }
    let heading = prefix
        .heading
        .union(&selected)
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut answer = Relation {
        heading: heading.clone(),
        rows: BTreeSet::new(),
    };
    let mut originals: Vec<Relation> = Vec::new();
    for arm in &block.branches {
        let result = execute(
            &arm.steps,
            body,
            root,
            prefix.clone(),
            &mut tables.clone(),
            database,
        )
        .project(heading.clone());
        for row in &result.rows {
            if block.kind == cq::blocks::Kind::Union
                || !originals.iter().any(|earlier| {
                    earlier
                        .rows
                        .iter()
                        .any(|witness| free.iter().all(|v| row[v] == witness[v]))
                })
            {
                answer.rows.insert(row.clone());
            }
        }
        originals.push(result);
    }
    answer
}

fn table(
    path: &Occurrence,
    body: &vo::syntax::CommaList<cq::BodyItem>,
    root: &[vo::Step],
    prefix: &Relation,
    tables: &Tables,
    database: &Database,
) -> Relation {
    if let Some(relation) = tables.get(path) {
        return relation.clone();
    }
    let block = find_block(root, path).expect("declared block occurrence");
    // The support is a relation on the block heading. Supply only columns that
    // are already in the row context; generators inside arms supply the rest.
    let source = source_block(body, &path.0);
    let references: BTreeSet<_> = source
        .branches
        .iter()
        .flat_map(|arm| used(&arm.body))
        .collect();
    let required: BTreeSet<_> = source_context(body, &path.0, BTreeSet::new())
        .intersection(&references)
        .cloned()
        .collect();
    let domain = if required.is_subset(&prefix.heading) {
        prefix.clone()
    } else {
        let mut domain = Relation {
            heading: BTreeSet::new(),
            rows: BTreeSet::from([Binding::new()]),
        };
        let mut scope = body;
        let mut parent = None;
        for (depth, pair) in path.0.chunks(2).enumerate() {
            let clause = pair[0].parse::<usize>().unwrap();
            for (c, item) in scope.iter().take(clause).enumerate() {
                let occurrence = parent.as_ref().map_or_else(
                    || Occurrence::new(c),
                    |(p, j): &(Occurrence, usize)| p.child(*j, c),
                );
                match item {
                    cq::BodyItem::Positive { .. } => domain = domain.join(&tables[&occurrence]),
                    cq::BodyItem::Equality(eq) => {
                        domain.rows.retain(|row| evaluate_equality(eq, row))
                    }
                    cq::BodyItem::Constraint(c) => {
                        domain.rows.retain(|row| evaluate_constraint(c, row))
                    }
                    cq::BodyItem::Negation(negative) => {
                        domain
                            .rows
                            .retain(|binding| !negative_matches(negative, binding, database));
                    }
                    _ => (),
                }
            }
            if pair.len() == 2 {
                let cq::BodyItem::Block(block) = &scope[clause] else {
                    panic!("domain scope")
                };
                let branch = pair[1].parse::<usize>().unwrap();
                scope = &block.branches[branch].body;
                parent = Some((Occurrence(path.0[..depth * 2 + 1].to_vec()), branch));
            }
        }
        domain
    };
    derive(
        block,
        body,
        root,
        domain.project(required),
        tables,
        database,
    )
}

fn execute(
    steps: &[vo::Step],
    body: &vo::syntax::CommaList<cq::BodyItem>,
    root: &[vo::Step],
    mut result: Relation,
    tables: &mut Tables,
    database: &Database,
) -> Relation {
    for step in steps {
        match step {
            vo::Step::Block(block, _) => {
                let relation = table(&block.source.clause, body, root, &result, tables, database);
                result = result.join(
                    &relation.project(
                        block
                            .source
                            .variables()
                            .map(|(_, v)| symbol_name(v))
                            .collect(),
                    ),
                );
            }
            vo::Step::Atom(atom, _) => {
                result = result.join(
                    &tables[&atom.clause]
                        .project(atom.variables().map(|(_, v)| symbol_name(v)).collect()),
                )
            }
            vo::Step::Negation(negative, _) => result
                .rows
                .retain(|binding| !negative_matches(negative, binding, database)),
            vo::Step::Guard(guard, _) => result
                .rows
                .retain(|row| evaluate_predicate(&guard.condition, row)),
            vo::Step::Equality(eq, _) => result.rows.retain(|row| evaluate_equality(eq, row)),
            vo::Step::Constraint(c, _) => result.rows.retain(|row| evaluate_constraint(c, row)),
            vo::Step::Let(..) | vo::Step::For(..) => {
                panic!("computed columns and generators use compiled written-order Rust")
            }
            vo::Step::Reduce(reduce, _) => match &reduce.reduction {
                vo::Reduction::With(_, support) => {
                    let mut left = table(&reduce.clause, body, root, &result, tables, database);
                    let right = table(&support.clause, body, root, &result, tables, database);
                    let same = |a: &str, b: &str| {
                        let mut reached = BTreeSet::from([a.to_owned()]);
                        loop {
                            let previous = reached.len();
                            for item in body {
                                if let cq::BodyItem::Equality(eq) = item {
                                    if let cq::EqualityValue::Variable(v) = &eq.right {
                                        let (l, r) = (symbol_name(&eq.left), symbol_name(v));
                                        if reached.contains(&l) || reached.contains(&r) {
                                            reached.insert(l);
                                            reached.insert(r);
                                        }
                                    }
                                }
                            }
                            if reached.len() == previous {
                                return reached.contains(b);
                            }
                        }
                    };
                    left.rows.retain(|row| {
                        right.rows.iter().any(|witness| {
                            support.variables().all(|(_, v)| {
                                let v = symbol_name(v);
                                row.iter()
                                    .filter(|(c, _)| same(c, &v))
                                    .all(|(_, value)| *value == witness[&v])
                            })
                        })
                    });
                    tables.insert(reduce.clause.clone(), left);
                }
            },
        }
    }
    result
}

fn negative_matches(negative: &cq::Negation, binding: &Binding, database: &Database) -> bool {
    database[&symbol_name(&negative.atom.relation)]
        .iter()
        .any(|row| {
            negative.atom.args.iter().zip(row).all(|(argument, value)| {
                matches!(cq::rust::unparen(argument), syn::Expr::Infer(_))
                    || evaluate_integer(argument, binding) == *value
            })
        })
}
