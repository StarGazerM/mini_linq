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
use cq_lang_ix::relational_plan as ra;
use cq_lang_ix::relational_plan::cq;
use cq_lang_ix::syntax::symbol_name;
use cq_lang_physical as physical;
use cq_lang_physical::policy::Policy;
use proc_macro2::Span;
use quote::format_ident;
use syn::Ident;

type Infos = BTreeMap<String, ra::RelationInfo>;

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
pub fn compile(source: &ix::Module, policy: &Policy) -> syn::Result<physical::Section> {
    ix::contract::check_module(source)?;
    let ra_module = &source.relational_plan;
    let plan = &ra_module.plan;
    let infos = ra::check_in(ra_module.program.inputs(), plan)?;
    let hosts = hosts(policy, ra_module)?;
    let returned = symbol_name(&plan.output.input);

    let (operators, reads) = operators(source, &infos, &hosts, policy)?;
    let modes = modes(plan, &operators, &reads, &hosts);
    let unique = canonical(plan, &infos, &hosts);

    let definitions = plan
        .definitions
        .iter()
        .zip(modes)
        .zip(operators)
        .map(|((definition, mode), operator)| physical::Definition {
            name: definition.result.clone(),
            eq_token: Default::default(),
            mode,
            canon: (!unique.contains(&symbol_name(&definition.result))
                && (symbol_name(&definition.result) == returned
                    || (policy.canon_projections()
                        && matches!(definition.operator, ra::Operator::Project(_)))))
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
            returned: plan.output.input.clone(),
            return_semi: Default::default(),
        },
    };
    physical::contract::check(&section, &physical::reference::Env::new(source)?)?;
    Ok(section)
}

