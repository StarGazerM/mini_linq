//! Narrow RA-to-RA certificates for reduction absorption and dead expansion.
//!
//! The ordinary schedule contract preserves independent witness wiring. This
//! additional proof checks actual named-definition substitutions instead of
//! relaxing that contract or identifying witnesses by occurrence number.

use super::{Kind, Schedule};
use crate::{Operator, Program, symbol_name};
use std::collections::{HashMap, HashSet};

pub(super) fn check(source: &Program, target: &Program, schedule: &Schedule) -> bool {
    check_absorption(source, target, schedule) || check_dead_expansion(source, target, schedule)
}

/// Supply an independently checked inverse of the dead-expansion law to the
/// VO schedule checker. This uses only the supplied target, never elaboration
/// or planner choices. Existential support equalities remain scoped until an
/// actual natural-join witness has been proved equivalent to that target.
pub(super) fn check_expanded(source: &Schedule, target: &Program) -> bool {
    if source.nodes.iter().any(|node| {
        !matches!(
            node.kind,
            Kind::Unit | Kind::Atom(_) | Kind::Project | Kind::Join(_) | Kind::Select
        )
    }) || target.plan.definitions.iter().any(|definition| {
        matches!(
            definition.operator,
            Operator::Bind(_) | Operator::Guard(_) | Operator::Anti(_) | Operator::Union(_)
        )
    }) {
        return false;
    }
    let mut expanded = target.clone();
    // Start outside support subtrees: exposing an outer semijoin's hidden
    // columns can restore primary access to the exact named support, without
    // flattening independent witnesses into one occurrence-wide equality.
    for index in (0..expanded.plan.definitions.len()).rev() {
        let Operator::Semijoin(join) = &expanded.plan.definitions[index].operator else {
            continue;
        };
        let replacement = Operator::NaturalJoin(Box::new(crate::NaturalJoin {
            natural_join_token: Default::default(),
            left: join.left.clone(),
            with_token: join.with_token,
            right: join.right.clone(),
        }));
        let previous =
            std::mem::replace(&mut expanded.plan.definitions[index].operator, replacement);
        let Ok(schedule) = Schedule::from_program(&expanded) else {
            expanded.plan.definitions[index].operator = previous;
            continue;
        };
        // The expanded program is the SOURCE of the forward set identity.
        // This catches new implicit downstream keys that did not exist in the
        // supplied target's headings, as well as selections and output.
        let demand = source_demand(&expanded, &schedule);
        let left = schedule.nodes[index].operands[0];
        if demand[index]
            .iter()
            .any(|name| !schedule.nodes[left].heading.contains_key(name))
        {
            expanded.plan.definitions[index].operator = previous;
            continue;
        }
        if super::compare(source, &schedule).is_ok() {
            return true;
        }
    }
    false
}

