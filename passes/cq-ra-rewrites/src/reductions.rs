use std::collections::{BTreeMap, BTreeSet};

use cq_lang_ra::{self as ra, Operator, symbol_name};
use syn::Ident;

pub(super) fn simplify(source: &ra::Program) -> syn::Result<ra::Program> {
    let analysis = ra::contract::check(source)?;
    let definitions = &source.plan.definitions;
    let indices: BTreeMap<_, _> = definitions
        .iter()
        .enumerate()
        .map(|(index, definition)| (symbol_name(&definition.result), index))
        .collect();
    let dependencies: Vec<Vec<usize>> = definitions
        .iter()
        .map(|definition| {
            let mut inputs = Vec::new();
            operands(&definition.operator, |input| {
                inputs.push(indices[&symbol_name(input)]);
            });
            inputs
        })
        .collect();
    let mut consumers = vec![Vec::new(); definitions.len()];
    for (consumer, inputs) in dependencies.iter().enumerate() {
        for &input in inputs {
            consumers[input].push(consumer);
        }
    }
    let mut negative = vec![false; definitions.len()];
    let mut opaque = vec![false; definitions.len()];
    for (index, definition) in definitions.iter().enumerate() {
        if let Operator::Anti(anti) = &definition.operator {
            negative[indices[&symbol_name(&anti.right)]] = true;
        }
        opaque[index] = matches!(
            definition.operator,
            Operator::Bind(_) | Operator::Guard(_) | Operator::Group(_)
        ) || dependencies[index].iter().any(|input| opaque[*input]);
    }
    for index in (0..definitions.len()).rev() {
        if negative[index] {
            for input in &dependencies[index] {
                negative[*input] = true;
            }
        }
    }
    let headings: Vec<_> = definitions
        .iter()
        .map(|definition| &analysis[&symbol_name(&definition.result)].heading)
        .collect();
    let proof = Lineage {
        definitions,
        dependencies: &dependencies,
        consumers: &consumers,
        returned: indices[&symbol_name(&source.plan.output.input)],
        headings,
        negative,
        opaque,
    };
    let mut target = source.clone();
    for (index, definition) in target.plan.definitions.iter_mut().enumerate() {
        if proof.negative[index] {
            continue;
        }
        let (left, right) = match &mut definition.operator {
            Operator::Semijoin(join) => (&mut join.left, &mut join.right),
            Operator::NaturalJoin(join) => (&mut join.left, &mut join.right),
            _ => continue,
        };
        // Prune backward support versions first: their removal can restore
        // piped scans and filters. Then remove a duplicate left reduction only
        // when the surviving RHS proves it redundant.
        let reduced_right = proof.absorb(dependencies[index][1], dependencies[index][0]);
        let reduced_left = proof.absorb(dependencies[index][0], reduced_right);
        *right = definitions[reduced_right].result.clone();
        *left = definitions[reduced_left].result.clone();
    }
    // Partial absorption of a shared restricted result gives its projected
    // consumers different row witnesses. Keep that shared materialization
    // unless every bypassed result becomes dead. Restoring one operand can
    // revive another bypassed result, so propagate restoration to a fixed point.
    let mut reachable = vec![false; definitions.len()];
    loop {
        reachable.fill(false);
        reachable[indices[&symbol_name(&target.plan.output.input)]] = true;
        for (index, definition) in target.plan.definitions.iter().enumerate().rev() {
            if reachable[index] {
                operands(&definition.operator, |input| {
                    reachable[indices[&symbol_name(input)]] = true;
                });
            }
        }
        let mut restored = false;
        for (index, definition) in target.plan.definitions.iter_mut().enumerate() {
            let (left, right) = match &mut definition.operator {
                Operator::Semijoin(join) => (&mut join.left, &mut join.right),
                Operator::NaturalJoin(join) => (&mut join.left, &mut join.right),
                _ => continue,
            };
            for (side, operand) in [left, right].into_iter().enumerate() {
                let original = dependencies[index][side];
                let replacement = indices[&symbol_name(operand)];
                let mut bypassed = original;
                while bypassed != replacement {
                    if reachable[bypassed] {
                        *operand = definitions[original].result.clone();
                        restored = true;
                        break;
                    }
                    bypassed = dependencies[bypassed][0];
                }
            }
        }
        if !restored {
            break;
        }
    }
    // Keep the established local realization intact before proposing future
    // support. A declined proposal must not undo another local absorption.
    for (index, definition) in target.plan.definitions.iter_mut().enumerate() {
        if !reachable[index] || proof.negative[index] {
            continue;
        }
        let Some(after) = &definition.moved_after else { continue };
        if definition.check.is_none() {
            continue;
        }
        let right = match &mut definition.operator {
            Operator::Semijoin(join) => &mut join.right,
            Operator::NaturalJoin(join) => &mut join.right,
            _ => continue,
        };
        let mut reduced = indices[&symbol_name(right)];
        // Formal rule RA-MovedSupport: restore only the drained check's own
        // recorded former support, not arbitrary future reduction witnesses.
        while let Operator::Semijoin(_) = &definitions[reduced].operator {
            if proof.negative[reduced] || proof.opaque[reduced] {
                break;
            }
            let [base, support] = dependencies[reduced][..] else {
                unreachable!("semijoin has two operands")
            };
            if !proof.occurrence(support).is_some_and(|occurrence| after.occurrences.iter().any(|former| former == occurrence)) {
                break;
            }
            let shared: Vec<_> = proof.headings[base]
                .intersection(proof.headings[support]).collect();
            let Some(consumer) = proof.restoring_join(index, support, &shared) else {
                break;
            };
            definition.absorptions.push(ra::Absorption {
                absorbed_token: Default::default(),
                left_token: None,
                reduced: definitions[reduced].result.clone(),
                via_token: Default::default(),
                consumer: definitions[consumer].result.clone(),
            });
            reduced = base;
        }
        *right = definitions[reduced].result.clone();
    }
    // A drained check can separate (P semi L) from its former join with L,
    // or with the reciprocal reduction (L semi (P semi L)). Preserve those
    // ordinary local absorptions at its already-recorded restoring join.
    let mut restored_adjacency = false;
    for (index, definition) in target.plan.definitions.iter_mut().enumerate() {
        let Some(fact) = definition.absorptions.get(definitions[index].absorptions.len()) else {
            continue;
        };
        let consumer = indices[&symbol_name(&fact.consumer)];
        let left = match &mut definition.operator {
            Operator::Semijoin(join) => &mut join.left,
            Operator::NaturalJoin(join) => &mut join.left,
            _ => continue,
        };
        let reduced = indices[&symbol_name(left)];
        if !matches!(definitions[reduced].operator, Operator::Semijoin(_)) {
            continue;
        }
        let [base, support] = dependencies[reduced][..] else { unreachable!() };
        let restoring = dependencies[consumer][1];
        if restoring != support
            && (!matches!(definitions[restoring].operator, Operator::Semijoin(_))
                || dependencies[restoring] != [support, reduced])
        {
            continue;
        }
        let after = definition.moved_after.as_ref().expect("moved check");
        if !proof.occurrence(support).is_some_and(|occurrence|
            after.occurrences.iter().any(|former| former == occurrence))
        {
            continue;
        }
        definition.absorptions.push(ra::Absorption {
            absorbed_token: Default::default(),
            left_token: Some(Default::default()),
            reduced: definitions[reduced].result.clone(),
            via_token: Default::default(),
            consumer: definitions[consumer].result.clone(),
        });
        *left = definitions[base].result.clone();
        restored_adjacency = true;
    }
    if restored_adjacency {
        // Reconsider the original local proposals now that the moved check no
        // longer keeps their reciprocal reductions live. No new ordering or
        // future-support choice is made here.
        for (index, definition) in target.plan.definitions.iter_mut().enumerate() {
            if proof.negative[index]
                || definition.absorptions.len() != definitions[index].absorptions.len()
            {
                continue;
            }
            let (left, right) = match &mut definition.operator {
                Operator::Semijoin(join) => (&mut join.left, &mut join.right),
                Operator::NaturalJoin(join) => (&mut join.left, &mut join.right),
                _ => continue,
            };
            let reduced_right = proof.absorb(dependencies[index][1], dependencies[index][0]);
            let reduced_left = proof.absorb(dependencies[index][0], reduced_right);
            *left = definitions[reduced_left].result.clone();
            *right = definitions[reduced_right].result.clone();
        }
    }
    loop {
        reachable.fill(false);
        reachable[indices[&symbol_name(&target.plan.output.input)]] = true;
        for (index, definition) in target.plan.definitions.iter().enumerate().rev() {
            if reachable[index] {
                operands(&definition.operator, |input| {
                    reachable[indices[&symbol_name(input)]] = true;
                });
            }
        }
        let mut restored = false;
        for (index, definition) in target.plan.definitions.iter_mut().enumerate() {
            let previous = definitions[index].absorptions.len();
            let (left, right) = match &mut definition.operator {
                Operator::Semijoin(join) => (&mut join.left, &mut join.right),
                Operator::NaturalJoin(join) => (&mut join.left, &mut join.right),
                _ => continue,
            };
            for (side, operand) in [left, right].into_iter().enumerate() {
                let left_side = side == 0;
                let recorded = definition.absorptions[previous..].iter()
                    .find(|fact| fact.left_token.is_some() == left_side);
                let original = recorded.map_or(dependencies[index][side],
                    |fact| indices[&symbol_name(&fact.reduced)]);
                let replacement = indices[&symbol_name(operand)];
                let mut bypassed = original;
                while bypassed != replacement {
                    if reachable[bypassed] {
                        *operand = definitions[original].result.clone();
                        let mut fact_index = 0;
                        definition.absorptions.retain(|fact| {
                            let keep = fact_index < previous || fact.left_token.is_some() != left_side;
                            fact_index += 1;
                            keep
                        });
                        restored = true;
                        break;
                    }
                    bypassed = dependencies[bypassed][0];
                }
            }
        }
        if !restored {
            break;
        }
    }
    let mut index = 0;
    target.plan.definitions.retain(|_| {
        let keep = reachable[index];
        index += 1;
        keep
    });
    ra::check_rewrite(source, &target)?;
    Ok(target)
}

