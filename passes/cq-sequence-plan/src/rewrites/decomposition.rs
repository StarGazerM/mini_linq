use cq_lang_sequence::{Program, Term};
use proc_macro2::TokenStream;

use crate::rewrites::{Tile, checked_in, definition};

/// Partition a producer chain at its input storage boundary, stopping after
/// an order-owning merge. The paired flatten keeps its row shape unchanged.
/// An existing tile is authored advice and is not replaced.
pub fn partition(physical: &cq_lang_sequence::structures::physical::Program, structures: &cq_lang_sequence::structures::Program, source: &Program, arguments: TokenStream) -> syn::Result<Program> { partition_in(&mut Default::default(), physical, structures, source, arguments) }

pub fn partition_in(checker: &mut cq_lang_sequence::contract::Checker, physical: &cq_lang_sequence::structures::physical::Program, structures: &cq_lang_sequence::structures::Program, source: &Program, arguments: TokenStream,) -> syn::Result<Program> { let Tile { name, n, items } = syn::parse2(arguments)?;
let index = definition(source, &name)?;
let analysis = checker.check_shared(physical, structures, source)?;
positive(&name, n, items)?;
let Some((index, depth)) = boundary(
    physical,
    &analysis,
    source,
    index,
    0,
    &source.sequence.definitions[index].term,
    true,
)?
else {
    return Ok(source.clone());
};
let mut target = source.clone();
let mut term = &mut target.sequence.definitions[index].term;
for _ in 0..depth {
    term = child_mut(term).expect("the located boundary has a unary path");
}
let inner = std::mem::replace(term, Term::Union(Vec::new()));
*term = Term::Flatten(Box::new(Term::Tile {
    n,
    items,
    inner: Box::new(inner),
}));
checked_in(checker, physical, structures, source, target) }

/// Materialize and partition a named flatten level, preserving its row shape.
/// This records the offsets boundary explicitly rather than hiding it in the
/// target emitter. Existing stores and tiles are never overwritten.
pub fn segment(physical: &cq_lang_sequence::structures::physical::Program, structures: &cq_lang_sequence::structures::Program, source: &Program, arguments: TokenStream) -> syn::Result<Program> { segment_in(&mut Default::default(), physical, structures, source, arguments) }

pub fn segment_in(checker: &mut cq_lang_sequence::contract::Checker, physical: &cq_lang_sequence::structures::physical::Program, structures: &cq_lang_sequence::structures::Program, source: &Program, arguments: TokenStream,) -> syn::Result<Program> { let Tile { name, n, items } = syn::parse2(arguments)?;
let index = definition(source, &name)?;
positive(&name, n, items)?;
if !matches!(source.sequence.definitions[index].term, Term::Flatten(_)) {
    return Err(syn::Error::new_spanned(
        name,
        "segment requires a named flatten level",
    ));
}
let mut target = source.clone();
let term = &mut target.sequence.definitions[index].term;
let inner = std::mem::replace(term, Term::Union(Vec::new()));
*term = Term::Flatten(Box::new(Term::Tile {
    n,
    items,
    inner: Box::new(Term::Store {
        capacity: None,
        inner: Box::new(inner),
    }),
}));
checked_in(checker, physical, structures, source, target) }

fn positive(name: &syn::Ident, n: usize, items: Option<usize>) -> syn::Result<()> {
    if n == 0 || items == Some(0) {
        return Err(syn::Error::new_spanned(
            name,
            "BE-Partition: tile and items sizes must be positive",
        ));
    }
    Ok(())
}

fn boundary(
    physical: &cq_lang_sequence::structures::physical::Program,
    analysis: &cq_lang_sequence::Analysis,
    program: &Program,
    index: usize,
    depth: usize,
    term: &Term,
    root: bool,
) -> syn::Result<Option<(usize, usize)>> {
    match term {
        Term::Tile { .. } | Term::PartitionK { .. } => Ok(None),
        // BE-MergeInput: provider order does not authorize a partial row stream.
        // Complete the ordered expansion first, then distribute its flat output.
        Term::Flatten(inner) if ordered_expansion(physical, analysis, program, inner) => {
            Ok(Some((index, depth)))
        }
        Term::Map(_, _) | Term::Compact(_, _)
            if ordered_expansion(physical, analysis, program, term) =>
        {
            // An unflattened expansion is not a flat distributing driver.
            // Keep it complete rather than descending into an earlier join.
            Ok(None)
        }
        Term::Ref(name) => {
            let producer = definition(program, name)?;
            let producer_term = &program.sequence.definitions[producer].term;
            if matches!(producer_term, Term::Store { .. }) {
                Ok(Some((index, depth)))
            } else {
                boundary(physical, analysis, program, producer, 0, producer_term, false)
            }
        }
        Term::Rows(_) => Ok(Some((index, depth))),
        Term::Store { .. } if !root => Ok(Some((index, depth))),
        Term::Map(_, inner)
        | Term::Compact(_, inner)
        | Term::Flatten(inner)
        | Term::Lookup { inner, .. }
        | Term::Fold(_, inner)
        | Term::Group(_, inner)
        | Term::Canon(inner)
        | Term::Store { inner, .. } => boundary(physical, analysis, program, index, depth + 1, inner, false),
        Term::Choose(_) | Term::Union(_) => Ok(None),
    }
}