fn check_absorption(source: &Program, target: &Program, schedule: &Schedule) -> bool {
    if source.plan.output != target.plan.output {
        return false;
    }
    // from_program emits exactly one schedule node per named definition.
    let indices: HashMap<_, _> = source
        .plan
        .definitions
        .iter()
        .enumerate()
        .map(|(index, definition)| (symbol_name(&definition.result), index))
        .collect();
    let mut negative = vec![false; schedule.nodes.len()];
    let mut opaque = vec![false; schedule.nodes.len()];
    for (index, node) in schedule.nodes.iter().enumerate() {
        if matches!(node.kind, Kind::Anti(_)) {
            negative[node.operands[1]] = true;
        }
        opaque[index] =
            matches!(node.kind, Kind::Rust(_)) || dependencies(node).any(|input| opaque[input]);
    }
    for (index, node) in schedule.nodes.iter().enumerate().rev() {
        if negative[index] {
            for input in dependencies(node) {
                negative[input] = true;
            }
        }
    }
    let mut retained = vec![false; schedule.nodes.len()];
    for definition in &target.plan.definitions {
        let Some(&index) = indices.get(&symbol_name(&definition.result)) else {
            return false;
        };
        retained[index] = true;
    }
    let proof = Proof {
        schedule,
        negative,
        opaque,
        retained,
    };
    let mut remaining = target.plan.definitions.iter().peekable();
    let mut substituted = false;
    for (index, definition) in source.plan.definitions.iter().enumerate() {
        let Some(other) = remaining
            .peek()
            .filter(|other| definition.result == other.result)
        else {
            // Only a bypassed semijoin can become dead: its base is retained,
            // and its support is still witnessed by the consumer's lineage.
            // The already-checked target contract proves no live reference to
            // this omitted definition remains.
            if !matches!(definition.operator, Operator::Semijoin(_))
                || proof.negative[index]
                || proof.opaque[index]
            {
                return false;
            }
            continue;
        };
        if definition.operator != other.operator {
            if proof.negative[index] {
                return false;
            }
            let replacement = match (&definition.operator, &other.operator) {
                (Operator::Semijoin(a), Operator::Semijoin(b)) if a.left == b.left => &b.right,
                (Operator::NaturalJoin(a), Operator::NaturalJoin(b)) if a.left == b.left => {
                    &b.right
                }
                _ => return false,
            };
            let Some(&replacement) = indices.get(&symbol_name(replacement)) else {
                return false;
            };
            let node = &schedule.nodes[index];
            if !proof.absorbed(node.operands[1], replacement, node.operands[0]) {
                return false;
            }
            substituted = true;
        }
        remaining.next();
    }
    substituted && remaining.next().is_none()
}

/// This certificate does not relax the ordinary schedule contract. It replays
/// only independently proved local set laws, then requires the exact target.
fn check_dead_expansion(source: &Program, target: &Program, schedule: &Schedule) -> bool {
    // Opaque execution and block/negative ancestry are outside these set laws.
    if source.plan.output.head != target.plan.output.head
        || source.plan.definitions.iter().any(|definition| {
            matches!(
                definition.operator,
                Operator::Bind(_) | Operator::Guard(_) | Operator::Anti(_) | Operator::Union(_)
            )
        })
    {
        return false;
    }
    let retained: HashSet<_> = target
        .plan
        .definitions
        .iter()
        .map(|definition| symbol_name(&definition.result))
        .collect();
    let targets: HashMap<_, _> = target
        .plan
        .definitions
        .iter()
        .map(|definition| (symbol_name(&definition.result), &definition.operator))
        .collect();
    let mut current = source.clone();
    let mut current_schedule = schedule.clone();
    loop {
        if current.plan.definitions == target.plan.definitions
            && current.plan.output == target.plan.output
        {
            return true;
        }
        // Recompute from the current independently certified SOURCE, never
        // from the proposed target. A proved bypass can make an earlier
        // expansion's formerly shared columns unobservable.
        let demand = source_demand(&current, &current_schedule);
        let proof = Proof {
            schedule: &current_schedule,
            negative: vec![false; current_schedule.nodes.len()],
            opaque: vec![false; current_schedule.nodes.len()],
            retained: current
                .plan
                .definitions
                .iter()
                .map(|definition| retained.contains(&symbol_name(&definition.result)))
                .collect(),
        };
        let indices: HashMap<_, _> = current
            .plan
            .definitions
            .iter()
            .enumerate()
            .map(|(index, definition)| (symbol_name(&definition.result), index))
            .collect();
        let mut rewrite = None;
        for (index, definition) in current.plan.definitions.iter().enumerate() {
            let (left_name, right_name, natural) = match &definition.operator {
                Operator::NaturalJoin(join) => (&join.left, &join.right, true),
                Operator::Semijoin(join) => (&join.left, &join.right, false),
                _ => continue,
            };
            let node = &current_schedule.nodes[index];
            let left = node.operands[0];
            let right = node.operands[1];
            let dead_columns = demand[index]
                .iter()
                .all(|name| current_schedule.nodes[left].heading.contains_key(name));
            let target_operator = targets.get(&symbol_name(&definition.result));
            if target_operator.is_none() && (!natural || dead_columns) {
                let shared: Vec<_> = current_schedule.nodes[left]
                    .heading
                    .keys()
                    .filter(|name| current_schedule.nodes[right].heading.contains_key(*name))
                    .collect();
                if proof.contained(left, right, &shared, true) {
                    rewrite = Some((index, Rewrite::Bypass(left_name.clone())));
                    break;
                }
            }
            if natural && dead_columns && matches!(target_operator, Some(Operator::Semijoin(_))) {
                rewrite = Some((index, Rewrite::Membership));
                break;
            }
            let replacement = match target_operator {
                Some(Operator::NaturalJoin(join)) if natural && join.left == *left_name => {
                    &join.right
                }
                Some(Operator::Semijoin(join)) if !natural && join.left == *left_name => {
                    &join.right
                }
                _ => continue,
            };
            if replacement != right_name
                && let Some(&replacement_index) = indices.get(&symbol_name(replacement))
                && proof.absorbed(right, replacement_index, left)
            {
                rewrite = Some((index, Rewrite::Support(replacement.clone())));
                break;
            }
        }
        let Some((index, rewrite)) = rewrite else {
            return false;
        };
        apply_rewrite(&mut current, index, rewrite);
        prune_unreachable(&mut current, &retained);
        let Ok(next_schedule) = Schedule::from_program(&current) else {
            return false;
        };
        current_schedule = next_schedule;
        // Each step removes a join, changes natural join to semijoin, or
        // strictly shortens a semijoin support chain: replay terminates.
    }
}

