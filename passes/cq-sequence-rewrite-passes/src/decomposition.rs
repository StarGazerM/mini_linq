use cq_lang_sequence::{Program, Term};
use proc_macro2::TokenStream;

use crate::{Tile, checked_in, definition};

/// Partition a producer chain at its input storage boundary. The paired
/// flatten keeps its row shape unchanged, including across named pipe terms.
/// An existing tile is authored advice and is not replaced.
pub fn partition(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    partition_in(&mut Default::default(), source, arguments)
}

pub fn partition_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    arguments: TokenStream,
) -> syn::Result<Program> {
    let Tile { name, n, items } = syn::parse2(arguments)?;
    let index = definition(source, &name)?;
    checker.check_shared(source)?;
    positive(&name, n, items)?;
    let Some((index, depth)) = boundary(
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
    checked_in(checker, source, target)
}

/// Materialize and partition a named flatten level, preserving its row shape.
/// This records the offsets boundary explicitly rather than hiding it in the
/// target emitter. Existing stores and tiles are never overwritten.
pub fn segment(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    segment_in(&mut Default::default(), source, arguments)
}

pub fn segment_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    arguments: TokenStream,
) -> syn::Result<Program> {
    let Tile { name, n, items } = syn::parse2(arguments)?;
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
    checked_in(checker, source, target)
}

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
    program: &Program,
    index: usize,
    depth: usize,
    term: &Term,
    root: bool,
) -> syn::Result<Option<(usize, usize)>> {
    match term {
        Term::Tile { .. } | Term::PartitionK { .. } => Ok(None),
        Term::Ref(name) => {
            let producer = definition(program, name)?;
            let producer_term = &program.sequence.definitions[producer].term;
            if matches!(producer_term, Term::Store { .. }) {
                Ok(Some((index, depth)))
            } else {
                boundary(program, producer, 0, producer_term, false)
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
        | Term::Store { inner, .. } => boundary(program, index, depth + 1, inner, false),
        Term::Choose(_) | Term::Union(_) => Ok(None),
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
