//! IX → Physical: one physical operator per RA result, the mode its reads
//! allow, and which results must be made canonical.
//!
//! This pass decides three things and nothing else:
//!
//! - the **operator**: a natural join is `probe[C] l, r` — `r` the side IX
//!   names as indexed, `C` the attributes both headings share — or
//!   `merge[C] l, r` under `policy { join = merge; }`; a join without shared
//!   attributes is the Cartesian `probe[] l, r` under either rule, since
//!   there is no key to co-iterate on; a semijoin is `semi[C] l, r`; a
//!   select is `range[c]` when its input renames a host the policy keeps
//!   `keyed [c] ordered` on that one column and the condition bounds `c` by
//!   an integer literal inside the total comparison fragment, else `select`;
//!   every other operator keeps its logical name;
//! - the **mode**: a rename is an `alias` when the host's policy key answers
//!   every keyed, ranged or merged read of it (a rename that is only scanned
//!   is always an alias), else a `let` for Structures to build over; any
//!   other result is a `let` when it is read through a key, read more than
//!   once, or merged on a key it is not itself sorted on by its own merge,
//!   else a `pipe`;
//! - the **canon** flag: on the returned result when uniqueness does not
//!   follow from the policy's `canon` hosts through joins, semijoins,
//!   selects, extends and key-keeping projections — the same inference
//!   Structures repeats with the built values it adds. Under
//!   `policy { project = canon; }`, every projection whose kept columns do
//!   not retain an inferred input key is also flagged. Its reads still
//!   decide its mode: a canon pipe deduplicates its stream, a canon let
//!   materializes a set.
//!
//! No Rust type is named here (Structures binds every keyed read to a
//! value), and `policy { semijoin = prefer_lazy; }` is not read: whether a
//! semijoin's test is deferred to its consumer is a printer's choice with no
//! trace in this language. [`check`] independently checks erasure and policy
//! obligations; any admissible alias/let/pipe choice is accepted, without
//! replaying operator or mode selection.

use std::collections::{BTreeMap, BTreeSet};

use cq_lang_ix as ix;
use cq_lang_ix::ra::Operator as Op;
use cq_lang_ix::syntax as cq;
use cq_lang_ix::syntax::symbol_name;
use cq_lang_physical as physical;
use cq_lang_physical::policy::Policy;
use syn::Ident;

/// A host's key as the policy states it: sorted host attribute names.
struct HostKey {
    columns: Vec<String>,
    ordered: bool,
}

/// The policy's facts about every declared host relation.
struct Hosts {
    keys: BTreeMap<String, Option<HostKey>>,
    canon: BTreeSet<String>,
}

/// How one result is read, gathered from every reader in RA order.
#[derive(Default)]
struct Reads {
    /// Operand references, the return counted once.
    count: usize,
    /// Key columns of every `probe`/`semi` read of the result.
    keyed: Vec<Vec<String>>,
    /// Ordered column of every `range` read of the result.
    ranged: Vec<String>,
    /// Key columns of every `merge` read of the result.
    merged: Vec<Vec<String>>,
}

