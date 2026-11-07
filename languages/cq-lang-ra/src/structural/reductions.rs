//! Narrow RA-to-RA certificates for reduction absorption and dead expansion.
//!
//! The ordinary schedule contract preserves independent witness wiring.
//! Absorption and forward dead expansion prove named-definition substitutions.
//! The inverse dead-expansion certificate also compares independently named
//! programs, but only after source-side demand certifies each exposed support.
//! None of these proofs merges independently projected witnesses.

use super::{Atoms, Kind, Structure};
use crate::{Operator, Program, symbol_name};
use std::collections::{HashMap, HashSet};

pub(super) fn check(
    source: &Program,
    target: &Program,
    schedule: &Structure,
    atoms: &mut Atoms,
) -> bool {
    check_absorption(source, target, atoms)
        || check_dead_expansion(source, target, schedule, atoms)
        || check_inverse_dead_expansion(schedule, target, atoms)
}

/// Compare independently named programs through the inverse of dead expansion.
/// Every exposed support is first certified by the forward law's source-side
/// demand check; the final structural comparison still preserves witness wiring.
fn check_inverse_dead_expansion(
    source: &Structure,
    target: &Program,
    atoms: &mut Atoms,
) -> bool {
    if source.nodes.iter().any(|node| {
        !matches!(
            node.kind,
            Kind::Unit | Kind::Atom(_) | Kind::Project | Kind::Join(_) | Kind::Select
        )
    }) || target.plan.definitions.iter().any(|definition| {
        !matches!(
            definition.operator,
            Operator::Unit(_)
                | Operator::Rename(_)
                | Operator::Project(_)
                | Operator::NaturalJoin(_)
                | Operator::Semijoin(_)
                | Operator::Select(_)
        )
    }) {
        return false;
    }
    let mut expanded = target.clone();
    // Expose outer supports first, without merging independent witnesses.
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
        let Ok(schedule) = Structure::from_program(&expanded, atoms) else {
            expanded.plan.definitions[index].operator = previous;
            continue;
        };
        // Demand belongs to the expanded SOURCE of the forward identity:
        // newly introduced downstream join keys must not become invisible.
        let demand = source_demand(&expanded, &schedule);
        let left = schedule.nodes[index].operands[0];
        if demand[index]
            .iter()
            .any(|name| !schedule.nodes[left].heading.contains_key(name))
        {
            expanded.plan.definitions[index].operator = previous;
            continue;
        }
        if source.check(&schedule).is_ok() {
            return true;
        }
    }
    false
}

// Formal rule RA-Absorb: Certify linear key-retaining pipes to a current named support witness, independent of serialization order.
fn check_absorption(source: &Program, target: &Program, atoms: &mut Atoms) -> bool {
    if source.plan.output != target.plan.output {
        return false;
    }
    let targets: HashMap<_, _> = target
        .plan
        .definitions
        .iter()
        .map(|d| (symbol_name(&d.result), d))
        .collect();
    let mut current = source.clone();
    loop {
        if current.plan.definitions.len() == targets.len()
            && current.plan.definitions.iter().all(|d| {
                targets.get(&symbol_name(&d.result)) == Some(&d)
            })
        {
            return true;
        }
        let Ok(schedule) = Structure::from_program(&current, atoms) else {
            return false;
        };
        let mut uses = vec![(0, 0); schedule.nodes.len()];
        let mut negative = vec![false; schedule.nodes.len()];
        let mut opaque = vec![false; schedule.nodes.len()];
        for (i, node) in schedule.nodes.iter().enumerate() {
            for input in dependencies(node) {
                uses[input].0 += 1;
                uses[input].1 = i;
            }
            if matches!(node.kind, Kind::Anti(_)) {
                negative[node.operands[1]] = true;
            }
            opaque[i] = matches!(node.kind, Kind::Group(_) | Kind::Rust(_))
                || dependencies(node).any(|input| opaque[input]);
        }
        let output = current
            .plan
            .definitions
            .iter()
            .position(|d| d.result == current.plan.output.input)
            .unwrap();
        uses[output].0 += 1;
        uses[output].1 = schedule.nodes.len();
        for (i, node) in schedule.nodes.iter().enumerate().rev() {
            if negative[i] {
                for input in dependencies(node) {
                    negative[input] = true;
                }
            }
        }
        let proof = Proof {
            schedule: &schedule,
            negative,
            grouped: opaque,
            retained: current
                .plan
                .definitions
                .iter()
                .map(|d| targets.contains_key(&symbol_name(&d.result)))
                .collect(),
        };
        // The target names the eliminated reductions; the certificate proves
        // their identities, without calling or comparing with the producer.
        let Some(index) = (0..schedule.nodes.len()).rev().find(|&i| {
            !proof.retained[i]
                && !proof.negative[i]
                && !proof.grouped[i]
                && (proof.linear_absorption(i, &uses) || proof.direct_absorption(i, output))
        }) else {
            return false;
        };
        let base = current.plan.definitions[schedule.nodes[index].operands[0]].result.clone();
        apply_rewrite(&mut current, index, Rewrite::Bypass(base));
        let mut live = vec![false; schedule.nodes.len()];
        live[output] = true;
        for i in (0..schedule.nodes.len()).rev() {
            if live[i] {
                for input in dependencies(&schedule.nodes[i]) {
                    let input = if input == index {
                        schedule.nodes[index].operands[0]
                    } else {
                        input
                    };
                    live[input] = true;
                }
            }
        }
        let mut i = 0;
        current.plan.definitions.retain(|_| {
            let keep = live[i];
            i += 1;
            keep
        });
    }
}