/// Demand is computed from the SOURCE, including keys of every later join.
/// Target heading validity alone cannot detect an erased implicit equality.
fn source_demand(source: &Program, schedule: &Schedule) -> Vec<HashSet<String>> {
    let mut demand = vec![HashSet::new(); schedule.nodes.len()];
    if let Some(output) = schedule.output {
        demand[output].extend(schedule.nodes[output].heading.keys().cloned());
    }
    for (index, definition) in source.plan.definitions.iter().enumerate().rev() {
        let node = &schedule.nodes[index];
        match &definition.operator {
            Operator::Project(project) => {
                demand[index].extend(project.attributes.iter().map(symbol_name));
            }
            Operator::Select(select) => {
                demand[index].extend(
                    select
                        .condition
                        .equalities
                        .iter()
                        .flat_map(|equality| equality.variables())
                        .map(symbol_name),
                );
                demand[index].extend(
                    select
                        .condition
                        .constraints
                        .iter()
                        .flat_map(|constraint| constraint.variables())
                        .map(symbol_name),
                );
            }
            Operator::NaturalJoin(_) | Operator::Semijoin(_) => {
                let left = &schedule.nodes[node.operands[0]].heading;
                let right = &schedule.nodes[node.operands[1]].heading;
                demand[index].extend(
                    left.keys()
                        .filter(|name| right.contains_key(*name))
                        .cloned(),
                );
            }
            _ => {}
        }
        // Edges always point backwards, so split borrowing avoids temporary
        // copies of each node's demand during propagation.
        let (earlier, current) = demand.split_at_mut(index);
        for &input in &node.operands {
            earlier[input].extend(
                current[0]
                    .iter()
                    .filter(|name| schedule.nodes[input].heading.contains_key(*name))
                    .cloned(),
            );
        }
    }
    demand
}

enum Rewrite {
    Bypass(syn::Ident),
    Membership,
    Support(syn::Ident),
}