/// Lower IX to Physical under the policy, then check the Physical contract.
pub fn compile(source: &ix::Program, policy: &Policy) -> syn::Result<physical::Program> {
    ix::contract::check(source)?;
    let info = ix::ra::contract::check(&source.ra)?;
    let operations: Vec<_> = source
        .ra
        .plan
        .definitions
        .iter()
        .map(|d| (d.result.clone(), d.operator.clone()))
        .collect();
    let inputs: Vec<_> = source
        .ra
        .plan
        .declarations
        .iter()
        .filter(|d| symbol_name(&d.name) != symbol_name(&source.ra.plan.output.head.relation))
        .cloned()
        .collect();
    let hosts = hosts(policy, &inputs)?;
    let returned = source.ra.plan.output.input.clone();
    let (operators, reads) = operators(source, &info, &operations, &hosts, policy)?;
    let modes = modes(&operations, &operators, &reads, &hosts);
    let unique = canonical(&info, &operations, &hosts);
    let definitions = operations
        .iter()
        .zip(modes)
        .zip(operators)
        .map(|(((name, op), mode), operator)| physical::Definition {
            name: name.clone(),
            eq_token: Default::default(),
            mode,
            canon: (!unique.contains(&symbol_name(name))
                && (name == &returned
                    || (policy.canon_projections() && matches!(op, Op::Project(_)))))
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
    check_demands(source, &target)?;
    Ok(target)
}

fn operators(
    source: &ix::Program,
    info: &ix::ra::Analysis,
    operations: &[(Ident, Op)],
    hosts: &Hosts,
    policy: &Policy,
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
                    bracket: Default::default(),
                    columns: columns.iter().map(|c| lh.bindings[c].clone()).collect(),
                    left: left.clone(),
                    comma: Default::default(),
                    right: right.clone(),
                };
                if matches!(op, Op::Semijoin(_)) {
                    probe(&mut reads, right, columns);
                    physical::Operator::Semi(Default::default(), keyed)
                } else if policy.merge_joins() && !columns.is_empty() {
                    merge(&mut reads, left, right, columns);
                    physical::Operator::Merge(Default::default(), keyed)
                } else {
                    probe(&mut reads, right, columns);
                    physical::Operator::Probe(Default::default(), keyed)
                }
            }
            Op::Select(s) => {
                scan(&mut reads, &s.input);
                let rename = operations
                    .iter()
                    .find(|(name, _)| *name == s.input)
                    .map(|(_, op)| op);
                match ordered_column(hosts, rename).filter(|column| bounded(&s.condition, column)) {
                    Some(column) => {
                        reads
                            .get_mut(&symbol_name(&s.input))
                            .unwrap()
                            .ranged
                            .push(column.clone());
                        physical::Operator::Range(
                            Default::default(),
                            physical::Ranged {
                                bracket: Default::default(),
                                column: info[&symbol_name(&s.input)].bindings[&column].clone(),
                                input: s.input.clone(),
                                if_token: s.if_token,
                                condition_paren: s.condition_paren,
                                condition: s.condition.clone(),
                            },
                        )
                    }
                    None => physical::Operator::Select(s.clone()),
                }
            }
            Op::Extend(e) => {
                scan(&mut reads, &e.input);
                physical::Operator::Extend(e.clone())
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
    hosts: &Hosts,
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
                Op::Rename(_) if hosts.answers(op, result) => {
                    physical::Mode::Alias(Default::default())
                }
                Op::Rename(_) => physical::Mode::Let(Default::default()),
                _ if !result.keyed.is_empty()
                    || !result.ranged.is_empty()
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
    hosts: &Hosts,
) -> BTreeSet<String> {
    let mut unique: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, op) in operations {
        let name = symbol_name(name);
        let heading: Vec<_> = info[&name].heading.iter().cloned().collect();
        let key = match op {
            Op::Unit(_) => Some(Vec::new()),
            Op::Rename(r) => hosts
                .canon
                .contains(&symbol_name(&r.relation))
                .then(|| heading.clone()),
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
            Op::Extend(e) => unique.get(&symbol_name(&e.input)).cloned(),
            Op::Project(p) => unique
                .get(&symbol_name(&p.input))
                .filter(|key| key.iter().all(|c| heading.contains(c)))
                .cloned(),
        };
        if let Some(key) = key {
            unique.insert(name, key);
        }
    }
    unique.into_keys().collect()
}

