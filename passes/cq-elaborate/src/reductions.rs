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
    let mut negative = vec![false; definitions.len()];
    let mut opaque = vec![false; definitions.len()];
    for (index, definition) in definitions.iter().enumerate() {
        if let Operator::Anti(anti) = &definition.operator {
            negative[indices[&symbol_name(&anti.right)]] = true;
        }
        opaque[index] = matches!(definition.operator, Operator::Bind(_) | Operator::Guard(_))
            || dependencies[index].iter().any(|input| opaque[*input]);
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
        headings,
        negative,
        opaque,
    };
    let mut target = source.clone();
    for (index, definition) in target.plan.definitions.iter_mut().enumerate() {
        if proof.negative[index] {
            continue;
        }
        let right = match &mut definition.operator {
            Operator::Semijoin(join) => &mut join.right,
            Operator::NaturalJoin(join) => &mut join.right,
            _ => continue,
        };
        let left = dependencies[index][0];
        *right = definitions[proof.absorb(indices[&symbol_name(right)], left)]
            .result
            .clone();
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
            let right = match &mut definition.operator {
                Operator::Semijoin(join) => &mut join.right,
                Operator::NaturalJoin(join) => &mut join.right,
                _ => continue,
            };
            let original = dependencies[index][1];
            let replacement = indices[&symbol_name(right)];
            let mut bypassed = original;
            while bypassed != replacement {
                if reachable[bypassed] {
                    *right = definitions[original].result.clone();
                    restored = true;
                    break;
                }
                bypassed = dependencies[bypassed][0];
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
    headings: Vec<&'a BTreeSet<String>>,
    negative: Vec<bool>,
    opaque: Vec<bool>,
}

impl Lineage<'_> {
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
                || shared.iter().any(|name| !self.headings[index].contains(*name))
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