fn apply_rewrite(program: &mut Program, index: usize, rewrite: Rewrite) {
    match rewrite {
        Rewrite::Bypass(left) => {
            let result = program.plan.definitions[index].result.clone();
            let replace = |input: &mut syn::Ident| {
                if *input == result {
                    *input = left.clone();
                }
            };
            for definition in &mut program.plan.definitions {
                match &mut definition.operator {
                    Operator::NaturalJoin(join) => {
                        replace(&mut join.left);
                        replace(&mut join.right);
                    }
                    Operator::Semijoin(join) => {
                        replace(&mut join.left);
                        replace(&mut join.right);
                    }
                    Operator::Project(project) => replace(&mut project.input),
                    Operator::Select(select) => replace(&mut select.input),
                    _ => {}
                }
            }
            replace(&mut program.plan.output.input);
        }
        Rewrite::Membership => {
            let Operator::NaturalJoin(join) = &program.plan.definitions[index].operator else {
                unreachable!();
            };
            program.plan.definitions[index].operator =
                Operator::Semijoin(Box::new(crate::Semijoin {
                    semijoin_token: Default::default(),
                    left: join.left.clone(),
                    with_token: join.with_token,
                    right: join.right.clone(),
                }));
        }
        Rewrite::Support(right) => match &mut program.plan.definitions[index].operator {
            Operator::NaturalJoin(join) => join.right = right,
            Operator::Semijoin(join) => join.right = right,
            _ => unreachable!(),
        },
    }
}

fn prune_unreachable(program: &mut Program, retained: &HashSet<String>) {
    let mut live = retained.clone();
    live.insert(symbol_name(&program.plan.output.input));
    for definition in program.plan.definitions.iter().rev() {
        if !live.contains(&symbol_name(&definition.result)) {
            continue;
        }
        match &definition.operator {
            Operator::NaturalJoin(join) => {
                live.insert(symbol_name(&join.left));
                live.insert(symbol_name(&join.right));
            }
            Operator::Semijoin(join) => {
                live.insert(symbol_name(&join.left));
                live.insert(symbol_name(&join.right));
            }
            Operator::Project(project) => {
                live.insert(symbol_name(&project.input));
            }
            Operator::Select(select) => {
                live.insert(symbol_name(&select.input));
            }
            _ => {}
        }
    }
    program
        .plan
        .definitions
        .retain(|definition| live.contains(&symbol_name(&definition.result)));
}

fn dependencies(node: &super::Node) -> impl Iterator<Item = usize> + '_ {
    node.operands
        .iter()
        .copied()
        .chain(node.correlation)
        .chain(node.domain)
}

struct Proof<'a> {
    schedule: &'a Schedule,
    negative: Vec<bool>,
    opaque: Vec<bool>,
    retained: Vec<bool>,
}

impl Proof<'_> {
    fn absorbed(&self, mut reduced: usize, replacement: usize, other: usize) -> bool {
        while reduced != replacement {
            let node = &self.schedule.nodes[reduced];
            if !matches!(node.kind, Kind::Join(true))
                || self.negative[reduced]
                || self.opaque[reduced]
                // A surviving restricted row still supplies correlated
                // projected witnesses. All its consumers must keep that
                // same row rather than partially bypass its restrictions.
                || self.retained[reduced]
            {
                return false;
            }
            let base = node.operands[0];
            let support = node.operands[1];
            let shared: Vec<_> = self.schedule.nodes[base]
                .heading
                .keys()
                .filter(|name| self.schedule.nodes[support].heading.contains_key(*name))
                .collect();
            if !self.contained(other, support, &shared, false) {
                return false;
            }
            reduced = base;
        }
        true
    }

    fn contained(
        &self,
        other: usize,
        support: usize,
        shared: &[&String],
        follow_support: bool,
    ) -> bool {
        let mut pending = vec![other];
        let mut seen = HashSet::new();
        while let Some(index) = pending.pop() {
            let node = &self.schedule.nodes[index];
            if !seen.insert(index) || shared.iter().any(|name| !node.heading.contains_key(*name)) {
                continue;
            }
            if index == support {
                return true;
            }
            match node.kind {
                Kind::Project | Kind::Select => pending.push(node.operands[0]),
                Kind::Join(true) if !follow_support => pending.push(node.operands[0]),
                // The new expansion certificate may follow an existential
                // right witness. The existing absorption contract keeps its
                // original left-only semijoin lineage. Every visited node
                // must retain ALL shared names in either mode.
                Kind::Join(_) => pending.extend(&node.operands),
                _ => {}
            }
        }
        false
    }
}