/// The policy's hosts, checked against the declared input relations.
fn hosts(policy: &Policy, inputs: &[cq::RelationDecl]) -> syn::Result<Hosts> {
    let layouts: BTreeMap<String, Vec<String>> = inputs
        .iter()
        .map(|input| {
            (
                symbol_name(&input.name),
                input.columns.iter().map(|c| symbol_name(&c.name)).collect(),
            )
        })
        .collect();
    let mut hosts = Hosts {
        keys: BTreeMap::new(),
        canon: BTreeSet::new(),
    };
    for host in &policy.hosts {
        let name = symbol_name(&host.name);
        let Some(layout) = layouts.get(&name) else {
            return Err(syn::Error::new_spanned(
                &host.name,
                "policy names an undeclared host relation",
            ));
        };
        let key = match &host.key {
            Some(key) => {
                let mut columns = Vec::with_capacity(key.columns.len());
                for column in &key.columns {
                    if !layout.contains(&symbol_name(column)) {
                        return Err(syn::Error::new_spanned(
                            column,
                            format!("`{name}` has no column `{column}`"),
                        ));
                    }
                    columns.push(symbol_name(column));
                }
                columns.sort();
                columns.dedup();
                Some(HostKey {
                    columns,
                    ordered: key.ordered.is_some(),
                })
            }
            None => None,
        };
        if hosts.keys.insert(name.clone(), key).is_some() {
            return Err(syn::Error::new_spanned(
                &host.name,
                "host relation configured twice",
            ));
        }
        if host.canon.is_some() {
            hosts.canon.insert(name);
        }
    }
    if let Some(missing) = layouts.keys().find(|name| !hosts.keys.contains_key(*name)) {
        return Err(syn::Error::new_spanned(
            policy,
            format!("policy must list host relation `{missing}`"),
        ));
    }
    Ok(hosts)
}

impl Hosts {
    /// The host's key columns translated into a rename's attribute names,
    /// sorted, with whether the key is ordered.
    fn key_through(&self, rename: &Op) -> Option<(Vec<String>, bool)> {
        let Op::Rename(r) = rename else {
            return None;
        };
        let key = self.keys[&symbol_name(&r.relation)].as_ref()?;
        let mut columns: Vec<String> = r
            .mapping
            .iter()
            .filter(|m| key.columns.contains(&symbol_name(&m.source)))
            .map(|m| symbol_name(&m.target))
            .collect();
        columns.sort();
        Some((columns, key.ordered))
    }

    /// Whether the host behind `rename` answers every read in place: keyed
    /// reads on its key, ranged and merged reads on its ordered key.
    fn answers(&self, rename: &Op, reads: &Reads) -> bool {
        let key = self.key_through(rename);
        let on_key = |columns: &[String], needs_order: bool| {
            key.as_ref().is_some_and(|(key, ordered)| {
                let mut wanted = columns.to_vec();
                wanted.sort();
                wanted == *key && (*ordered || !needs_order)
            })
        };
        reads.keyed.iter().all(|c| on_key(c, false))
            && reads
                .ranged
                .iter()
                .all(|c| on_key(std::slice::from_ref(c), true))
            && reads.merged.iter().all(|c| on_key(c, true))
    }
}

