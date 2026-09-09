//! Source-aware verification of administrative plans for the positive CQ fragment.
//!
//! Unlike local RA well-formedness, this checks occurrence ownership and safe
//! forgetting. Eager pruning and an arity bound are additional policy contracts.
//! Checks run in every build. This module verifies actual operators; it neither
//! invokes the planner nor trusts supplied headings, coverage sets, or widths.
//!
//! A reduction witness selects a fixed Yannakakis schedule (children ordered by
//! occurrence ID). Until RA has Semijoin, each step must be its adjacent
//! NaturalJoin/Project expansion. This certifies semantics and scheduling, not
//! an efficient physical implementation. Predicates and physical layouts are
//! outside this existing IR fragment and receive no verification claim here.

use std::collections::{BTreeMap, BTreeSet};

use cq_ir::{cq, relational_plan as ra, symbol_name};
use syn::Ident;

type Heading = BTreeSet<String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionPolicy {
    /// Preserve every future use; retaining extra columns is allowed.
    Safe,
    /// Every assembly operand must be trimmed before its next combination.
    /// Raw operator results may be followed by projections. Reduction schemas
    /// stay complete until both passes finish.
    Eager,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub projection: ProjectionPolicy,
    /// Bound every named logical result, including renames and reduction joins
    /// before projection. This is column count, not bytes or decomposition width.
    pub max_arity: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Witness {
    /// Rename result for each source occurrence, in source occurrence order.
    pub occurrences: Vec<String>,
    /// One parent per occurrence, with exactly one root (`None`). A witness
    /// certifies a supplied tree; this verifier does not discover one with GYO.
    pub reduction_tree: Option<Vec<Option<usize>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub headings: BTreeMap<String, Heading>,
    pub join_keys: BTreeMap<String, Heading>,
    pub peak_arity: usize,
}

/// An owned, immutable plan checked against its source and the recorded policy.
/// Construction is private: modifying a cloned RA requires verification again.
/// This is not a certificate of compiled execution or global cost optimality.
#[derive(Debug)]
pub struct VerifiedPlan {
    plan: ra::Module,
    report: Report,
}

impl VerifiedPlan {
    pub fn plan(&self) -> &ra::Module {
        &self.plan
    }

    pub fn report(&self) -> &Report {
        &self.report
    }
}

/// Check source preservation, every occurrence mapping, the optional full
/// reduction schedule, and linear ownership of the subsequent assembly frontier.
/// Valid RA outside these administrative rules is deliberately not certified.
pub fn verify(
    source: &cq::Module,
    plan: ra::Module,
    witness: &Witness,
    policy: Policy,
) -> syn::Result<VerifiedPlan> {
    cq::contract::check(source)?;
    for item in &source.program.query.body {
        item.positive_atom()?;
    }
    ra::contract::check(&plan)?;
    if plan.program != source.program {
        return Err(syn::Error::new_spanned(
            &plan.program,
            "plan changed the source program",
        ));
    }
    let occurrences = check_occurrences(source, &plan, witness)?;
    let variables: Vec<Heading> = source
        .program
        .query
        .body
        .iter()
        .map(|item| {
            let atom = item.positive_atom().expect("checked legacy fragment");
            atom.variables().map(symbol_name).collect()
        })
        .collect();
    let head = source
        .program
        .query
        .head
        .variables()
        .map(symbol_name)
        .collect();
    let mut checker = Checker {
        variables,
        head,
        occurrences,
        frontier: BTreeMap::new(),
        policy,
        report: Report {
            headings: BTreeMap::new(),
            join_keys: BTreeMap::new(),
            peak_arity: 0,
        },
    };
    let definitions = &plan.plan.definitions;
    let mut next = 0;
    if let Some(parents) = &witness.reduction_tree {
        let schedule = reduction_schedule(parents, &checker.variables, &plan.program)?;
        // Full-schema original renames precede reduction. Internal semijoin
        // dependencies are not disjoint assembly ownership.
        for _ in &witness.occurrences {
            let definition = definitions.get(next).ok_or_else(|| {
                syn::Error::new_spanned(&plan.plan.output, "missing reduction occurrence rename")
            })?;
            if !matches!(definition.operator, ra::Operator::Rename(_)) {
                return Err(syn::Error::new_spanned(
                    definition,
                    "reduction requires all original renames first",
                ));
            }
            checker.assemble(definition)?;
            next += 1;
        }
        let mut current = witness.occurrences.clone();
        for (target, support) in schedule {
            let Some(pair) = definitions.get(next..next + 2) else {
                return Err(syn::Error::new_spanned(
                    &plan.plan.output,
                    "incomplete two-pass reduction schedule",
                ));
            };
            checker.reduce(&pair[0], &pair[1], &current, target, support)?;
            current[target] = symbol_name(&pair[1].result);
            next += 2;
        }
        // Freeze the latest full-schema occurrences. Only these references may
        // seed assembly; stale predecessors remain legal RA dependencies only.
        checker.frontier = current
            .into_iter()
            .enumerate()
            .map(|(i, result)| {
                (
                    result,
                    Group {
                        occurrences: BTreeSet::from([i]),
                        heading: checker.variables[i].clone(),
                    },
                )
            })
            .collect();
    }
    for definition in &definitions[next..] {
        // Original renames must not be reintroduced after a reduction.
        if witness.reduction_tree.is_some()
            && matches!(definition.operator, ra::Operator::Rename(_))
        {
            return Err(syn::Error::new_spanned(
                definition,
                "rename after reduction freeze",
            ));
        }
        checker.assemble(definition)?;
    }
    let output = &plan.plan.output.input;
    let group = checker.take(output)?;
    if !checker.frontier.is_empty() || group.occurrences != (0..checker.variables.len()).collect() {
        return Err(syn::Error::new_spanned(
            output,
            "incomplete occurrence coverage at output",
        ));
    }
    if group.heading != checker.head {
        return Err(syn::Error::new_spanned(
            output,
            "output must retain exactly the head variables",
        ));
    }
    Ok(VerifiedPlan {
        plan,
        report: checker.report,
    })
}

fn check_occurrences(
    source: &cq::Module,
    plan: &ra::Module,
    witness: &Witness,
) -> syn::Result<BTreeMap<String, usize>> {
    let error = || {
        syn::Error::new_spanned(
            &plan.program,
            "occurrence coverage requires one distinct Rename per source atom",
        )
    };
    if witness.occurrences.len() != source.program.query.body.len() {
        return Err(error());
    }
    let mut occurrences = BTreeMap::new();
    for (i, (result, item)) in witness
        .occurrences
        .iter()
        .zip(&source.program.query.body)
        .enumerate()
    {
        if occurrences.insert(result.clone(), i).is_some() {
            return Err(error());
        }
        let definition = plan
            .plan
            .definitions
            .iter()
            .find(|d| symbol_name(&d.result) == *result)
            .ok_or_else(error)?;
        let ra::Operator::Rename(rename) = &definition.operator else {
            return Err(error());
        };
        let atom = item.positive_atom().expect("checked legacy fragment");
        let declaration = source
            .program
            .inputs()
            .find(|r| symbol_name(&r.name) == symbol_name(&atom.relation))
            .expect("checked CQ declares each atom");
        let exact = symbol_name(&rename.relation) == symbol_name(&atom.relation)
            && declaration
                .columns
                .iter()
                .zip(atom.variables())
                .all(|(column, variable)| {
                    rename.mapping.iter().any(|mapping| {
                        symbol_name(&mapping.source) == symbol_name(&column.name)
                            && symbol_name(&mapping.target) == symbol_name(variable)
                    })
                });
        if !exact {
            return Err(syn::Error::new_spanned(
                definition,
                format!("incorrect occurrence mapping for source atom {i}"),
            ));
        }
    }
    Ok(occurrences)
}

struct Group {
    occurrences: BTreeSet<usize>,
    heading: Heading,
}

struct Checker {
    variables: Vec<Heading>,
    head: Heading,
    occurrences: BTreeMap<String, usize>,
    frontier: BTreeMap<String, Group>,
    policy: Policy,
    report: Report,
}

impl Checker {
    fn required(&self, completed: &BTreeSet<usize>) -> Heading {
        let mut available = Heading::new();
        let mut future = self.head.clone();
        for (i, variables) in self.variables.iter().enumerate() {
            if completed.contains(&i) {
                available.extend(variables.iter().cloned());
            } else {
                future.extend(variables.iter().cloned());
            }
        }
        available.intersection(&future).cloned().collect()
    }

    fn take(&mut self, result: &Ident) -> syn::Result<Group> {
        self.frontier.remove(&symbol_name(result)).ok_or_else(|| {
            syn::Error::new_spanned(
                result,
                format!("`{result}` is not a current assembly frontier operand"),
            )
        })
    }

    fn normalized(&self, input: &Ident, group: &Group) -> syn::Result<()> {
        let required = self.required(&group.occurrences);
        if self.policy.projection == ProjectionPolicy::Eager && group.heading != required {
            let dead: Vec<_> = group.heading.difference(&required).collect();
            return Err(syn::Error::new_spanned(
                input,
                format!(
                    "eager pruning required before consuming `{input}`; dead variables {dead:?}"
                ),
            ));
        }
        Ok(())
    }

    fn record(&mut self, definition: &ra::Definition, heading: Heading) -> syn::Result<()> {
        let arity = heading.len();
        if let Some(limit) = self.policy.max_arity {
            if arity > limit {
                return Err(syn::Error::new_spanned(
                    definition,
                    format!(
                        "result `{}` arity {arity} exceeds limit {limit}",
                        definition.result
                    ),
                ));
            }
        }
        self.report.peak_arity = self.report.peak_arity.max(arity);
        self.report
            .headings
            .insert(symbol_name(&definition.result), heading);
        Ok(())
    }

    fn join_heading(&mut self, result: &Ident, join: &ra::NaturalJoin) -> Heading {
        let left = &self.report.headings[&symbol_name(&join.left)];
        let right = &self.report.headings[&symbol_name(&join.right)];
        let key = left.intersection(right).cloned().collect();
        self.report.join_keys.insert(symbol_name(result), key);
        left.union(right).cloned().collect()
    }

    fn assemble(&mut self, definition: &ra::Definition) -> syn::Result<()> {
        let result = symbol_name(&definition.result);
        let group = match &definition.operator {
            ra::Operator::Rename(_) => {
                let Some(&i) = self.occurrences.get(&result) else {
                    return Err(syn::Error::new_spanned(
                        definition,
                        "Rename is not assigned a source occurrence",
                    ));
                };
                Group {
                    occurrences: BTreeSet::from([i]),
                    heading: self.variables[i].clone(),
                }
            }
            ra::Operator::NaturalJoin(join) => {
                let left = self.take(&join.left)?;
                let right = self.take(&join.right)?;
                self.normalized(&join.left, &left)?;
                self.normalized(&join.right, &right)?;
                // Consuming frontier entries maintains a partition. The check
                // remains explicit to document the administrative premise.
                if !left.occurrences.is_disjoint(&right.occurrences) {
                    return Err(syn::Error::new_spanned(
                        join,
                        "assembly occurrences overlap",
                    ));
                }
                Group {
                    occurrences: left
                        .occurrences
                        .union(&right.occurrences)
                        .copied()
                        .collect(),
                    heading: self.join_heading(&definition.result, join),
                }
            }
            ra::Operator::Project(project) => {
                let mut input = self.take(&project.input)?;
                let heading = project.attributes.iter().map(symbol_name).collect();
                let required = self.required(&input.occurrences);
                if !required.is_subset(&heading) {
                    let missing: Vec<_> = required.difference(&heading).collect();
                    return Err(syn::Error::new_spanned(
                        project,
                        format!("unsafe projection loses required variables {missing:?}"),
                    ));
                }
                input.heading = heading;
                input
            }
            ra::Operator::Extend(_)
            | ra::Operator::Unit(_)
            | ra::Operator::Semijoin(_)
            | ra::Operator::Select(_) => {
                return Err(syn::Error::new_spanned(
                    definition,
                    "Unit has no administrative rule in the nonempty positive CQ fragment",
                ));
            }
        };
        self.record(definition, group.heading.clone())?;
        self.frontier.insert(result, group);
        Ok(())
    }

    fn reduce(
        &mut self,
        join_definition: &ra::Definition,
        project_definition: &ra::Definition,
        current: &[String],
        target: usize,
        support: usize,
    ) -> syn::Result<()> {
        let ra::Operator::NaturalJoin(join) = &join_definition.operator else {
            return Err(syn::Error::new_spanned(
                join_definition,
                "reduction step requires a NaturalJoin/Project pair",
            ));
        };
        if symbol_name(&join.left) != current[target]
            || symbol_name(&join.right) != current[support]
        {
            return Err(syn::Error::new_spanned(
                join,
                format!(
                    "expected latest reduction versions: {} with {}",
                    current[target], current[support]
                ),
            ));
        }
        let ra::Operator::Project(project) = &project_definition.operator else {
            return Err(syn::Error::new_spanned(
                project_definition,
                "reduction step requires a NaturalJoin/Project pair",
            ));
        };
        let heading = project
            .attributes
            .iter()
            .map(symbol_name)
            .collect::<Heading>();
        if symbol_name(&project.input) != symbol_name(&join_definition.result)
            || heading != self.variables[target]
        {
            return Err(syn::Error::new_spanned(
                project,
                "reduction schema must equal the target occurrence schema and project this step's join",
            ));
        }
        let joined = self.join_heading(&join_definition.result, join);
        self.record(join_definition, joined)?;
        self.record(project_definition, heading)
    }
}

/// Validate a rooted join tree and derive its fixed postorder/preorder schedule.
/// Iterative traversals avoid relying on the compiler process's stack depth.
fn reduction_schedule(
    parents: &[Option<usize>],
    variables: &[Heading],
    context: &cq::Program,
) -> syn::Result<Vec<(usize, usize)>> {
    let error = |message| syn::Error::new_spanned(context, message);
    let n = variables.len();
    if parents.len() != n {
        return Err(error("join tree must contain every occurrence"));
    }
    let roots: Vec<_> = parents
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.is_none().then_some(i))
        .collect();
    if roots.len() != 1 {
        return Err(error("join tree must have exactly one root"));
    }
    let mut children = vec![Vec::new(); n];
    let mut adjacent = vec![Vec::new(); n];
    for (i, parent) in parents.iter().enumerate() {
        if let Some(p) = *parent {
            if p >= n || p == i {
                return Err(error("invalid parent occurrence in join tree"));
            }
            children[p].push(i);
            adjacent[p].push(i);
            adjacent[i].push(p);
        }
    }
    let mut preorder = Vec::new();
    let mut postorder = Vec::new();
    let mut visited = BTreeSet::new();
    let mut pending = vec![(roots[0], false)];
    while let Some((i, leaving)) = pending.pop() {
        if leaving {
            postorder.push(i);
        } else {
            if !visited.insert(i) {
                return Err(error("witness is not a connected tree"));
            }
            preorder.push(i);
            pending.push((i, true));
            pending.extend(children[i].iter().rev().map(|c| (*c, false)));
        }
    }
    if visited.len() != n {
        return Err(error("witness is not a connected tree"));
    }
    let all_variables: Heading = variables.iter().flat_map(|v| v.iter().cloned()).collect();
    for variable in &all_variables {
        let containing: BTreeSet<_> = (0..n)
            .filter(|i| variables[*i].contains(variable))
            .collect();
        let mut reached = BTreeSet::new();
        let mut pending = vec![*containing.first().expect("variable has an occurrence")];
        while let Some(i) = pending.pop() {
            if reached.insert(i) {
                pending.extend(
                    adjacent[i]
                        .iter()
                        .copied()
                        .filter(|j| containing.contains(j)),
                );
            }
        }
        if reached != containing {
            return Err(error("join tree violates running intersection"));
        }
    }
    let up = postorder
        .into_iter()
        .filter_map(|i| parents[i].map(|p| (p, i)));
    let down = preorder
        .into_iter()
        .filter_map(|i| parents[i].map(|p| (i, p)));
    Ok(up.chain(down).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heading(names: &[&str]) -> Heading {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn remaining_uses_include_the_head_and_both_sides_of_a_bushy_cut() {
        let checker = Checker {
            variables: vec![
                heading(&["a", "b"]),
                heading(&["b", "c"]),
                heading(&["c", "d"]),
                heading(&["d", "e"]),
                heading(&["u"]),
            ],
            head: heading(&["a", "e"]),
            occurrences: BTreeMap::new(),
            frontier: BTreeMap::new(),
            policy: Policy {
                projection: ProjectionPolicy::Eager,
                max_arity: None,
            },
            report: Report {
                headings: BTreeMap::new(),
                join_keys: BTreeMap::new(),
                peak_arity: 0,
            },
        };
        for (completed, expected) in [
            (vec![0, 1], heading(&["a", "c"])),
            (vec![2, 3], heading(&["c", "e"])),
            (vec![0, 3], heading(&["a", "b", "d", "e"])),
            (vec![4], Heading::new()),
            (vec![0, 1, 2, 3, 4], heading(&["a", "e"])),
        ] {
            assert_eq!(checker.required(&completed.into_iter().collect()), expected);
        }
    }

    #[test]
    fn branched_reduction_visits_descendants_before_upward_and_after_downward_edges() {
        let context: cq::Module =
            syn::parse_str("struct P; relation R(x: i32); answer(x) :- R(x);").unwrap();
        let schedule = reduction_schedule(
            &[None, Some(0), Some(0), Some(0), Some(1), Some(2)],
            &vec![heading(&["x"]); 6],
            &context.program,
        )
        .unwrap();
        assert_eq!(
            schedule,
            vec![
                (1, 4),
                (0, 1),
                (2, 5),
                (0, 2),
                (0, 3),
                (1, 0),
                (4, 1),
                (2, 0),
                (5, 2),
                (3, 0),
            ]
        );
    }

    #[test]
    fn connected_is_insufficient_without_running_intersection() {
        let context: cq::Module =
            syn::parse_str("struct P; relation R(x: i32); answer(x) :- R(x);").unwrap();
        let error = reduction_schedule(
            &[None, Some(0), Some(1)],
            &[
                heading(&["a", "b"]),
                heading(&["b", "c"]),
                heading(&["c", "a"]),
            ],
            &context.program,
        )
        .unwrap_err();
        assert!(error.to_string().contains("running intersection"));
    }
}