fn ordered_expansion(
    physical: &cq_lang_sequence::structures::physical::Program,
    analysis: &cq_lang_sequence::Analysis,
    program: &Program,
    term: &Term,
) -> bool {
    use cq_lang_sequence::structures::{physical::Operator, symbol_name};
    match term {
        Term::Lookup { .. } => lookup_result(analysis, program, term).is_some_and(|name| {
            physical.program.definitions.iter().any(|definition| {
                symbol_name(&definition.name) == name
                    && matches!(definition.operator, Operator::Merge(..))
            })
        }),
        Term::Map(name, inner) | Term::Compact(name, inner) => {
            let source = input_result(analysis, program, inner);
            let mut current = Some(name);
            while let Some(name) = current {
                if source.as_deref() == Some(symbol_name(name).as_str()) {
                    break;
                }
                let Some(definition) = physical.program.definitions.iter()
                    .find(|definition| definition.name == *name) else {
                    return false;
                };
                if matches!(definition.operator, Operator::Merge(..)) {
                    return true;
                }
                current = definition.operator.input();
            }
            false
        }
        Term::Ref(name) => definition(program, name).ok().is_some_and(|index| {
            ordered_expansion(physical, analysis, program, &program.sequence.definitions[index].term)
        }),
        _ => false,
    }
}

fn input_result(analysis: &cq_lang_sequence::Analysis, program: &Program, term: &Term) -> Option<String> {
    use cq_lang_sequence::structures::symbol_name;
    match term {
        Term::Rows(name) | Term::Map(name, _) | Term::Compact(name, _)
        | Term::Group(name, _) => Some(symbol_name(name)),
        Term::Ref(name) => analysis.results.get(&symbol_name(name)).cloned(),
        Term::Flatten(inner) | Term::Canon(inner) | Term::Store { inner, .. }
        | Term::Tile { inner, .. } | Term::PartitionK { inner, .. } => input_result(analysis, program, inner),
        Term::Lookup { .. } => lookup_result(analysis, program, term),
        _ => None,
    }
}

/// Resolve the existing constructor-preorder reader record, never reselect a
/// Physical reader from another lookup's aggregate provider capability.
fn lookup_result(
    analysis: &cq_lang_sequence::Analysis,
    program: &Program,
    target: &Term,
) -> Option<String> {
    fn ordinal(term: &Term, target: &Term, next: &mut usize) -> Option<usize> {
        if matches!(term, Term::Lookup { .. }) {
            let slot = *next;
            *next += 1;
            if std::ptr::eq(term, target) {
                return Some(slot);
            }
        }
        match term {
            Term::Map(_, inner) | Term::Compact(_, inner) | Term::Flatten(inner)
            | Term::Fold(_, inner) | Term::Group(_, inner) | Term::Canon(inner)
            | Term::Tile { inner, .. } | Term::PartitionK { inner, .. }
            | Term::Lookup { inner, .. } | Term::Store { inner, .. } => ordinal(inner, target, next),
            Term::Choose(arms) => arms.iter().find_map(|(_, arm)| ordinal(arm, target, next)),
            Term::Union(arms) => arms.iter().find_map(|arm| ordinal(arm, target, next)),
            Term::Rows(_) | Term::Ref(_) => None,
        }
    }
    let mut next = 0;
    let slot = program.sequence.definitions.iter()
        .find_map(|definition| ordinal(&definition.term, target, &mut next))?;
    match analysis.lookup_readers.get(&slot)? {
        cq_lang_sequence::structures::ReaderRef::Result(name) => Some(name.clone()),
        _ => None,
    }
}

fn child_mut(term: &mut Term) -> Option<&mut Term> {
    match term {
        Term::Map(_, inner)
        | Term::Compact(_, inner)
        | Term::Flatten(inner)
        | Term::Lookup { inner, .. }
        | Term::Fold(_, inner)
        | Term::Group(_, inner)
        | Term::Canon(inner)
        | Term::Tile { inner, .. }
        | Term::PartitionK { inner, .. }
        | Term::Store { inner, .. } => Some(inner),
        _ => None,
    }
}