/// The single ordered key column of the host behind `rename`, in the
/// rename's attribute name.
fn ordered_column(hosts: &Hosts, rename: Option<&Op>) -> Option<String> {
    let (columns, ordered) = hosts.key_through(rename?)?;
    match columns.as_slice() {
        [column] if ordered => Some(column.clone()),
        _ => None,
    }
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

/// Both merge sides are enumerated in key order; the left side's scan is
/// already counted by the caller.
fn merge(reads: &mut BTreeMap<String, Reads>, left: &Ident, right: &Ident, columns: Vec<String>) {
    let right = reads
        .get_mut(&symbol_name(right))
        .expect("operand is a result");
    right.count += 1;
    right.merged.push(columns.clone());
    reads
        .get_mut(&symbol_name(left))
        .expect("operand is a result")
        .merged
        .push(columns);
}

fn conjuncts(expr: &syn::Expr) -> Vec<&syn::Expr> {
    match cq::rust::unparen(expr) {
        syn::Expr::Binary(syn::ExprBinary {
            op: syn::BinOp::And(_),
            left,
            right,
            ..
        }) => {
            let mut all = conjuncts(left);
            all.extend(conjuncts(right));
            all
        }
        other => vec![other],
    }
}

/// Whether some conjunct of `condition` bounds `attribute` from below or
/// above by an integer literal: a comparison in the total fragment other
/// than `!=`, whose strict bound does not overflow `i32`.
fn bounded(condition: &syn::Expr, attribute: &str) -> bool {
    use cq::predicate::{Comparison, Predicate, Term};
    conjuncts(condition).into_iter().any(|part| {
        let Some(predicate) = Predicate::from_expr(part) else {
            return false;
        };
        // `literal cmp attribute` reads as the flipped `attribute cmp literal`.
        let (literal, flipped) = match (&predicate.left, &predicate.right) {
            (Term::Variable(v), Term::Integer(k)) if symbol_name(v) == attribute => (*k, false),
            (Term::Integer(k), Term::Variable(v)) if symbol_name(v) == attribute => (*k, true),
            _ => return false,
        };
        match (predicate.comparison, flipped) {
            (Comparison::Eq(_) | Comparison::Ge(_) | Comparison::Le(_), _) => true,
            (Comparison::Gt(_), false) | (Comparison::Lt(_), true) => {
                literal.checked_add(1).is_some()
            }
            (Comparison::Lt(_), false) | (Comparison::Gt(_), true) => {
                literal.checked_sub(1).is_some()
            }
            (Comparison::Ne(_), _) => false,
        }
    })
}

/// Erase physical choices to the relational program they implement.
pub fn erase(target: &physical::Program) -> ix::ra::Program {
    use ix::ra;
    ra::Program {
        relational_token: Default::default(),
        relational_brace: Default::default(),
        plan: ra::Plan {
            declarations: target.program.declarations.clone(),
            definitions: target
                .program
                .definitions
                .iter()
                .map(|d| ra::Definition {
                    result: d.name.clone(),
                    eq_token: d.eq_token,
                    semi_token: d.semi_token,
                    operator: match &d.operator {
                        physical::Operator::Unit(u) => Op::Unit(u.clone()),
                        physical::Operator::Rename(r) => Op::Rename(r.clone()),
                        physical::Operator::Select(s) => Op::Select(s.clone()),
                        physical::Operator::Extend(e) => Op::Extend(e.clone()),
                        physical::Operator::Project(p) => Op::Project(p.clone()),
                        physical::Operator::Probe(_, k) | physical::Operator::Merge(_, k) => {
                            Op::NaturalJoin(Box::new(ra::NaturalJoin {
                                natural_join_token: Default::default(),
                                left: k.left.clone(),
                                with_token: Default::default(),
                                right: k.right.clone(),
                            }))
                        }
                        physical::Operator::Semi(_, k) => Op::Semijoin(Box::new(ra::Semijoin {
                            semijoin_token: Default::default(),
                            left: k.left.clone(),
                            with_token: Default::default(),
                            right: k.right.clone(),
                        })),
                        physical::Operator::Range(_, r) => Op::Select(Box::new(ra::Select {
                            select_token: Default::default(),
                            input: r.input.clone(),
                            if_token: r.if_token,
                            condition_paren: r.condition_paren,
                            condition: r.condition.clone(),
                        })),
                    },
                })
                .collect(),
            output: ra::Output {
                output_token: Default::default(),
                input: target.program.returned.clone(),
                as_token: target.program.as_token,
                head: target.program.head.clone(),
                semi_token: target.program.return_semi,
            },
        },
    }
}

fn check_demands(source: &ix::Program, target: &physical::Program) -> syn::Result<()> {
    let analysis = physical::contract::check(target)?;
    let demands: BTreeSet<_> = source
        .indexes
        .iter()
        .map(|i| ix::signature(&i.requirement))
        .collect();
    for d in &target.program.definitions {
        if let Some((right, columns)) = d.operator.keyed() {
            let right_name = symbol_name(right);
            if !demands.contains(&(
                right_name.clone(),
                physical::ordinals(&analysis, &right_name, &columns),
            )) {
                return Err(syn::Error::new_spanned(
                    &d.name,
                    format!(
                        "probe `{}` needs demand `@{right_name}[{}]`",
                        d.name,
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