struct Lineage<'a> {
    definitions: &'a [ra::Definition],
    dependencies: &'a [Vec<usize>],
    consumers: &'a [Vec<usize>],
    returned: usize,
    headings: Vec<&'a BTreeSet<String>>,
    negative: Vec<bool>,
    opaque: Vec<bool>,
}

impl Lineage<'_> {
    fn occurrence(&self, mut input: usize) -> Option<&ra::syntax::Occurrence> {
        loop {
            match &self.definitions[input].operator {
                Operator::Rename(rename) => return Some(&rename.occurrence),
                Operator::Semijoin(_) | Operator::Select(_) | Operator::Project(_) => {
                    input = self.dependencies[input][0];
                }
                _ => return None,
            }
        }
    }

    fn restoring_join(&self, mut prefix: usize, support: usize, shared: &[&String]) -> Option<usize> {
        if shared.is_empty() || shared.iter().any(|key| !self.headings[prefix].contains(*key)) {
            return None;
        }
        loop {
            if prefix == self.returned || self.negative[prefix] || self.opaque[prefix] {
                return None;
            }
            let [next] = self.consumers[prefix][..] else {
                return None;
            };
            if self.negative[next] || self.opaque[next]
                || self.dependencies[next].first() != Some(&prefix)
                || shared.iter().any(|key| !self.headings[next].contains(*key))
            {
                return None;
            }
            match self.definitions[next].operator {
                Operator::NaturalJoin(_) => {
                    if self.dependencies[next][1] == support {
                        return Some(next);
                    }
                    if self.headings[next] != self.headings[prefix] {
                        return None;
                    }
                }
                Operator::Semijoin(_) | Operator::Select(_) => {}
                Operator::Project(_) if self.headings[prefix].is_subset(self.headings[next]) => {}
                _ => return None,
            }
            prefix = next;
        }
    }

    fn absorb(&self, mut reduced: usize, other: usize) -> usize {
        while matches!(self.definitions[reduced].operator, Operator::Semijoin(_))
            && !self.negative[reduced]
            && !self.opaque[reduced]
        {
            let [base, support] = self.dependencies[reduced][..] else {
                unreachable!("semijoin has two operands")
            };
            let shared: Vec<_> = self.headings[base]
                .intersection(self.headings[support])
                .collect();
            if !self.contained(other, support, &shared) {
                break;
            }
            reduced = base;
        }
        reduced
    }

    fn contained(&self, other: usize, support: usize, shared: &[&String]) -> bool {
        let mut pending = vec![other];
        let mut seen = BTreeSet::new();
        while let Some(index) = pending.pop() {
            if !seen.insert(index)
                || shared
                    .iter()
                    .any(|name| !self.headings[index].contains(*name))
            {
                continue;
            }
            if index == support {
                return true;
            }
            match self.definitions[index].operator {
                Operator::Semijoin(_) | Operator::Project(_) | Operator::Select(_) => {
                    pending.push(self.dependencies[index][0]);
                }
                Operator::NaturalJoin(_) => pending.extend(&self.dependencies[index]),
                _ => {}
            }
        }
        false
    }
}

