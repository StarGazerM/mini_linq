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
//! trace in this language.

use std::collections::{BTreeMap, BTreeSet};

use cq_lang_ix as ix;
use cq_lang_ix::Operation as Op;
use cq_lang_ix::syntax as cq;
use cq_lang_ix::syntax::symbol_name;
use cq_lang_physical as physical;
use cq_lang_physical::policy::Policy;
use proc_macro2::Span;
use quote::format_ident;
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
pub fn compile(
    source: &ix::Section,
    env: &impl ix::Env,
    policy: &Policy,
) -> syn::Result<physical::Section> {
    ix::contract::check(source, env)?;
    let operations = env.operations();
    let hosts = hosts(policy, &env.inputs())?;
    let returned = env.returned();
    let (operators, reads) = operators(source, env, &operations, &hosts, policy)?;
    let modes = modes(&operations, &operators, &reads, &hosts);
    let unique = canonical(env, &operations, &hosts);
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
                    || (policy.canon_projections() && matches!(op, Op::Project { .. }))))
            .then(Default::default),
            operator,
            semi_token: Default::default(),
        })
        .collect();
    let section = physical::Section {
        physical_token: Default::default(),
        brace: Default::default(),
        program: physical::Program {
            definitions,
            return_token: Default::default(),
            returned,
            return_semi: Default::default(),
        },
    };
    physical::contract::check(&section, &Env::new(source, env)?)?;
    Ok(section)
}

fn operators(
    source: &ix::Section,
    env: &impl ix::Env,
    operations: &[(Ident, Op)],
    hosts: &Hosts,
    policy: &Policy,
) -> syn::Result<(Vec<physical::Operator>, BTreeMap<String, Reads>)> {
    let demands: BTreeSet<_> = source
        .indexes
        .iter()
        .map(|i| ix::signature(&i.requirement))
        .collect();
    let heading = |id: &Ident| env.heading(&symbol_name(id)).expect("checked heading");
    let mut reads: BTreeMap<String, Reads> = operations
        .iter()
        .map(|(id, _)| (symbol_name(id), Reads::default()))
        .collect();
    let mut operators = Vec::new();
    for (name, op) in operations {
        let operator = match op {
            Op::Unit => physical::Operator::Unit(Default::default()),
            Op::Rename { .. } => physical::Operator::Rename(Default::default()),
            Op::Join { left, right } | Op::Semijoin { left, right } => {
                if !demands.contains(&(
                    symbol_name(right),
                    ix::result_join_columns(heading(left), heading(right)),
                )) {
                    return Err(syn::Error::new_spanned(
                        name,
                        format!(
                            "{} `{name}` has no index use",
                            if matches!(op, Op::Join { .. }) {
                                "natural join"
                            } else {
                                "semijoin"
                            }
                        ),
                    ));
                }
                let columns: Vec<String> = heading(left)
                    .iter()
                    .filter(|c| heading(right).contains(c))
                    .cloned()
                    .collect();
                scan(&mut reads, left);
                let keyed = keyed(&columns, left, right);
                if matches!(op, Op::Semijoin { .. }) {
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
            Op::Select { input, condition } => {
                scan(&mut reads, input);
                let rename = operations
                    .iter()
                    .find(|(name, _)| name == input)
                    .map(|(_, op)| op);
                match ordered_column(hosts, rename).filter(|column| bounded(condition, column)) {
                    Some(column) => {
                        reads
                            .get_mut(&symbol_name(input))
                            .unwrap()
                            .ranged
                            .push(column.clone());
                        physical::Operator::Range(
                            Default::default(),
                            physical::Ranged {
                                bracket: Default::default(),
                                column: ident(&column),
                            },
                        )
                    }
                    None => physical::Operator::Select(Default::default()),
                }
            }
            Op::Extend { input, .. } => {
                scan(&mut reads, input);
                physical::Operator::Extend(Default::default())
            }
            Op::Project { input } => {
                scan(&mut reads, input);
                physical::Operator::Project(Default::default())
            }
        };
        operators.push(operator);
    }
    scan(&mut reads, &env.returned());
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
                Op::Rename { .. } if hosts.answers(op, result) => {
                    physical::Mode::Alias(Default::default())
                }
                Op::Rename { .. } => physical::Mode::Let(Default::default()),
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

fn canonical(env: &impl ix::Env, operations: &[(Ident, Op)], hosts: &Hosts) -> BTreeSet<String> {
    let mut unique: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, op) in operations {
        let name = symbol_name(name);
        let heading = env.heading(&name).expect("checked heading");
        let key = match op {
            Op::Unit => Some(Vec::new()),
            Op::Rename { host, .. } => hosts
                .canon
                .contains(&symbol_name(host))
                .then(|| heading.to_vec()),
            Op::Join { left, right } => match (
                unique.get(&symbol_name(left)),
                unique.get(&symbol_name(right)),
            ) {
                (Some(l), Some(r)) => {
                    let mut key: Vec<_> = l.iter().chain(r).cloned().collect();
                    key.sort();
                    key.dedup();
                    Some(key)
                }
                _ => None,
            },
            Op::Semijoin { left: input, .. }
            | Op::Select { input, .. }
            | Op::Extend { input, .. } => unique.get(&symbol_name(input)).cloned(),
            Op::Project { input } => unique
                .get(&symbol_name(input))
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
        let Op::Rename { host, mapping } = rename else {
            return None;
        };
        let key = self.keys[&symbol_name(host)].as_ref()?;
        let mut columns: Vec<String> = mapping
            .iter()
            .filter(|(_, source)| key.columns.contains(&symbol_name(source)))
            .map(|(target, _)| symbol_name(target))
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

fn keyed(columns: &[String], left: &Ident, right: &Ident) -> physical::Keyed {
    physical::Keyed {
        bracket: Default::default(),
        columns: columns.iter().map(|c| ident(c)).collect(),
        left: left.clone(),
        comma: Default::default(),
        right: right.clone(),
    }
}

fn ident(name: &str) -> Ident {
    syn::parse_str::<Ident>(name)
        .unwrap_or_else(|_| format_ident!("r#{}", name, span = Span::call_site()))
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

/// IX → Physical correspondence: IX preserved, the Physical contract holds,
/// and the program is exactly what the policy decides.
pub fn check(
    source: &ix::Section,
    env: &impl ix::Env,
    target: &physical::Section,
    policy: &Policy,
) -> bool {
    Env::new(source, env).is_ok_and(|env| physical::contract::well_formed(target, &env))
        && compile(source, env, policy).is_ok_and(|expected| expected == *target)
}

mod env;
pub use env::Env;
#[cfg(feature = "reference")]
pub mod reference {
    use super::*;
    pub fn compile(source: &ix::Module, policy: &Policy) -> syn::Result<physical::Section> {
        super::compile(
            source,
            &ix::reference::Env::new(&source.relational_plan)?,
            policy,
        )
    }
    pub fn check(source: &ix::Module, target: &physical::Section, policy: &Policy) -> bool {
        ix::reference::Env::new(&source.relational_plan)
            .is_ok_and(|env| super::check(source, &env, target, policy))
    }
}