/// The physical operator of every result, and the reads each performs on
/// its operands (the return's scan included).
fn operators(
    source: &ix::Module,
    infos: &Infos,
    hosts: &Hosts,
    policy: &Policy,
) -> syn::Result<(Vec<physical::Operator>, BTreeMap<String, Reads>)> {
    let plan = &source.relational_plan.plan;
    let demands: BTreeSet<_> = source
        .indexes
        .iter()
        .map(|index| ix::signature(&index.requirement))
        .collect();
    let has_demand = |left: &Ident, right: &Ident| {
        demands.contains(&(
            symbol_name(right),
            ix::result_join_columns(
                &infos[&symbol_name(left)]
                    .heading
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>(),
                &infos[&symbol_name(right)]
                    .heading
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>(),
            ),
        ))
    };
    let renames: BTreeMap<String, &ra::Rename> = plan
        .definitions
        .iter()
        .filter_map(|d| match &d.operator {
            ra::Operator::Rename(r) => Some((symbol_name(&d.result), &**r)),
            _ => None,
        })
        .collect();
    let shared = |left: &Ident, right: &Ident| -> Vec<String> {
        infos[&symbol_name(left)]
            .heading
            .intersection(&infos[&symbol_name(right)].heading)
            .cloned()
            .collect()
    };

    let mut reads: BTreeMap<String, Reads> = plan
        .definitions
        .iter()
        .map(|d| (symbol_name(&d.result), Reads::default()))
        .collect();
    let mut operators = Vec::with_capacity(plan.definitions.len());
    for definition in &plan.definitions {
        let name = symbol_name(&definition.result);
        let operator = match &definition.operator {
            ra::Operator::Unit(_) => physical::Operator::Unit(Default::default()),
            ra::Operator::Rename(_) => physical::Operator::Rename(Default::default()),
            ra::Operator::NaturalJoin(join) => {
                let columns = shared(&join.left, &join.right);
                let right = &join.right;
                if !has_demand(&join.left, right) {
                    return Err(syn::Error::new_spanned(
                        &definition.result,
                        format!("natural join `{name}` has no index use"),
                    ));
                }
                scan(&mut reads, &join.left);
                let keyed = keyed(&columns, &join.left, right);
                if policy.merge_joins() && !columns.is_empty() {
                    merge(&mut reads, &join.left, right, columns);
                    physical::Operator::Merge(Default::default(), keyed)
                } else {
                    probe(&mut reads, right, columns);
                    physical::Operator::Probe(Default::default(), keyed)
                }
            }
            ra::Operator::Semijoin(join) => {
                let columns = shared(&join.left, &join.right);
                let right = &join.right;
                if !has_demand(&join.left, right) {
                    return Err(syn::Error::new_spanned(
                        &definition.result,
                        format!("semijoin `{name}` has no index use"),
                    ));
                }
                scan(&mut reads, &join.left);
                let keyed = keyed(&columns, &join.left, right);
                probe(&mut reads, right, columns);
                physical::Operator::Semi(Default::default(), keyed)
            }
            ra::Operator::Select(select) => {
                scan(&mut reads, &select.input);
                let input = symbol_name(&select.input);
                match ordered_column(hosts, renames.get(&input).copied())
                    .filter(|column| bounded(&select.condition, column))
                {
                    Some(column) => {
                        reads.get_mut(&input).unwrap().ranged.push(column.clone());
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
            ra::Operator::Extend(extend) => {
                scan(&mut reads, &extend.input);
                physical::Operator::Extend(Default::default())
            }
            ra::Operator::Project(project) => {
                scan(&mut reads, &project.input);
                physical::Operator::Project(Default::default())
            }
        };
        operators.push(operator);
    }
    scan(&mut reads, &plan.output.input);
    Ok((operators, reads))
}

/// The mode of every result: what its reads require, and what a host
/// answers in place.
fn modes(
    plan: &ra::Plan,
    operators: &[physical::Operator],
    reads: &BTreeMap<String, Reads>,
    hosts: &Hosts,
) -> Vec<physical::Mode> {
    let own_merge: BTreeMap<String, Vec<String>> = plan
        .definitions
        .iter()
        .zip(operators)
        .filter_map(|(d, op)| match op {
            physical::Operator::Merge(_, keyed) => Some((
                symbol_name(&d.result),
                keyed.columns.iter().map(symbol_name).collect(),
            )),
            _ => None,
        })
        .collect();
    plan.definitions
        .iter()
        .map(|definition| {
            let name = symbol_name(&definition.result);
            let result = &reads[&name];
            match &definition.operator {
                ra::Operator::Rename(rename) if hosts.answers(rename, result) => {
                    physical::Mode::Alias(Default::default())
                }
                ra::Operator::Rename(_) => physical::Mode::Let(Default::default()),
                _ => {
                    let keyed = !result.keyed.is_empty() || !result.ranged.is_empty();
                    let unsorted = result
                        .merged
                        .iter()
                        .any(|columns| own_merge.get(&name) != Some(columns));
                    if keyed || result.count >= 2 || unsorted {
                        physical::Mode::Let(Default::default())
                    } else {
                        physical::Mode::Pipe(Default::default())
                    }
                }
            }
        })
        .collect()
}

/// The results unique by key inference from the policy's canonical hosts.
fn canonical(plan: &ra::Plan, infos: &Infos, hosts: &Hosts) -> BTreeSet<String> {
    let mut unique: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for definition in &plan.definitions {
        let name = symbol_name(&definition.result);
        let heading: Vec<String> = infos[&name].heading.iter().cloned().collect();
        let key = match &definition.operator {
            ra::Operator::Unit(_) => Some(Vec::new()),
            ra::Operator::Rename(r) => hosts
                .canon
                .contains(&symbol_name(&r.relation))
                .then_some(heading),
            ra::Operator::NaturalJoin(j) => {
                match (
                    unique.get(&symbol_name(&j.left)),
                    unique.get(&symbol_name(&j.right)),
                ) {
                    (Some(kl), Some(kr)) => {
                        let mut key: Vec<String> = kl.iter().chain(kr).cloned().collect();
                        key.sort();
                        key.dedup();
                        Some(key)
                    }
                    _ => None,
                }
            }
            ra::Operator::Semijoin(j) => unique.get(&symbol_name(&j.left)).cloned(),
            ra::Operator::Select(s) => unique.get(&symbol_name(&s.input)).cloned(),
            ra::Operator::Extend(e) => unique.get(&symbol_name(&e.input)).cloned(),
            ra::Operator::Project(p) => unique
                .get(&symbol_name(&p.input))
                .filter(|key| key.iter().all(|k| heading.contains(k)))
                .cloned(),
        };
        if let Some(key) = key {
            unique.insert(name, key);
        }
    }
    unique.into_keys().collect()
}

/// The policy's hosts, checked against the declared input relations.
fn hosts(policy: &Policy, ra_module: &ra::Module) -> syn::Result<Hosts> {
    let layouts: BTreeMap<String, Vec<String>> = ra_module
        .program
        .inputs()
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
    fn key_through(&self, rename: &ra::Rename) -> Option<(Vec<String>, bool)> {
        let key = self.keys[&symbol_name(&rename.relation)].as_ref()?;
        let mut columns: Vec<String> = rename
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
    fn answers(&self, rename: &ra::Rename, reads: &Reads) -> bool {
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
fn ordered_column(hosts: &Hosts, rename: Option<&ra::Rename>) -> Option<String> {
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
    use cq::{Comparison, Predicate, Term};
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
pub fn check(source: &ix::Module, target: &physical::Section, policy: &Policy) -> bool {
    physical::reference::Env::new(source)
        .is_ok_and(|env| physical::contract::well_formed(target, &env))
        && compile(source, policy).is_ok_and(|expected| expected == *target)
}