// RA denotes sets. A join whose added columns have no observer can therefore
// become a semijoin: canon(project_H(L join R)) = canon(L semi R), H ⊆ cols(L).
// Do not apply this identity across Rust, negative, or block boundaries.
pub(super) fn eliminate_expansions(mut source: ra::Program) -> syn::Result<ra::Program> {
    ra::contract::check(&source)?;
    if source.plan.definitions.iter().any(|definition| {
        matches!(
            definition.operator,
            Operator::Bind(_)
                | Operator::Guard(_)
                | Operator::Anti(_)
                | Operator::Union(_)
                | Operator::Group(_)
        )
    }) {
        return Ok(source);
    }
    loop {
        let analysis = ra::contract::check(&source)?;
        let definitions = &source.plan.definitions;
        let indices: BTreeMap<_, _> = definitions
            .iter()
            .enumerate()
            .map(|(index, definition)| (symbol_name(&definition.result), index))
            .collect();
        let headings: Vec<_> = definitions
            .iter()
            .map(|definition| &analysis[&symbol_name(&definition.result)].heading)
            .collect();
        let mut demand = vec![BTreeSet::new(); definitions.len()];
        let output = indices[&symbol_name(&source.plan.output.input)];
        demand[output].extend(headings[output].iter().cloned());
        let dependencies: Vec<Vec<usize>> = definitions
            .iter()
            .map(|definition| {
                let mut inputs = Vec::new();
                operands(&definition.operator, |input| {
                    inputs.push(indices[&symbol_name(input)])
                });
                inputs
            })
            .collect();
        for (index, definition) in definitions.iter().enumerate().rev() {
            let mut needed = demand[index].clone();
            match &definition.operator {
                Operator::NaturalJoin(_) | Operator::Semijoin(_) => {
                    let left = dependencies[index][0];
                    let right = dependencies[index][1];
                    let shared: BTreeSet<_> = headings[left]
                        .intersection(headings[right])
                        .cloned()
                        .collect();
                    needed.extend(shared.iter().cloned());
                    demand[left].extend(needed.intersection(headings[left]).cloned());
                    if matches!(definition.operator, Operator::NaturalJoin(_)) {
                        demand[right].extend(needed.intersection(headings[right]).cloned());
                    } else {
                        demand[right].extend(shared);
                    }
                }
                Operator::Project(project) => {
                    demand[dependencies[index][0]]
                        .extend(project.attributes.iter().map(symbol_name));
                }
                Operator::Select(select) => {
                    for equality in &select.condition.equalities {
                        needed.extend(equality.variables().map(symbol_name));
                    }
                    for constraint in &select.condition.constraints {
                        needed.extend(constraint.variables().map(symbol_name));
                    }
                    demand[dependencies[index][0]].extend(needed);
                }
                Operator::Unit(_) | Operator::Rename(_) => {}
                _ => unreachable!("opaque and block operators excluded"),
            }
        }
        let mut replacement = None;
        for (index, definition) in definitions.iter().enumerate().rev() {
            if !matches!(
                definition.operator,
                Operator::NaturalJoin(_) | Operator::Semijoin(_)
            ) {
                continue;
            }
            let left = dependencies[index][0];
            let right = dependencies[index][1];
            let shared: Vec<_> = headings[left].intersection(headings[right]).collect();
            let mut pending = vec![left];
            let mut seen = BTreeSet::new();
            let mut implied = false;
            while let Some(current) = pending.pop() {
                if !seen.insert(current)
                    || shared
                        .iter()
                        .any(|column| !headings[current].contains(*column))
                {
                    continue;
                }
                if current == right {
                    implied = true;
                    break;
                }
                match &definitions[current].operator {
                    Operator::NaturalJoin(_) | Operator::Semijoin(_) => {
                        pending.extend(&dependencies[current]);
                    }
                    Operator::Project(_) | Operator::Select(_) => {
                        pending.push(dependencies[current][0]);
                    }
                    _ => {}
                }
            }
            if !implied {
                continue;
            }
            match &definition.operator {
                Operator::NaturalJoin(join)
                    if headings[dependencies[index][1]]
                        .difference(headings[dependencies[index][0]])
                        .all(|column| !demand[index].contains(column)) =>
                {
                    let mut target = source.clone();
                    target.plan.definitions[index].operator =
                        Operator::Semijoin(Box::new(ra::Semijoin {
                            semijoin_token: Default::default(),
                            left: join.left.clone(),
                            with_token: Default::default(),
                            right: join.right.clone(),
                        }));
                    replacement = Some(target);
                    break;
                }
                Operator::Semijoin(join) => {
                    let mut target = source.clone();
                    for definition in &mut target.plan.definitions {
                        replace_operand(
                            &mut definition.operator,
                            &definitions[index].result,
                            &join.left,
                        );
                    }
                    if target.plan.output.input == definitions[index].result {
                        target.plan.output.input = join.left.clone();
                    }
                    let mut reachable = vec![false; definitions.len()];
                    reachable[indices[&symbol_name(&target.plan.output.input)]] = true;
                    for (index, definition) in target.plan.definitions.iter().enumerate().rev() {
                        if reachable[index] {
                            operands(&definition.operator, |input| {
                                reachable[indices[&symbol_name(input)]] = true;
                            });
                        }
                    }
                    let mut index = 0;
                    target.plan.definitions.retain(|_| {
                        let keep = reachable[index];
                        index += 1;
                        keep
                    });
                    replacement = Some(target);
                    break;
                }
                _ => {}
            }
        }
        let Some(target) = replacement else {
            return Ok(source);
        };
        ra::check_rewrite(&source, &target)?;
        source = target;
    }
}

