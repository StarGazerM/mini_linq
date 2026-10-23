use cq_lang_sequence::{LevelNode, Machine, Program, Term};

use crate::Advice;

/// Enumerate paired partitions in definition order. Named pipe producers remain
/// named: the shared rewrite follows their references without fusing them.
/// Existing tiles are authored scheduling choices, including their item counts.
pub(crate) fn advise(
    checker: &mut cq_lang_sequence::contract::Checker,
    program: &Program,
    machine: &Machine,
    items: usize,
    segmented: bool,
) -> syn::Result<Vec<Advice>> {
    let Some(width) = width(&machine.root) else {
        return Ok(Vec::new());
    };
    let mut current = program.clone();
    let mut advice = Vec::new();
    for definition in &program.sequence.definitions {
        if !matches!(definition.term, Term::Store { .. }) {
            continue;
        }
        let Some(grade) = program.graded.as_ref().and_then(|graded| {
            graded
                .grades
                .iter()
                .find(|grade| grade.name == definition.name)
        }) else {
            continue;
        };
        if grade.level.len() > 1
            && let Term::Store { inner, .. } = &definition.term
            && let Term::Ref(producer) = inner.as_ref()
            && program.graded.as_ref().is_some_and(|graded| {
                graded
                    .grades
                    .iter()
                    .any(|grade| grade.name == *producer && grade.level.len() == 1)
            })
        {
            continue; // A host-produced upload is not a device computation.
        }
        if grade.level.len() == 1 && grade.level[0] == machine.root.name {
            let Term::Store { inner, .. } = &definition.term else {
                unreachable!()
            };
            let Term::Ref(producer) = inner.as_ref() else {
                continue;
            };
            if !program.graded.as_ref().is_some_and(|graded| {
                graded
                    .grades
                    .iter()
                    .any(|grade| grade.name == *producer && grade.level.len() > 1)
            }) {
                continue;
            }
        }
        if has_tile(&current, &definition.term) {
            continue;
        }
        if let Some(name) = unstored_input(&current, &definition.term) {
            let step = Advice::Store(name);
            current = step.apply_in(checker, &current)?;
            advice.push(step);
        }
        let step = Advice::Partition {
            name: definition.name.clone(),
            n: width,
            items: (items > 1 && expands(&current, &definition.term)).then_some(items),
        };
        let next = step.apply_in(checker, &current)?;
        if next != current {
            advice.push(step);
            current = next;
        }
    }
    if !segmented {
        return Ok(advice);
    }
    // The first partition distributes the input. Every named expansion after
    // that partition publishes offsets before another contiguous partition.
    // Physical retention boundaries already present in the input stay intact.
    let inner_width = width.checked_mul(items).ok_or_else(|| {
        syn::Error::new(
            machine.name.span(),
            "P-Decompose: partition width overflows usize",
        )
    })?;
    for definition in &program.sequence.definitions {
        if !matches!(&definition.term, Term::Flatten(inner) if matches!(inner.as_ref(), Term::Lookup { .. }))
        {
            continue;
        }
        let Some(grade) = program.graded.as_ref().and_then(|graded| {
            graded
                .grades
                .iter()
                .find(|grade| grade.name == definition.name)
        }) else {
            continue;
        };
        if grade.level.len() == 1 && grade.level[0] == machine.root.name {
            continue;
        }
        if has_tile(program, &definition.term) {
            continue;
        }
        let step = Advice::Segment {
            name: definition.name.clone(),
            n: inner_width,
            items: (items > 1).then_some(items),
        };
        current = step.apply_in(checker, &current)?;
        advice.push(step);
    }
    Ok(advice)
}

pub(crate) fn width(level: &LevelNode) -> Option<usize> {
    level
        .width
        .or_else(|| level.children.iter().find_map(width))
}

fn unstored_input(program: &Program, term: &Term) -> Option<syn::Ident> {
    match term {
        Term::Ref(name) => match producer(program, name)? {
            Term::Rows(_) => Some(name.clone()),
            Term::Store { .. } => None,
            term => unstored_input(program, term),
        },
        Term::Map(_, inner)
        | Term::Flatten(inner)
        | Term::Compact(_, inner)
        | Term::Lookup { inner, .. }
        | Term::Fold(_, inner)
        | Term::Group(_, inner)
        | Term::Canon(inner)
        | Term::Store { inner, .. } => unstored_input(program, inner),
        Term::Tile { .. } | Term::Union(_) | Term::Choose(_) | Term::Rows(_) => None,
    }
}

fn producer<'a>(program: &'a Program, name: &syn::Ident) -> Option<&'a Term> {
    program
        .sequence
        .definitions
        .iter()
        .find(|definition| definition.name == *name)
        .map(|definition| &definition.term)
}

fn has_tile(program: &Program, term: &Term) -> bool {
    match term {
        Term::Tile { .. } => true,
        Term::Ref(name) => producer(program, name)
            .is_some_and(|term| !matches!(term, Term::Store { .. }) && has_tile(program, term)),
        Term::Map(_, inner)
        | Term::Flatten(inner)
        | Term::Compact(_, inner)
        | Term::Lookup { inner, .. }
        | Term::Fold(_, inner)
        | Term::Group(_, inner)
        | Term::Canon(inner)
        | Term::Store { inner, .. } => has_tile(program, inner),
        Term::Union(arms) => arms.iter().any(|term| has_tile(program, term)),
        Term::Choose(arms) => arms.iter().any(|(_, term)| has_tile(program, term)),
        Term::Rows(_) => false,
    }
}

fn expands(program: &Program, term: &Term) -> bool {
    match term {
        Term::Flatten(inner) if matches!(inner.as_ref(), Term::Lookup { .. }) => true,
        Term::Ref(name) => producer(program, name)
            .is_some_and(|term| !matches!(term, Term::Store { .. }) && expands(program, term)),
        Term::Map(_, inner)
        | Term::Flatten(inner)
        | Term::Compact(_, inner)
        | Term::Lookup { inner, .. }
        | Term::Fold(_, inner)
        | Term::Group(_, inner)
        | Term::Canon(inner)
        | Term::Tile { inner, .. }
        | Term::Store { inner, .. } => expands(program, inner),
        Term::Union(arms) => arms.iter().any(|term| expands(program, term)),
        Term::Choose(arms) => arms.iter().any(|(_, term)| expands(program, term)),
        Term::Rows(_) => false,
    }
}