/// This certificate does not relax the ordinary schedule contract. It replays
/// only independently proved local set laws, then requires the exact target.
fn check_dead_expansion(
    source: &Program,
    target: &Program,
    schedule: &Structure,
    atoms: &mut Atoms,
) -> bool {
    // Opaque execution and block/negative ancestry are outside these set laws.
    if source.plan.output.head != target.plan.output.head
        || source.plan.definitions.iter().any(|definition| {
            matches!(
                definition.operator,
                Operator::Bind(_)
                    | Operator::Guard(_)
                    | Operator::Anti(_)
                    | Operator::Union(_)
                    | Operator::Group(_)
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
            grouped: vec![false; current_schedule.nodes.len()],
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
        let Ok(next_schedule) = Structure::from_program(&current, atoms) else {
            return false;
        };
        current_schedule = next_schedule;
        // Each step removes a join, changes natural join to semijoin, or
        // strictly shortens a semijoin support chain: replay terminates.
    }
}

/// Demand is computed from the SOURCE, including keys of every later join.
/// Target heading validity alone cannot detect an erased implicit equality.
fn source_demand(source: &Program, schedule: &Structure) -> Vec<HashSet<String>> {
    let mut demand = vec![HashSet::new(); schedule.nodes.len()];
    if let Some(output) = schedule.output {
        demand[output].extend(schedule.nodes[output].heading.keys().cloned());
    }
    for (index, definition) in source.plan.definitions.iter().enumerate().rev() {
        let node = &schedule.nodes[index];
        match &definition.operator {
            Operator::Group(_) => {
                let input = node.operands[0];
                demand[input].extend(schedule.nodes[input].heading.keys().cloned());
            }
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
                    Operator::Group(group) => replace(&mut group.input),
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
            Operator::Group(group) => {
                live.insert(symbol_name(&group.input));
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
    schedule: &'a Structure,
    negative: Vec<bool>,
    grouped: Vec<bool>,
    retained: Vec<bool>,
}

impl Proof<'_> {
    // Formal rule RA-AbsorbDirect: Certify simultaneous bypass of every direct absorbing use of a shared reduction.
    fn direct_absorption(&self, reduction: usize, output: usize) -> bool {
        let node = &self.schedule.nodes[reduction];
        if reduction == output || !matches!(node.kind, Kind::Join(true)) {
            return false;
        }
        let support = node.operands[1];
        let keys: Vec<_> = self.schedule.nodes[node.operands[0]]
            .heading
            .keys()
            .filter(|key| self.schedule.nodes[support].heading.contains_key(*key))
            .collect();
        let mut used = false;
        for consumer in &self.schedule.nodes {
            if consumer.correlation == Some(reduction) || consumer.domain == Some(reduction) {
                return false;
            }
            for (side, &input) in consumer.operands.iter().enumerate() {
                if input != reduction {
                    continue;
                }
                if !matches!(consumer.kind, Kind::Join(_))
                    || !self.contained(consumer.operands[1 - side], support, &keys, false)
                {
                    return false;
                }
                used = true;
            }
        }
        used
    }

    fn linear_absorption(&self, reduction: usize, uses: &[(usize, usize)]) -> bool {
        let node = &self.schedule.nodes[reduction];
        if !matches!(node.kind, Kind::Join(true)) {
            return false;
        }
        let support = node.operands[1];
        let keys: Vec<_> = self.schedule.nodes[node.operands[0]]
            .heading
            .keys()
            .filter(|key| self.schedule.nodes[support].heading.contains_key(*key))
            .collect();
        let mut previous = reduction;
        loop {
            if keys
                .iter()
                .any(|key| !self.schedule.nodes[previous].heading.contains_key(*key))
            {
                return false;
            }
            let (1, consumer) = uses[previous] else { return false };
            let Some(next) = self.schedule.nodes.get(consumer) else {
                return false;
            };
            match next.kind {
                Kind::Join(semi) => {
                    let side = usize::from(next.operands[0] != previous);
                    if self.contained(next.operands[1 - side], support, &keys, false) {
                        return true;
                    }
                    if semi && side == 1 {
                        return false;
                    }
                }
                Kind::Select => {}
                _ => return false,
            }
            previous = consumer;
        }
    }

    // The separate dead-expansion certificate retains its local support law;
    // general reduction absorption is certified by linear_absorption above.
    fn absorbed(&self, mut reduced: usize, replacement: usize, other: usize) -> bool {
        while reduced != replacement {
            let node = &self.schedule.nodes[reduced];
            if !matches!(node.kind, Kind::Join(true))
                || self.negative[reduced]
                || self.grouped[reduced]
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