fn replace_operand(operator: &mut Operator, old: &Ident, new: &Ident) {
    let replace = |input: &mut Ident| {
        if input == old {
            *input = new.clone();
        }
    };
    match operator {
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
        Operator::Unit(_) | Operator::Rename(_) => {}
        _ => unreachable!("opaque and block operators excluded"),
    }
}

// Metadata references participate in liveness just like ordinary operands.
// In particular, a block's finite domain and opaque prefix are not disposable.
fn operands(operator: &Operator, mut visit: impl FnMut(&Ident)) {
    match operator {
        Operator::Unit(_) | Operator::Rename(_) => {}
        Operator::NaturalJoin(join) => {
            visit(&join.left);
            visit(&join.right);
        }
        Operator::Semijoin(join) => {
            visit(&join.left);
            visit(&join.right);
        }
        Operator::Anti(anti) => {
            visit(&anti.left);
            visit(&anti.right);
        }
        Operator::Project(project) => visit(&project.input),
        Operator::Group(group) => visit(&group.input),
        Operator::Select(select) => visit(&select.input),
        Operator::Bind(bind) => visit(&bind.input),
        Operator::Guard(guard) => visit(&guard.input),
        Operator::Union(union) => {
            for input in &union.operands {
                visit(input);
            }
            if let Some(origin) = &union.origin {
                if let Some(over) = &origin.over {
                    visit(&over.prefix);
                }
                if let Some(domain) = &origin.domain {
                    visit(&domain.input);
                }
            }
        }
    }
}
