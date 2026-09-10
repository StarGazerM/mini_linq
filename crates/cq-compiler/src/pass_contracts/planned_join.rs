//! Independent CQ -> RA edge relation. This reads actual definitions; it never
//! invokes the scheduling policy or the rule implementation that emits RA.
use crate::cq_to_relational::{self, JoinPlanReport, Rule};
use cq_ir::{cq, relational_plan as ra, symbol_name};
use std::collections::{BTreeMap, BTreeSet};

type Heading = BTreeSet<String>;
struct Group {
    coverage: BTreeSet<usize>,
    result: String,
}
#[derive(PartialEq, Eq)]
enum Phase {
    Start,
    Local,
    Reduce,
    Assemble,
    Done,
}

pub(crate) fn check(
    source: &cq::Module,
    target: &ra::Module,
    proof: &JoinPlanReport,
) -> syn::Result<()> {
    cq::contract::check(source)?;
    ra::contract::check(target)?;
    require(
        target.program == source.program,
        "planning changed the source annotation",
    )?;
    let clauses = cq_to_relational::dependencies::ClauseDependencies::analyze(source)?;
    if !clauses.supports_graph_rules {
        let mut prefix = Vec::new();
        for (clause, item) in source.program.query.body.iter().enumerate() {
            if let cq::BodyItem::Positive { atom } = item {
                for (argument, _) in atom
                    .args
                    .iter()
                    .enumerate()
                    .filter(|(_, arg)| cq::rust::literal(arg))
                {
                    prefix.push(Rule::Constant { clause, argument });
                }
            }
        }
        require(
            proof.derivation.starts_with(&prefix) && proof.derivation.last() == Some(&Rule::Finish),
            "missing constant normalization or Finish",
        )?;
        let order = proof.derivation[prefix.len()..proof.derivation.len() - 1]
            .iter()
            .map(|rule| match rule {
                Rule::Match { clause } | Rule::Bind { clause } | Rule::Guard { clause } => {
                    match (rule, source.program.query.body.get(*clause)) {
                        (Rule::Match { .. }, Some(cq::BodyItem::Positive { .. }))
                        | (Rule::Guard { .. }, Some(cq::BodyItem::Filter(_)))
                        | (Rule::Bind { .. }, Some(cq::BodyItem::Let(_))) => Ok(*clause),
                        _ => Err(syn::Error::new_spanned(source, "invalid clause derivation")),
                    }
                }
                _ => Err(syn::Error::new_spanned(source, "invalid clause derivation")),
            })
            .collect::<syn::Result<Vec<_>>>()?;
        super::ordered::check_scheduled_in(source, target, &order, &clauses)?;
        let peak = ra::check_in(target.program.inputs(), &target.plan)?
            .values()
            .map(|r| r.heading.len())
            .max()
            .unwrap_or(0);
        require(
            proof.analysis.is_empty()
                && proof.join_tree.is_none()
                && proof.peak_arity == peak
                && proof.options.max_arity.is_none_or(|limit| peak <= limit),
            "Rust clauses require a legal complete derivation and exact width",
        )?;
        return Ok(());
    }
    let info = ra::check_in(target.program.inputs(), &target.plan)?;
    let atoms: Vec<_> = source
        .program
        .query
        .body
        .iter()
        .filter_map(cq::BodyItem::atom)
        .collect();
    let edges = cq_to_relational::incidence(source);
    cq_to_relational::check_analysis(&edges, proof.options, &proof.analysis, &proof.join_tree)?;
    let predicates: Vec<_> = source
        .program
        .query
        .body
        .iter()
        .filter_map(|item| match item {
            cq::BodyItem::Filter(f) => cq::Predicate::from_expr(&f.condition),
            _ => None,
        })
        .collect();
    let head: Heading = source
        .program
        .query
        .head
        .variables()
        .map(symbol_name)
        .collect();
    let mut pending: BTreeSet<_> = (0..predicates.len()).collect();
    let mut groups = BTreeMap::<usize, Group>::new();
    let mut phase = Phase::Start;
    let mut introduced = 0;
    let mut up = BTreeSet::new();
    let mut down = BTreeSet::new();
    let mut definitions = target.plan.definitions.iter();
    let ready = |groups: &BTreeMap<usize, Group>, pending: &BTreeSet<usize>| {
        pending.iter().any(|p| {
            groups.values().any(|g| {
                predicates[*p]
                    .variables()
                    .is_subset(&info[&g.result].heading)
            })
        })
    };
    let keep = |g: &Group, pending: &BTreeSet<usize>| -> Heading {
        let needed: Heading = head
            .iter()
            .cloned()
            .chain(
                edges
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !g.coverage.contains(i))
                    .flat_map(|(_, h)| h.iter().cloned()),
            )
            .chain(pending.iter().flat_map(|p| predicates[*p].variables()))
            .collect();
        info[&g.result]
            .heading
            .intersection(&needed)
            .cloned()
            .collect()
    };
    for rule in &proof.derivation {
        match *rule {
            Rule::Match { .. }
            | Rule::Guard { .. }
            | Rule::Bind { .. }
            | Rule::Constant { .. }
            | Rule::Finish => {
                return Err(error("ordered rule in graph derivation"));
            }
            Rule::Introduce { occurrence } => {
                require(
                    phase == Phase::Start && occurrence == introduced && occurrence < atoms.len(),
                    "occurrence introduction is missing, repeated or out of order",
                )?;
                let d = next(&mut definitions)?;
                let ra::Operator::Rename(r) = &d.operator else {
                    return Err(error("occurrence requires a complete Rename"));
                };
                let atom = atoms[occurrence];
                let input = source
                    .program
                    .inputs()
                    .find(|i| symbol_name(&i.name) == symbol_name(&atom.relation))
                    .unwrap();
                require(
                    symbol_name(&r.relation) == symbol_name(&atom.relation)
                        && input.columns.iter().zip(atom.variables()).all(|(c, v)| {
                            r.mapping.iter().any(|m| {
                                symbol_name(&m.source) == symbol_name(&c.name)
                                    && symbol_name(&m.target) == symbol_name(v)
                            })
                        }),
                    "Rename does not preserve the occurrence's positional tuple constraint",
                )?;
                groups.insert(
                    occurrence,
                    Group {
                        coverage: BTreeSet::from([occurrence]),
                        result: symbol_name(&d.result),
                    },
                );
                introduced += 1;
                if introduced == atoms.len() {
                    phase = Phase::Local;
                }
            }
            Rule::Select { group, predicate } => {
                require(
                    matches!(phase, Phase::Local | Phase::Assemble) && pending.contains(&predicate),
                    "selection requires a pending predicate in an applicable phase",
                )?;
                let g = get(&groups, group)?;
                require(
                    predicates[predicate]
                        .variables()
                        .is_subset(&info[&g.result].heading),
                    "predicate variables were pruned or are not yet available",
                )?;
                let d = next(&mut definitions)?;
                require(
                    matches!(&d.operator, ra::Operator::Select(s) if symbol_name(&s.input) == g.result && cq::Predicate::from_expr(&s.condition).as_ref() == Some(&predicates[predicate])),
                    "selection changed its predicate or consumed an obsolete input",
                )?;
                groups.get_mut(&group).unwrap().result = symbol_name(&d.result);
                pending.remove(&predicate);
            }
            Rule::Begin => {
                require(
                    phase == Phase::Local && !ready(&groups, &pending),
                    "begin requires all occurrences and eager local selections",
                )?;
                phase = if proof.join_tree.is_some() {
                    Phase::Reduce
                } else {
                    Phase::Assemble
                };
            }
            Rule::Up { child } | Rule::Down { child } => {
                require(
                    phase == Phase::Reduce,
                    "semijoin reduction outside reduction phase",
                )?;
                let tree = proof.join_tree.as_ref().unwrap();
                let parent = tree
                    .parent
                    .get(child)
                    .copied()
                    .flatten()
                    .ok_or_else(|| error("reduction must follow a non-root tree edge"))?;
                let downward = matches!(rule, Rule::Down { .. });
                let (left, right) = if downward {
                    require(
                        up.len() + 1 == atoms.len()
                            && !down.contains(&child)
                            && (tree.parent[parent].is_none() || down.contains(&parent)),
                        "downward dependency or exactly-once premise fails",
                    )?;
                    (child, parent)
                } else {
                    require(
                        !up.contains(&child)
                            && down.is_empty()
                            && tree.children(child).all(|c| up.contains(&c)),
                        "upward dependency or exactly-once premise fails",
                    )?;
                    (parent, child)
                };
                let l = get(&groups, left)?;
                let r = get(&groups, right)?;
                let d = next(&mut definitions)?;
                require(
                    matches!(&d.operator, ra::Operator::Semijoin(s) if symbol_name(&s.left) == l.result && symbol_name(&s.right) == r.result),
                    "semijoin must use both latest occurrence versions in the scheduled direction",
                )?;
                require(
                    info[&symbol_name(&d.result)].heading == edges[left],
                    "reduction changed an occurrence heading",
                )?;
                groups.get_mut(&left).unwrap().result = symbol_name(&d.result);
                if downward {
                    down.insert(child);
                } else {
                    up.insert(child);
                }
            }
            Rule::Freeze => {
                require(
                    phase == Phase::Reduce
                        && up.len() + 1 == atoms.len()
                        && down.len() + 1 == atoms.len(),
                    "freeze requires both complete reduction passes",
                )?;
                phase = Phase::Assemble;
            }
            Rule::Trim { group } => {
                require(
                    phase == Phase::Assemble && !ready(&groups, &pending),
                    "pruning requires assembly phase and applied ready predicates",
                )?;
                let g = get(&groups, group)?;
                let retained = keep(g, &pending);
                require(
                    retained != info[&g.result].heading,
                    "trim must remove an unused attribute",
                )?;
                let d = next(&mut definitions)?;
                require(
                    matches!(&d.operator, ra::Operator::Project(p) if symbol_name(&p.input) == g.result && p.attributes.iter().map(symbol_name).collect::<Heading>() == retained),
                    "projection must retain exactly head, crossing, and pending-predicate uses",
                )?;
                groups.get_mut(&group).unwrap().result = symbol_name(&d.result);
            }
            Rule::Combine { left, right } => {
                require(
                    phase == Phase::Assemble
                        && !ready(&groups, &pending)
                        && groups
                            .values()
                            .all(|g| keep(g, &pending) == info[&g.result].heading),
                    "combine requires eager selection and pruning",
                )?;
                let l = get(&groups, left)?;
                let r = get(&groups, right)?;
                require(
                    l.coverage.is_disjoint(&r.coverage),
                    "combine must partition occurrence ownership",
                )?;
                let d = next(&mut definitions)?;
                require(
                    matches!(&d.operator, ra::Operator::NaturalJoin(j) if symbol_name(&j.left) == l.result && symbol_name(&j.right) == r.result),
                    "join must consume exactly the two current frontier results",
                )?;
                let coverage = l.coverage.union(&r.coverage).copied().collect();
                groups.remove(&right);
                groups.insert(
                    left,
                    Group {
                        coverage,
                        result: symbol_name(&d.result),
                    },
                );
            }
            Rule::Output { group } => {
                require(
                    phase == Phase::Assemble && groups.len() == 1 && pending.is_empty(),
                    "output requires completed assembly and all predicates",
                )?;
                let g = get(&groups, group)?;
                require(
                    g.coverage == (0..atoms.len()).collect()
                        && info[&g.result].heading == head
                        && symbol_name(&target.plan.output.input) == g.result,
                    "output lacks source coverage, exact head, or latest result",
                )?;
                phase = Phase::Done;
            }
        }
    }
    require(
        phase == Phase::Done && definitions.next().is_none(),
        "unfinished derivation or unaccounted RA definitions",
    )?;
    let peak = info.values().map(|r| r.heading.len()).max().unwrap_or(0);
    require(
        proof.peak_arity == peak && proof.options.max_arity.is_none_or(|limit| peak <= limit),
        "incorrect arity report or exceeded bound",
    )
}

fn get(groups: &BTreeMap<usize, Group>, id: usize) -> syn::Result<&Group> {
    groups
        .get(&id)
        .ok_or_else(|| error("rule must refer to a current frontier entry"))
}
fn next<'a>(
    definitions: &mut impl Iterator<Item = &'a ra::Definition>,
) -> syn::Result<&'a ra::Definition> {
    definitions
        .next()
        .ok_or_else(|| error("rule has no emitted RA definition"))
}
fn require(premise: bool, message: &str) -> syn::Result<()> {
    if premise { Ok(()) } else { Err(error(message)) }
}
fn error(message: &str) -> syn::Error {
    syn::Error::new(proc_macro2::Span::call_site(), message)
}
